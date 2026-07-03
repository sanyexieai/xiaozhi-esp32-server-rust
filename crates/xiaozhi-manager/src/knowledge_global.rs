use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::db::Database;

/// 与 Go `buildKnowledgeGlobalConfigData` 对齐：从已启用的 `knowledge_search` 配置构建全局 provider 信息。
pub fn build_knowledge_global_config_data(db: &Database) -> Value {
    let fallback = json!({
        "default_provider": "dify",
        "providers": {},
    });

    let Ok(configs) = db.list_configs("knowledge_search") else {
        return fallback;
    };

    let mut selected: BTreeMap<String, crate::db::ConfigRow> = BTreeMap::new();
    for cfg in configs.into_iter().filter(|c| c.enabled) {
        let provider = cfg.provider.trim().to_lowercase();
        if provider.is_empty() {
            continue;
        }
        match selected.get(&provider) {
            None => {
                selected.insert(provider, cfg);
            }
            Some(prev) => {
                if !prev.is_default && cfg.is_default {
                    selected.insert(provider, cfg);
                }
            }
        }
    }

    if selected.is_empty() {
        return fallback;
    }

    let mut default_provider = String::new();
    let mut providers = serde_json::Map::new();
    for (provider, cfg) in &selected {
        let mut payload = serde_json::Map::new();
        if !cfg.json_data.trim().is_empty() {
            if let Ok(v) = serde_json::from_str::<Value>(&cfg.json_data) {
                if let Value::Object(map) = v {
                    payload = map;
                }
            }
        }
        providers.insert(provider.clone(), Value::Object(payload));
        if cfg.is_default {
            default_provider = provider.clone();
        }
    }

    if default_provider.is_empty() {
        default_provider = selected.keys().next().cloned().unwrap_or_else(|| "dify".to_string());
    }

    json!({
        "default_provider": default_provider,
        "providers": Value::Object(providers),
    })
}

pub fn resolve_default_knowledge_provider(db: &Database) -> String {
    build_knowledge_global_config_data(db)
        .get("default_provider")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "dify".to_string())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::db::{ConfigInput, Database};

    #[test]
    fn picks_default_provider_from_enabled_configs() {
        let path = std::env::temp_dir().join(format!(
            "xz-mgr-kb-global-{}.db",
            uuid::Uuid::new_v4()
        ));
        let db = Database::open(&PathBuf::from(path)).unwrap();
        db.create_config(&ConfigInput {
            r#type: "knowledge_search".into(),
            name: "rag".into(),
            config_id: "rag-1".into(),
            provider: "ragflow".into(),
            json_data: r#"{"api_url":"http://localhost"}"#.into(),
            enabled: true,
            is_default: false,
        })
        .unwrap();
        db.create_config(&ConfigInput {
            r#type: "knowledge_search".into(),
            name: "dify".into(),
            config_id: "dify-1".into(),
            provider: "dify".into(),
            json_data: r#"{"api_url":"http://dify"}"#.into(),
            enabled: true,
            is_default: true,
        })
        .unwrap();

        let global = build_knowledge_global_config_data(&db);
        assert_eq!(global["default_provider"], "dify");
        assert!(global["providers"]["dify"].is_object());
        assert_eq!(
            global["providers"]["dify"]["api_url"],
            "http://dify"
        );
    }
}
