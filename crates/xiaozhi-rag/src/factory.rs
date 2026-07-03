use std::sync::Arc;

use reqwest::Client;
use xiaozhi_config::user::KnowledgeSearchHit;
use xiaozhi_core::{Error, Result, rag as rag_const};

use crate::local::LocalSearcher;
use crate::traits::RagSearcher;

macro_rules! impl_rag {
    ($name:ident, $provider:expr) => {
        pub struct $name {
            base_url: String,
            api_key: String,
            client: Client,
        }

        impl $name {
            pub fn from_config(config: &serde_json::Value) -> Result<Self> {
                Ok(Self {
                    base_url: config
                        .get("base_url")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    api_key: config
                        .get("api_key")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    client: Client::new(),
                })
            }
        }

        #[async_trait::async_trait]
        impl RagSearcher for $name {
            async fn search(
                &self,
                query: &str,
                kb_id: &str,
                top_k: usize,
                threshold: f64,
            ) -> Result<Vec<KnowledgeSearchHit>> {
                tracing::debug!(
                    "{} RAG search: query={query}, kb={kb_id}, top_k={top_k}, threshold={threshold}",
                    $provider
                );
                let url = format!("{}/search", self.base_url.trim_end_matches('/'));
                let resp = self
                    .client
                    .post(&url)
                    .bearer_auth(&self.api_key)
                    .json(&serde_json::json!({
                        "query": query,
                        "kb_id": kb_id,
                        "top_k": top_k,
                        "threshold": threshold,
                    }))
                    .send()
                    .await
                    .map_err(|e| Error::Http(format!("{} RAG 失败: {e}", $provider)))?;

                if !resp.status().is_success() {
                    return Ok(vec![]);
                }

                let hits: Vec<KnowledgeSearchHit> = resp.json().await.unwrap_or_default();
                Ok(hits)
            }
        }
    };
}

pub mod dify {
    use super::*;
    impl_rag!(DifySearcher, "dify");
}
pub mod ragflow {
    use super::*;
    impl_rag!(RagflowSearcher, "ragflow");
}
pub mod weknora {
    use super::*;
    impl_rag!(WeKnoraSearcher, "weknora");
}

pub fn create_searcher(provider: &str, config: &serde_json::Value) -> Result<Arc<dyn RagSearcher>> {
    match provider {
        rag_const::LOCAL => {
            let docs: Vec<(String, String)> = config
                .get("documents")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| {
                            let title = item.get("title")?.as_str()?.to_string();
                            let content = item.get("content")?.as_str()?.to_string();
                            Some((title, content))
                        })
                        .collect()
                })
                .unwrap_or_default();
            Ok(Arc::new(LocalSearcher::from_documents(docs)))
        }
        rag_const::DIFY => Ok(Arc::new(dify::DifySearcher::from_config(config)?)),
        rag_const::RAGFLOW => Ok(Arc::new(ragflow::RagflowSearcher::from_config(config)?)),
        rag_const::WEKNORA => Ok(Arc::new(weknora::WeKnoraSearcher::from_config(config)?)),
        other => Err(Error::Unsupported(format!("不支持的 RAG 类型: {other}"))),
    }
}
