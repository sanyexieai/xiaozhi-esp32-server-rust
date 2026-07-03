use async_trait::async_trait;
use xiaozhi_config::user::KnowledgeSearchHit;
use xiaozhi_core::Result;

#[async_trait]
pub trait RagSearcher: Send + Sync {
    async fn search(
        &self,
        query: &str,
        kb_id: &str,
        top_k: usize,
        threshold: f64,
    ) -> Result<Vec<KnowledgeSearchHit>>;
}
