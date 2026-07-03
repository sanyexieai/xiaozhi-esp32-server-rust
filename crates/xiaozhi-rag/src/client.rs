use reqwest::Client;
use xiaozhi_config::user::KnowledgeSearchHit;
use xiaozhi_config_provider::build_http_client;
use xiaozhi_core::Result;

pub struct KnowledgeClient {
    base_url: String,
    auth_token: String,
    client: Client,
}

impl KnowledgeClient {
    pub fn new(base_url: String, auth_token: String) -> Self {
        Self {
            base_url: base_url.clone(),
            auth_token,
            client: build_http_client(&base_url),
        }
    }

    pub async fn search(
        &self,
        kb_ids: &[u64],
        query: &str,
        top_k: usize,
        threshold: f64,
    ) -> Result<Vec<KnowledgeSearchHit>> {
        if kb_ids.is_empty() || query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let url = format!(
            "{}/api/internal/knowledge/search",
            self.base_url.trim_end_matches('/')
        );
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.auth_token))
            .json(&serde_json::json!({
                "knowledge_base_ids": kb_ids,
                "query": query,
                "top_k": top_k,
                "threshold": threshold,
            }))
            .send()
            .await
            .map_err(|e| xiaozhi_core::Error::Http(format!("知识库检索失败: {e}")))?;

        if !resp.status().is_success() {
            return Ok(Vec::new());
        }

        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        let results = body
            .get("results")
            .or_else(|| body.get("data").and_then(|d| d.get("results")))
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        Ok(results)
    }
}
