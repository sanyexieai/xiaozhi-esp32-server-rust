//! 知识库检索：本地 SQLite + 外部 RAG（`xiaozhi-rag::create_searcher`）

use serde_json::{json, Value};
use xiaozhi_rag::create_searcher;

use crate::db::{Database, KbSearchHit, KnowledgeBaseDetail};

#[derive(Debug, Clone)]
pub struct AggregatedKbHit {
    pub knowledge_base_id: i64,
    pub document_id: Option<i64>,
    pub title: String,
    pub content: String,
    pub score: f64,
}

pub async fn search_knowledge_bases(
    db: &Database,
    kb_ids: &[i64],
    query: &str,
    top_k: usize,
    threshold: f64,
) -> Vec<AggregatedKbHit> {
    let query = query.trim();
    if query.is_empty() || kb_ids.is_empty() || top_k == 0 {
        return Vec::new();
    }

    let mut hits = Vec::new();
    for kb_id in kb_ids {
        let Some(kb) = db.get_knowledge_base(*kb_id).ok().flatten() else {
            continue;
        };
        if kb.status.eq_ignore_ascii_case("inactive") {
            continue;
        }
        let provider = normalize_kb_provider(&kb.provider);
        if provider == "local" {
            if let Ok(local_hits) = db.search_kb_documents(&[*kb_id], query, top_k, threshold) {
                hits.extend(local_hits.into_iter().map(map_local_hit));
            }
            continue;
        }
        match search_external_kb(db, &kb, &provider, query, top_k, threshold).await {
            Ok(mut external) => hits.append(&mut external),
            Err(e) => tracing::warn!(
                kb_id = kb.id,
                provider,
                "外部知识库检索失败: {e}"
            ),
        }
    }

    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    hits.truncate(top_k);
    hits
}

fn map_local_hit(hit: KbSearchHit) -> AggregatedKbHit {
    AggregatedKbHit {
        knowledge_base_id: hit.knowledge_base_id,
        document_id: Some(hit.document_id),
        title: hit.title,
        content: hit.content,
        score: hit.score,
    }
}

fn normalize_kb_provider(provider: &str) -> String {
    match provider.trim().to_lowercase().as_str() {
        "" | "local" => "local".to_string(),
        "dify" => "dify".to_string(),
        "ragflow" => "ragflow".to_string(),
        "weknora" => "weknora".to_string(),
        other => other.to_string(),
    }
}

async fn search_external_kb(
    db: &Database,
    kb: &KnowledgeBaseDetail,
    provider: &str,
    query: &str,
    top_k: usize,
    threshold: f64,
) -> Result<Vec<AggregatedKbHit>, String> {
    let cfg_row = db
        .find_knowledge_search_config(provider)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("未配置 {provider} 知识检索连接"))?;

    let mut cfg: Value = serde_json::from_str(&cfg_row.json_data).unwrap_or(json!({}));
    if let Value::Object(ref mut map) = cfg {
        if let Some(inner) = map.remove("config") {
            if let Value::Object(inner_map) = inner {
                for (k, v) in inner_map {
                    map.entry(k).or_insert(v);
                }
            }
        }
    }

    let searcher = create_searcher(provider, &cfg).map_err(|e| e.to_string())?;
    let external_kb_id = external_kb_id_from_json(&kb.config_json, kb.id);
    if external_kb_id.trim().is_empty() {
        return Ok(Vec::new());
    }

    let rag_hits = searcher
        .search(query, &external_kb_id, top_k, threshold)
        .await
        .map_err(|e| e.to_string())?;

    Ok(rag_hits
        .into_iter()
        .map(|hit| AggregatedKbHit {
            knowledge_base_id: kb.id,
            document_id: None,
            title: if hit.title.trim().is_empty() {
                kb.name.clone()
            } else {
                hit.title
            },
            content: hit.content,
            score: hit.score,
        })
        .collect())
}

pub fn external_kb_id_from_json(config_json: &str, local_id: i64) -> String {
    let Ok(v) = serde_json::from_str::<Value>(config_json) else {
        return local_id.to_string();
    };
    for key in [
        "external_kb_id",
        "dataset_id",
        "knowledge_base_id",
        "kb_id",
    ] {
        if let Some(s) = v.get(key).and_then(|x| x.as_str()) {
            let s = s.trim();
            if !s.is_empty() {
                return s.to_string();
            }
        }
    }
    local_id.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_external_kb_id_from_config_json() {
        let json = r#"{"external_kb_id":"ds-123"}"#;
        assert_eq!(external_kb_id_from_json(json, 9), "ds-123");
        assert_eq!(external_kb_id_from_json("{}", 9), "9");
    }
}
