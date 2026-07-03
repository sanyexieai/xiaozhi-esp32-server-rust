use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

/// 与 Go `GenerateConfigKey` 对齐：provider + config 的稳定指纹，用于资源池 key。
pub fn generate_config_key(provider: &str, config: &Value) -> String {
    let canonical = Value::Object({
        let mut map = Map::new();
        map.insert("provider".into(), Value::String(provider.to_string()));
        map.insert("config".into(), sort_json(config));
        map
    });
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    let hash = Sha256::digest(bytes);
    format!("{:016x}", u128::from_be_bytes(hash[..16].try_into().unwrap_or([0; 16])))
}

fn sort_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<_> = map.keys().cloned().collect();
            keys.sort();
            let mut sorted = Map::new();
            for key in keys {
                if let Some(v) = map.get(&key) {
                    sorted.insert(key, sort_json(v));
                }
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.iter().map(sort_json).collect()),
        other => other.clone(),
    }
}
