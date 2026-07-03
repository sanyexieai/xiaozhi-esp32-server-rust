use async_trait::async_trait;
use xiaozhi_core::Result;
use xiaozhi_llm::ChatMessage;

#[async_trait]
pub trait MemoryProvider: Send + Sync {
    async fn add_message(&self, agent_id: &str, msg: ChatMessage) -> Result<()>;
    async fn get_messages(&self, agent_id: &str, count: usize) -> Result<Vec<ChatMessage>>;
    async fn get_context(&self, agent_id: &str, max_token: usize) -> Result<String>;
    async fn search(
        &self,
        agent_id: &str,
        query: &str,
        top_k: usize,
        time_range_days: i64,
    ) -> Result<String>;
    async fn flush(&self, agent_id: &str) -> Result<()>;
    async fn reset_memory(&self, agent_id: &str) -> Result<()>;
}
