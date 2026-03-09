use cel_interpreter::objects::Map;
use cel_interpreter::{Context, Program, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crate::config::{GuardrailAction, GuardrailDirection, GuardrailOnError, GuardrailRule};
use crate::message::InboundMessage;

pub fn load_rules_from_dir(dir: &Path) -> Vec<GuardrailRule> {
    if !dir.exists() {
        return vec![];
    }
    let mut entries: Vec<_> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
        Err(e) => {
            tracing::error!(error = %e, "Failed to read guardrails dir");
            return vec![];
        }
    };
    entries.sort_by_key(|e| e.file_name());
    let mut rules = vec![];
    for entry in entries {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(path = %path.display(), error = %e, "Failed to read rule file");
                continue;
            }
        };
        let rule: GuardrailRule = match serde_json::from_str(&content) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(path = %path.display(), error = %e, "Failed to parse rule file");
                continue;
            }
        };
        if !rule.enabled {
            tracing::debug!(name = %rule.name, "Skipping disabled guardrail rule");
            continue;
        }
        rules.push(rule);
    }
    rules
}

/// Convert a `serde_json::Value` into a `cel_interpreter::Value`.
///
/// Conversion rules:
/// - JSON null  → CEL Value::Null
/// - JSON bool  → CEL Value::Bool
/// - JSON string → CEL Value::String(Arc<String>)
/// - JSON integer → CEL Value::Int(i64)
/// - JSON float  → CEL Value::Float(f64)
/// - JSON array  → CEL Value::List(Arc<Vec<Value>>)
/// - JSON object → CEL Value::Map
///
/// IMPORTANT: `has()` is NOT available in cel-interpreter. Option<T> fields
/// serialized as JSON null MUST map to CEL Value::Null (not a missing key).
pub fn json_to_cel_value(value: serde_json::Value) -> Value {
    match value {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::String(s) => Value::String(Arc::new(s)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(f) = n.as_f64() {
                Value::Float(f)
            } else {
                // Fallback: treat as string
                Value::String(Arc::new(n.to_string()))
            }
        }
        serde_json::Value::Array(arr) => {
            let cel_list: Vec<Value> = arr.into_iter().map(json_to_cel_value).collect();
            Value::List(Arc::new(cel_list))
        }
        serde_json::Value::Object(obj) => {
            let mut map: HashMap<cel_interpreter::objects::Key, Value> = HashMap::new();
            for (k, v) in obj {
                map.insert(
                    cel_interpreter::objects::Key::String(Arc::new(k)),
                    json_to_cel_value(v),
                );
            }
            Value::Map(Map::from(map))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardrailVerdict {
    Allow,
    Block {
        rule_name: String,
        reject_message: String,
    },
}

pub struct CompiledRule {
    pub name: String,
    pub program: Arc<Program>,
    pub action: GuardrailAction,
    pub direction: GuardrailDirection,
    pub on_error: GuardrailOnError,
    pub reject_message: Option<String>,
}

pub struct GuardrailEngine {
    rules: Vec<CompiledRule>,
}

impl GuardrailEngine {
    pub fn from_rules(rules: Vec<GuardrailRule>) -> Self {
        let mut compiled = Vec::new();
        for rule in rules {
            match Program::compile(&rule.expression) {
                Ok(program) => {
                    compiled.push(CompiledRule {
                        name: rule.name,
                        program: Arc::new(program),
                        action: rule.action,
                        direction: rule.direction,
                        on_error: rule.on_error,
                        reject_message: rule.reject_message,
                    });
                }
                Err(e) => {
                    tracing::error!(
                        rule_name = %rule.name,
                        expression = %rule.expression,
                        error = %e,
                        "Failed to compile guardrail CEL expression, skipping rule"
                    );
                }
            }
        }
        GuardrailEngine { rules: compiled }
    }

    pub fn evaluate_inbound(&self, message: &InboundMessage) -> GuardrailVerdict {
        let mut json_val = match serde_json::to_value(message) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(error = %e, "Failed to serialize InboundMessage for guardrail evaluation");
                return GuardrailVerdict::Allow;
            }
        };
        // Ensure fields omitted by skip_serializing_if are present as zero-values in CEL.
        // `attachments` is omitted when empty (Vec::is_empty), causing "No such key" errors.
        if let Some(obj) = json_val.as_object_mut() {
            obj.entry("attachments")
                .or_insert(serde_json::Value::Array(vec![]));
        }
        let cel_val = json_to_cel_value(json_val);

        let mut ctx = Context::default();
        ctx.add_variable_from_value("message", cel_val);

        for rule in &self.rules {
            if rule.direction == GuardrailDirection::Outbound {
                continue;
            }

            match rule.program.execute(&ctx) {
                Ok(Value::Bool(true)) => match rule.action {
                    GuardrailAction::Block => {
                        let reject_msg = rule
                            .reject_message
                            .clone()
                            .unwrap_or_else(|| rule.name.clone());
                        return GuardrailVerdict::Block {
                            rule_name: rule.name.clone(),
                            reject_message: reject_msg,
                        };
                    }
                    GuardrailAction::Log => {
                        tracing::warn!(
                            rule_name = %rule.name,
                            "Guardrail rule matched (log only)"
                        );
                    }
                },
                Ok(_) => {}
                Err(e) => {
                    tracing::error!(
                        rule_name = %rule.name,
                        error = %e,
                        "Guardrail rule evaluation error"
                    );
                    match rule.on_error {
                        GuardrailOnError::Block => {
                            let reject_msg = rule
                                .reject_message
                                .clone()
                                .unwrap_or_else(|| rule.name.clone());
                            return GuardrailVerdict::Block {
                                rule_name: rule.name.clone(),
                                reject_message: reject_msg,
                            };
                        }
                        GuardrailOnError::Allow => {}
                    }
                }
            }
        }

        GuardrailVerdict::Allow
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cel_interpreter::objects::Key;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    fn write_rule(dir: &TempDir, filename: &str, content: &str) {
        fs::write(dir.path().join(filename), content).unwrap();
    }

    fn minimal_rule_json(name: &str) -> String {
        format!(
            r#"{{"name":"{name}","expression":"true","enabled":true}}"#,
            name = name
        )
    }

    fn disabled_rule_json(name: &str) -> String {
        format!(
            r#"{{"name":"{name}","expression":"true","enabled":false}}"#,
            name = name
        )
    }

    #[test]
    fn test_load_rules_three_valid_files_in_filename_order() {
        let dir = TempDir::new().unwrap();
        write_rule(&dir, "03_c.json", &minimal_rule_json("rule_c"));
        write_rule(&dir, "01_a.json", &minimal_rule_json("rule_a"));
        write_rule(&dir, "02_b.json", &minimal_rule_json("rule_b"));

        let rules = load_rules_from_dir(dir.path());

        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].name, "rule_a");
        assert_eq!(rules[1].name, "rule_b");
        assert_eq!(rules[2].name, "rule_c");
    }

    #[test]
    fn test_load_rules_sorted_lexicographically() {
        let dir = TempDir::new().unwrap();
        write_rule(&dir, "02_b.json", &minimal_rule_json("rule_b"));
        write_rule(&dir, "01_a.json", &minimal_rule_json("rule_a"));

        let rules = load_rules_from_dir(dir.path());

        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].name, "rule_a");
        assert_eq!(rules[1].name, "rule_b");
    }

    #[test]
    fn test_load_rules_skips_malformed_json() {
        let dir = TempDir::new().unwrap();
        write_rule(&dir, "01_valid.json", &minimal_rule_json("valid_rule"));
        write_rule(&dir, "02_bad.json", "this is not json {{{");

        let rules = load_rules_from_dir(dir.path());

        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "valid_rule");
    }

    #[test]
    fn test_load_rules_skips_disabled_rules() {
        let dir = TempDir::new().unwrap();
        write_rule(&dir, "01_enabled.json", &minimal_rule_json("enabled_rule"));
        write_rule(
            &dir,
            "02_disabled.json",
            &disabled_rule_json("disabled_rule"),
        );

        let rules = load_rules_from_dir(dir.path());

        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "enabled_rule");
    }

    #[test]
    fn test_load_rules_nonexistent_dir_returns_empty() {
        let rules = load_rules_from_dir(std::path::Path::new(
            "/nonexistent/path/that/does/not/exist",
        ));
        assert!(rules.is_empty());
    }

    #[test]
    fn test_load_rules_empty_dir_returns_empty() {
        let dir = TempDir::new().unwrap();
        let rules = load_rules_from_dir(dir.path());
        assert!(rules.is_empty());
    }

    #[test]
    fn test_load_rules_ignores_non_json_files() {
        let dir = TempDir::new().unwrap();
        write_rule(&dir, "rule.txt", &minimal_rule_json("txt_rule"));
        write_rule(
            &dir,
            "rule.disabled",
            &minimal_rule_json("disabled_ext_rule"),
        );
        write_rule(&dir, "rule.json", &minimal_rule_json("json_rule"));

        let rules = load_rules_from_dir(dir.path());

        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "json_rule");
    }

    fn get_map_value(cel_val: &Value, key: &str) -> Option<Value> {
        if let Value::Map(m) = cel_val {
            m.map.get(&Key::String(Arc::new(key.to_string()))).cloned()
        } else {
            None
        }
    }

    #[test]
    fn test_null() {
        let result = json_to_cel_value(serde_json::Value::Null);
        assert!(matches!(result, Value::Null));
    }

    #[test]
    fn test_bool_true() {
        let result = json_to_cel_value(json!(true));
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_bool_false() {
        let result = json_to_cel_value(json!(false));
        assert!(matches!(result, Value::Bool(false)));
    }

    #[test]
    fn test_string() {
        let result = json_to_cel_value(json!("hello"));
        assert!(matches!(result, Value::String(s) if s.as_ref() == "hello"));
    }

    #[test]
    fn test_empty_string() {
        let result = json_to_cel_value(json!(""));
        assert!(matches!(result, Value::String(s) if s.as_ref() == ""));
    }

    #[test]
    fn test_positive_integer() {
        let result = json_to_cel_value(json!(42));
        assert!(matches!(result, Value::Int(42)));
    }

    #[test]
    fn test_negative_integer() {
        let result = json_to_cel_value(json!(-7));
        assert!(matches!(result, Value::Int(-7)));
    }

    #[test]
    fn test_zero() {
        let result = json_to_cel_value(json!(0));
        assert!(matches!(result, Value::Int(0)));
    }

    #[test]
    fn test_float() {
        let result = json_to_cel_value(json!(1.5));
        if let Value::Float(f) = result {
            assert!((f - 1.5_f64).abs() < 1e-10);
        } else {
            panic!("Expected Float, got {:?}", result);
        }
    }

    #[test]
    fn test_empty_array() {
        let result = json_to_cel_value(json!([]));
        if let Value::List(list) = result {
            assert!(list.is_empty());
        } else {
            panic!("Expected List");
        }
    }

    #[test]
    fn test_array_of_strings() {
        let result = json_to_cel_value(json!(["a", "b", "c"]));
        if let Value::List(list) = result {
            assert_eq!(list.len(), 3);
            assert!(matches!(&list[0], Value::String(s) if s.as_ref() == "a"));
            assert!(matches!(&list[1], Value::String(s) if s.as_ref() == "b"));
            assert!(matches!(&list[2], Value::String(s) if s.as_ref() == "c"));
        } else {
            panic!("Expected List");
        }
    }

    #[test]
    fn test_mixed_array() {
        let result = json_to_cel_value(json!([1, "two", null, true]));
        if let Value::List(list) = result {
            assert_eq!(list.len(), 4);
            assert!(matches!(&list[0], Value::Int(1)));
            assert!(matches!(&list[1], Value::String(s) if s.as_ref() == "two"));
            assert!(matches!(&list[2], Value::Null));
            assert!(matches!(&list[3], Value::Bool(true)));
        } else {
            panic!("Expected List");
        }
    }

    #[test]
    fn test_empty_object() {
        let result = json_to_cel_value(json!({}));
        assert!(matches!(result, Value::Map(_)));
        if let Value::Map(m) = result {
            assert!(m.map.is_empty());
        }
    }

    #[test]
    fn test_simple_object() {
        let result = json_to_cel_value(json!({"name": "alice", "age": 30}));
        let name = get_map_value(&result, "name");
        let age = get_map_value(&result, "age");
        assert!(matches!(name, Some(Value::String(s)) if s.as_ref() == "alice"));
        assert!(matches!(age, Some(Value::Int(30))));
    }

    #[test]
    fn test_nested_object() {
        let result = json_to_cel_value(json!({"a": {"b": null}}));
        let a = get_map_value(&result, "a").expect("key 'a' missing");
        let b = get_map_value(&a, "b").expect("key 'b' missing");
        assert!(matches!(b, Value::Null));
    }

    #[test]
    fn test_option_field_as_null() {
        // Option<T> fields serialize to JSON null — must map to CEL Null (not missing key)
        let result = json_to_cel_value(json!({"username": null}));
        let username = get_map_value(&result, "username");
        assert!(
            matches!(username, Some(Value::Null)),
            "Option<T> None must map to CEL Null, got {:?}",
            username
        );
    }

    #[test]
    fn test_complex_nested() {
        let result = json_to_cel_value(json!({
            "source": {
                "protocol": "telegram",
                "from": {
                    "id": "123",
                    "username": null
                }
            },
            "text": "hello",
            "attachments": []
        }));
        let source = get_map_value(&result, "source").expect("source missing");
        let protocol = get_map_value(&source, "protocol").expect("protocol missing");
        assert!(matches!(protocol, Value::String(s) if s.as_ref() == "telegram"));

        let from = get_map_value(&source, "from").expect("from missing");
        let username = get_map_value(&from, "username").expect("username key missing");
        assert!(matches!(username, Value::Null));

        let attachments = get_map_value(&result, "attachments").expect("attachments missing");
        assert!(matches!(attachments, Value::List(l) if l.is_empty()));
    }

    use crate::config::{GuardrailAction, GuardrailDirection, GuardrailOnError, GuardrailType};
    use crate::message::{InboundMessage, MessageSource, UserInfo};
    use chrono::Utc;

    fn make_rule(
        name: &str,
        expression: &str,
        action: GuardrailAction,
        direction: GuardrailDirection,
        on_error: GuardrailOnError,
        reject_message: Option<&str>,
    ) -> GuardrailRule {
        GuardrailRule {
            name: name.to_string(),
            r#type: GuardrailType::Cel,
            expression: expression.to_string(),
            action,
            direction,
            on_error,
            reject_message: reject_message.map(|s| s.to_string()),
            enabled: true,
        }
    }

    fn test_message(text: &str) -> InboundMessage {
        InboundMessage {
            route: json!({"channel": "test"}),
            credential_id: "test_cred".to_string(),
            source: MessageSource {
                protocol: "test".to_string(),
                chat_id: "chat_1".to_string(),
                message_id: "msg_1".to_string(),
                reply_to_message_id: None,
                from: UserInfo {
                    id: "user_1".to_string(),
                    username: Some("testuser".to_string()),
                    display_name: None,
                },
            },
            text: text.to_string(),
            attachments: vec![],
            timestamp: Utc::now(),
            extra_data: None,
        }
    }

    #[test]
    fn test_engine_true_block_returns_block() {
        let rules = vec![make_rule(
            "block_all",
            "true",
            GuardrailAction::Block,
            GuardrailDirection::Inbound,
            GuardrailOnError::Allow,
            None,
        )];
        let engine = GuardrailEngine::from_rules(rules);
        let msg = test_message("hello");
        let verdict = engine.evaluate_inbound(&msg);
        assert_eq!(
            verdict,
            GuardrailVerdict::Block {
                rule_name: "block_all".to_string(),
                reject_message: "block_all".to_string(),
            }
        );
    }

    #[test]
    fn test_engine_false_block_returns_allow() {
        let rules = vec![make_rule(
            "never_fire",
            "false",
            GuardrailAction::Block,
            GuardrailDirection::Inbound,
            GuardrailOnError::Allow,
            None,
        )];
        let engine = GuardrailEngine::from_rules(rules);
        let msg = test_message("hello");
        assert_eq!(engine.evaluate_inbound(&msg), GuardrailVerdict::Allow);
    }

    #[test]
    fn test_engine_true_log_returns_allow() {
        let rules = vec![make_rule(
            "log_all",
            "true",
            GuardrailAction::Log,
            GuardrailDirection::Inbound,
            GuardrailOnError::Allow,
            None,
        )];
        let engine = GuardrailEngine::from_rules(rules);
        let msg = test_message("hello");
        assert_eq!(engine.evaluate_inbound(&msg), GuardrailVerdict::Allow);
    }

    #[test]
    fn test_engine_short_circuits_on_first_block() {
        let rules = vec![
            make_rule(
                "first",
                "true",
                GuardrailAction::Block,
                GuardrailDirection::Inbound,
                GuardrailOnError::Allow,
                None,
            ),
            make_rule(
                "second",
                "true",
                GuardrailAction::Block,
                GuardrailDirection::Inbound,
                GuardrailOnError::Allow,
                None,
            ),
        ];
        let engine = GuardrailEngine::from_rules(rules);
        let msg = test_message("hello");
        let verdict = engine.evaluate_inbound(&msg);
        assert_eq!(
            verdict,
            GuardrailVerdict::Block {
                rule_name: "first".to_string(),
                reject_message: "first".to_string(),
            }
        );
    }

    #[test]
    fn test_engine_invalid_expression_skipped() {
        let rules = vec![
            make_rule(
                "bad_rule",
                "this is not valid CEL !!!",
                GuardrailAction::Block,
                GuardrailDirection::Inbound,
                GuardrailOnError::Allow,
                None,
            ),
            make_rule(
                "good_rule",
                "true",
                GuardrailAction::Block,
                GuardrailDirection::Inbound,
                GuardrailOnError::Allow,
                None,
            ),
        ];
        let engine = GuardrailEngine::from_rules(rules);
        assert_eq!(engine.rules.len(), 1);
        let msg = test_message("hello");
        let verdict = engine.evaluate_inbound(&msg);
        assert_eq!(
            verdict,
            GuardrailVerdict::Block {
                rule_name: "good_rule".to_string(),
                reject_message: "good_rule".to_string(),
            }
        );
    }

    #[test]
    fn test_engine_on_error_allow() {
        let rules = vec![make_rule(
            "error_rule",
            "message.nonexistent_field == true",
            GuardrailAction::Block,
            GuardrailDirection::Inbound,
            GuardrailOnError::Allow,
            None,
        )];
        let engine = GuardrailEngine::from_rules(rules);
        let msg = test_message("hello");
        assert_eq!(engine.evaluate_inbound(&msg), GuardrailVerdict::Allow);
    }

    #[test]
    fn test_engine_on_error_block() {
        let rules = vec![make_rule(
            "error_rule",
            "message.nonexistent_field == true",
            GuardrailAction::Block,
            GuardrailDirection::Inbound,
            GuardrailOnError::Block,
            None,
        )];
        let engine = GuardrailEngine::from_rules(rules);
        let msg = test_message("hello");
        assert_eq!(
            engine.evaluate_inbound(&msg),
            GuardrailVerdict::Block {
                rule_name: "error_rule".to_string(),
                reject_message: "error_rule".to_string(),
            }
        );
    }

    #[test]
    fn test_engine_message_text_matches_password_block() {
        let rules = vec![make_rule(
            "no_passwords",
            r#"message.text.matches("password")"#,
            GuardrailAction::Block,
            GuardrailDirection::Inbound,
            GuardrailOnError::Allow,
            Some("Message contains sensitive content"),
        )];
        let engine = GuardrailEngine::from_rules(rules);
        let msg = test_message("my password is secret");
        let verdict = engine.evaluate_inbound(&msg);
        assert_eq!(
            verdict,
            GuardrailVerdict::Block {
                rule_name: "no_passwords".to_string(),
                reject_message: "Message contains sensitive content".to_string(),
            }
        );
    }

    #[test]
    fn test_engine_message_text_matches_password_allow() {
        let rules = vec![make_rule(
            "no_passwords",
            r#"message.text.matches("password")"#,
            GuardrailAction::Block,
            GuardrailDirection::Inbound,
            GuardrailOnError::Allow,
            None,
        )];
        let engine = GuardrailEngine::from_rules(rules);
        let msg = test_message("hello world");
        assert_eq!(engine.evaluate_inbound(&msg), GuardrailVerdict::Allow);
    }

    #[test]
    fn test_engine_is_empty() {
        let empty = GuardrailEngine::from_rules(vec![]);
        assert!(empty.is_empty());

        let non_empty = GuardrailEngine::from_rules(vec![make_rule(
            "rule",
            "true",
            GuardrailAction::Block,
            GuardrailDirection::Inbound,
            GuardrailOnError::Allow,
            None,
        )]);
        assert!(!non_empty.is_empty());
    }

    #[test]
    fn test_engine_outbound_rule_skipped_for_inbound() {
        let rules = vec![make_rule(
            "outbound_only",
            "true",
            GuardrailAction::Block,
            GuardrailDirection::Outbound,
            GuardrailOnError::Allow,
            None,
        )];
        let engine = GuardrailEngine::from_rules(rules);
        let msg = test_message("hello");
        assert_eq!(engine.evaluate_inbound(&msg), GuardrailVerdict::Allow);
    }

    #[test]
    fn test_engine_both_direction_applies_to_inbound() {
        let rules = vec![make_rule(
            "both_dir",
            "true",
            GuardrailAction::Block,
            GuardrailDirection::Both,
            GuardrailOnError::Allow,
            None,
        )];
        let engine = GuardrailEngine::from_rules(rules);
        let msg = test_message("hello");
        assert_eq!(
            engine.evaluate_inbound(&msg),
            GuardrailVerdict::Block {
                rule_name: "both_dir".to_string(),
                reject_message: "both_dir".to_string(),
            }
        );
    }
}
