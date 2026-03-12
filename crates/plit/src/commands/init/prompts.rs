use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use dialoguer::{Confirm, Input, Password, Select, theme::ColorfulTheme};
use genai::adapter::AdapterKind;

use crate::output;

pub struct UserInputs {
    pub gateway_port: u16,
    pub pipelit_port: u16,
    pub admin_username: String,
    pub admin_password: String,
    pub redis_url: String,
    pub platform_base_url: String,
    pub llm_provider: String,
    pub llm_api_key: String,
    pub llm_base_url: String,
    pub llm_model: String,
}

pub async fn collect() -> Result<UserInputs> {
    let theme = ColorfulTheme::default();

    let gateway_port = prompt_port(&theme, "Gateway port", 8080)?;

    let pipelit_port = loop {
        let port = prompt_port(&theme, "Pipelit port", 8000)?;
        if port == gateway_port {
            output::error(&format!("Port {} is already used for the gateway", port));
            continue;
        }
        break port;
    };

    let admin_username: String = Input::with_theme(&theme)
        .with_prompt("Admin username")
        .default("admin".to_string())
        .interact_text()?;

    if admin_username.trim().is_empty() {
        bail!("Admin username cannot be empty");
    }

    let admin_password = Password::with_theme(&theme)
        .with_prompt("Admin password")
        .with_confirmation("Confirm password", "Passwords don't match")
        .interact()?;

    if admin_password.is_empty() {
        bail!("Admin password cannot be empty");
    }

    let redis_url = prompt_redis_url(&theme)?;

    let platform_base_url: String = Input::with_theme(&theme)
        .with_prompt("Platform base URL")
        .default(format!("http://localhost:{}", pipelit_port))
        .interact_text()?;

    let (llm_provider, llm_api_key, llm_base_url, llm_model) = prompt_llm_config(&theme).await?;

    Ok(UserInputs {
        gateway_port,
        pipelit_port,
        admin_username,
        admin_password,
        redis_url,
        platform_base_url,
        llm_provider,
        llm_api_key,
        llm_base_url,
        llm_model,
    })
}

fn prompt_port(theme: &ColorfulTheme, label: &str, default: u16) -> Result<u16> {
    loop {
        let port: u16 = Input::with_theme(theme)
            .with_prompt(label)
            .default(default)
            .interact_text()?;

        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => {
                drop(listener);
                return Ok(port);
            }
            Err(_) => {
                output::error(&format!("Port {} is already in use", port));
            }
        }
    }
}

fn prompt_redis_url(theme: &ColorfulTheme) -> Result<String> {
    loop {
        let url: String = Input::with_theme(theme)
            .with_prompt("Redis URL")
            .default("redis://localhost:6379/0".to_string())
            .interact_text()?;

        match validate_redis(&url) {
            Ok(()) => return Ok(url),
            Err(e) => {
                output::error(&format!("Redis connection failed: {}", e));
                let retry = Confirm::with_theme(theme)
                    .with_prompt("Re-enter Redis URL?")
                    .default(true)
                    .interact()?;
                if !retry {
                    bail!("Redis is required for Pipelit");
                }
            }
        }
    }
}

