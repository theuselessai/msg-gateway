use clap::{Parser, Subcommand};

mod client;
mod commands;
mod output;

/// plit — Pipelit ecosystem CLI for msg-gateway
///
/// Send and receive messages, manage credentials, and check health.
/// Talks to the gateway's generic adapter interface — works with any backend.
#[derive(Parser)]
#[command(name = "plit", version, about)]
struct Cli {
    /// Gateway URL
    #[arg(
        long,
        env = "GATEWAY_URL",
        default_value = "http://localhost:8080",
        global = true
    )]
    gateway_url: String,

    /// Credential token (for chat/send/listen commands)
    #[arg(long, env = "GATEWAY_TOKEN", global = true)]
    token: Option<String>,

    /// Admin token (for credentials/health commands)
    #[arg(long, env = "GATEWAY_ADMIN_TOKEN", global = true)]
    admin_token: Option<String>,

    /// Force JSON output (auto-enabled when stdout is not a TTY)
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Interactive chat REPL — send messages and see responses in real time
    Chat {
        /// Credential ID to chat through
        credential_id: String,

        /// Chat/conversation ID
        #[arg(long)]
        chat_id: String,

        /// Your user ID (sent as from.id)
        #[arg(long, default_value = "cli-user")]
        user_id: String,
    },

    /// Send a single message (fire-and-forget)
    Send {
        /// Credential ID to send through
        credential_id: String,

        /// Chat/conversation ID
        #[arg(long)]
        chat_id: String,

        /// Message text (reads from stdin if omitted)
        #[arg(long)]
        text: Option<String>,

        /// Your user ID (sent as from.id)
        #[arg(long, default_value = "cli-user")]
        user_id: String,
    },

    /// Listen for outbound messages on a WebSocket (streams JSONL to stdout)
    Listen {
        /// Credential ID to listen on
        credential_id: String,

        /// Chat/conversation ID
        #[arg(long)]
        chat_id: String,
    },

    /// Credential management
    #[command(subcommand)]
    Credentials(CredentialCommands),

    /// Check gateway health
    Health,

    /// Interactive setup wizard — bootstrap Pipelit + Gateway from scratch
    Init,

    /// Start the plit stack (gateway + Pipelit + workers)
    Start {
        /// Include frontend dev server with hot reload
        #[arg(long)]
        dev: bool,
    },

    /// Stop the running plit stack
    Stop,

    /// Remove all plit data, config, and Pipelit clone
    Uninstall,
}

#[derive(Subcommand)]
enum CredentialCommands {
    /// List all credentials
    List,

    /// Create a new credential
    Create {
        /// Credential ID
        id: String,

        /// Adapter type (e.g. "generic", "telegram")
        #[arg(long)]
        adapter: String,

        /// Credential token
        #[arg(long)]
        token: String,

        /// Backend to route to (e.g. "pipelit", "opencode")
        #[arg(long)]
        backend: Option<String>,

        /// Route config as JSON string
        #[arg(long)]
        route: Option<String>,

        /// Adapter-specific config as JSON string
        #[arg(long)]
        config: Option<String>,

        /// Activate immediately
        #[arg(long)]
        active: bool,
    },

    /// Activate a credential
    Activate {
        /// Credential ID
        id: String,
    },

    /// Deactivate a credential
    Deactivate {
        /// Credential ID
        id: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let is_tty = std::io::IsTerminal::is_terminal(&std::io::stdout());
    let json_output = cli.json || !is_tty;

    let (cfg_token, cfg_admin_token, cfg_gateway_url) = load_config_defaults();

    let ctx = commands::Context {
        gateway_url: cfg_gateway_url.unwrap_or(cli.gateway_url),
        token: cli.token.or(cfg_token),
        admin_token: cli.admin_token.or(cfg_admin_token),
        json_output,
    };

    match cli.command {
        Commands::Chat {
            credential_id,
            chat_id,
            user_id,
        } => commands::chat::run(&ctx, &credential_id, &chat_id, &user_id).await,

        Commands::Send {
            credential_id,
            chat_id,
            text,
            user_id,
        } => commands::send::run(&ctx, &credential_id, &chat_id, text.as_deref(), &user_id).await,

        Commands::Listen {
            credential_id,
            chat_id,
        } => commands::listen::run(&ctx, &credential_id, &chat_id).await,

        Commands::Credentials(cmd) => commands::credentials::run(&ctx, cmd).await,

        Commands::Health => {
            let healthy = commands::health::run(&ctx).await?;
            if !healthy {
                std::process::exit(1);
            }
            Ok(())
        }

        Commands::Init => commands::init::run().await,

        Commands::Start { dev } => commands::start::run(dev).await,

        Commands::Stop => commands::stop::run(),

        Commands::Uninstall => commands::uninstall::run(),
    }
}

fn load_config_defaults() -> (Option<String>, Option<String>, Option<String>) {
    let path = match commands::init::config::config_json_path() {
        Ok(p) if p.exists() => p,
        _ => return (None, None, None),
    };
    let raw = match std::fs::read_to_string(&path) {
        Ok(r) => r,
        _ => return (None, None, None),
    };
    let cfg: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(c) => c,
        _ => return (None, None, None),
    };

    let token = cfg["credentials"]
        .as_object()
        .and_then(|creds| creds.values().next())
        .and_then(|c| c["token"].as_str())
        .map(String::from);

    let admin_token = cfg["gateway"]["admin_token"].as_str().map(String::from);

    let listen = cfg["gateway"]["listen"].as_str().unwrap_or("");
    let gateway_url = if !listen.is_empty() {
        Some(format!("http://{listen}"))
    } else {
        None
    };

    (token, admin_token, gateway_url)
}
