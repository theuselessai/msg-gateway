#![allow(dead_code)]

use cel_interpreter::objects::Map;
use cel_interpreter::Value;
use std::collections::HashMap;
use std::sync::Arc;

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

#[cfg(test)]
mod tests {
    use super::*;
    use cel_interpreter::objects::Key;
    use serde_json::json;

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
}
