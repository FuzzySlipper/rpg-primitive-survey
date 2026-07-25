use std::fmt::Write;

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

pub fn structural_signature(value: &Value, normalize_identifier_maps: bool) -> String {
    let shape = shape_of(value, normalize_identifier_maps);
    let encoded = serde_json::to_vec(&shape).expect("shape serialization cannot fail");
    let digest = Sha256::digest(encoded);
    digest.iter().fold(String::new(), |mut output, byte| {
        write!(output, "{byte:02x}").expect("writing to a String cannot fail");
        output
    })
}

fn shape_of(value: &Value, normalize_identifier_maps: bool) -> Value {
    match value {
        Value::Null => Value::String("null".to_owned()),
        Value::Bool(_) => Value::String("boolean".to_owned()),
        Value::Number(number) if number.is_i64() || number.is_u64() => {
            Value::String("integer".to_owned())
        }
        Value::Number(_) => Value::String("number".to_owned()),
        Value::String(_) => Value::String("string".to_owned()),
        Value::Array(values) => {
            let mut shapes = values
                .iter()
                .map(|value| shape_of(value, normalize_identifier_maps))
                .map(|shape| {
                    serde_json::to_string(&shape).expect("shape serialization cannot fail")
                })
                .collect::<Vec<_>>();
            shapes.sort();
            shapes.dedup();
            Value::Array(
                shapes
                    .into_iter()
                    .map(|encoded| {
                        serde_json::from_str(&encoded).expect("serialized shape must decode")
                    })
                    .collect(),
            )
        }
        Value::Object(values) => {
            if normalize_identifier_maps && looks_like_identifier_map(values) {
                let mut member_shapes = values
                    .values()
                    .map(|value| shape_of(value, normalize_identifier_maps))
                    .map(|shape| {
                        serde_json::to_string(&shape).expect("shape serialization cannot fail")
                    })
                    .collect::<Vec<_>>();
                member_shapes.sort();
                member_shapes.dedup();
                return serde_json::json!({
                    "$identifierMap": member_shapes
                        .into_iter()
                        .map(|encoded| serde_json::from_str::<Value>(&encoded)
                            .expect("serialized shape must decode"))
                        .collect::<Vec<_>>()
                });
            }
            let shape = values
                .iter()
                .map(|(key, value)| (key.clone(), shape_of(value, normalize_identifier_maps)))
                .collect::<Map<_, _>>();
            Value::Object(shape)
        }
    }
}

fn looks_like_identifier_map(values: &Map<String, Value>) -> bool {
    !values.is_empty()
        && values.iter().all(|(key, value)| {
            key.len() == 16
                && key
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric())
                && value.is_object()
        })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::structural_signature;

    #[test]
    fn signatures_ignore_values_but_retain_structure() {
        let left = json!({"name": "Synthetic Blade", "system": {"damage": 4}});
        let same_shape = json!({"name": "Synthetic Wand", "system": {"damage": 99}});
        let different = json!({"name": "Synthetic Wand", "system": {"damage": "4"}});

        assert_eq!(
            structural_signature(&left, false),
            structural_signature(&same_shape, false)
        );
        assert_ne!(
            structural_signature(&left, false),
            structural_signature(&different, false)
        );
    }

    #[test]
    fn signatures_ignore_dynamic_identifier_map_keys() {
        let left = json!({
            "system": {
                "activities": {
                    "aaaaaaaaaaaaaaaa": {"type": "attack", "target": {"kind": "single"}}
                }
            }
        });
        let same_shape = json!({
            "system": {
                "activities": {
                    "B9xY7wV5uT3sR1qP": {"type": "save", "target": {"kind": "area"}}
                }
            }
        });
        assert_eq!(
            structural_signature(&left, true),
            structural_signature(&same_shape, true)
        );
        assert_ne!(
            structural_signature(&left, false),
            structural_signature(&same_shape, false)
        );
    }
}
