use std::fmt::Write;

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

pub fn structural_signature(value: &Value) -> String {
    let shape = shape_of(value);
    let encoded = serde_json::to_vec(&shape).expect("shape serialization cannot fail");
    let digest = Sha256::digest(encoded);
    digest.iter().fold(String::new(), |mut output, byte| {
        write!(output, "{byte:02x}").expect("writing to a String cannot fail");
        output
    })
}

fn shape_of(value: &Value) -> Value {
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
                .map(shape_of)
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
            let shape = values
                .iter()
                .map(|(key, value)| (key.clone(), shape_of(value)))
                .collect::<Map<_, _>>();
            Value::Object(shape)
        }
    }
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
            structural_signature(&left),
            structural_signature(&same_shape)
        );
        assert_ne!(
            structural_signature(&left),
            structural_signature(&different)
        );
    }
}