fn validate_redis(url: &str) -> Result<()> {
    let is_tls = url.starts_with("rediss://");
    let stripped = url
        .strip_prefix("redis://")
        .or_else(|| url.strip_prefix("rediss://"))
        .unwrap_or(url);

    // Strip user:pass@ if present (redis://user:pass@host:port/db)
    let after_auth = stripped.split('@').next_back().unwrap_or(stripped);

    // Strip /db suffix
    let host_port = after_auth.split('/').next().unwrap_or(after_auth);

    // Strip ?query parameters
    let host_port = host_port.split('?').next().unwrap_or(host_port);

    let (host, port) = if host_port.starts_with('[') {
        // IPv6: [::1]:6379 or [::1]
        if let Some((bracket_host, rest)) = host_port.split_once(']') {
            let h = &bracket_host[1..]; // strip leading '['
            let port = if let Some(port_str) = rest.strip_prefix(':') {
                port_str
                    .parse::<u16>()
                    .with_context(|| format!("Invalid port number: {}", port_str))?
            } else {
                6379
            };
            (h, port)
        } else {
            bail!("Malformed IPv6 address in Redis URL: {}", host_port);
        }
    } else if let Some((h, p)) = host_port.rsplit_once(':') {
        let port = p
            .parse::<u16>()
            .with_context(|| format!("Invalid port number: {}", p))?;
        (h, port)
    } else {
        (host_port, 6379)
    };

    let host = if host.is_empty() { "localhost" } else { host };

    let addr = format!("{}:{}", host, port);

    let socket_addr = addr
        .to_socket_addrs()
        .context("Could not resolve Redis address")?
        .next()
        .context("No addresses found for Redis host")?;

    let mut stream = TcpStream::connect_timeout(&socket_addr, Duration::from_secs(3))
        .context("Could not connect to Redis")?;

    if is_tls {
        // TLS — can't do PING/PONG without a TLS handshake, but TCP connect
        // succeeded so the server is listening on this port.
        drop(stream);
        return Ok(());
    }

    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;

    stream.write_all(b"PING\r\n")?;

    let mut buf = [0u8; 64];
    let n = stream.read(&mut buf)?;
    let response = String::from_utf8_lossy(&buf[..n]);

    if response.starts_with("+PONG") {
        Ok(())
    } else {
        bail!("Unexpected response: {}", response.trim())
    }
}

async fn prompt_llm_config(theme: &ColorfulTheme) -> Result<(String, String, String, String)> {
    let providers = &[
        "OpenAI",
        "Anthropic",
        "Gemini",
        "Ollama",
        "OpenAI-compatible",
    ];
    let selection = Select::with_theme(theme)
        .with_prompt("LLM provider")
        .items(providers)
        .default(0)
        .interact()?;

    let provider = match selection {
        0 => "openai",
        1 => "anthropic",
        2 => "gemini",
        3 => "ollama",
        4 => "openai-compatible",
        _ => unreachable!(),
    };

    let is_ollama = provider == "ollama";
    let adapter_kind = provider_to_adapter_kind(provider);

    let base_url = match provider {
        "ollama" => Input::with_theme(theme)
            .with_prompt("Ollama base URL")
            .default("http://localhost:11434".to_string())
            .interact_text()?,
        "openai-compatible" => loop {
            let url: String = Input::with_theme(theme)
                .with_prompt("API base URL")
                .interact_text()?;
            if !url.trim().is_empty() {
                break url;
            }
            output::error("Base URL is required for OpenAI-compatible providers");
        },
        _ => String::new(),
    };

    loop {
        let api_key = if is_ollama {
            String::new()
        } else {
            Password::with_theme(theme)
                .with_prompt("API key")
                .interact()?
        };

        match fetch_models(adapter_kind, &api_key).await {
            Ok(models) if models.is_empty() => {
                let model: String = Input::with_theme(theme)
                    .with_prompt("Model name (no models found, enter manually)")
                    .interact_text()?;
                return Ok((provider.to_string(), api_key, base_url, model));
            }
            Ok(models) => {
                let idx = Select::with_theme(theme)
                    .with_prompt("Model")
                    .items(&models)
                    .default(0)
                    .interact()?;
                return Ok((provider.to_string(), api_key, base_url, models[idx].clone()));
            }
            Err(e) => {
                output::error(&format!("Failed to fetch models: {}", e));
                let retry = Confirm::with_theme(theme)
                    .with_prompt(if is_ollama {
                        "Retry?"
                    } else {
                        "Re-enter API key?"
                    })
                    .default(true)
                    .interact()?;
                if !retry {
                    bail!("LLM configuration is required");
                }
            }
        }
    }
}

fn provider_to_adapter_kind(provider: &str) -> AdapterKind {
    match provider {
        "openai" | "openai-compatible" => AdapterKind::OpenAI,
        "anthropic" => AdapterKind::Anthropic,
        "gemini" => AdapterKind::Gemini,
        "ollama" => AdapterKind::Ollama,
        _ => AdapterKind::OpenAI,
    }
}

async fn fetch_models(adapter_kind: AdapterKind, api_key: &str) -> Result<Vec<String>> {
    let client = if api_key.is_empty() {
        genai::Client::default()
    } else {
        let key = api_key.to_string();
        genai::Client::builder()
            .with_auth_resolver_fn(move |_| {
                Ok(Some(genai::resolver::AuthData::from_single(key.clone())))
            })
            .build()
    };
    client
        .all_model_names(adapter_kind)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))
}
