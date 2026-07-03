//! MemOS 记忆服务 (兼容 Mem0 API)

use async_trait::async_trait;
use xiaozhi_core::Result;
use xiaozhi_llm::ChatMessage;

use crate::mem0::Mem0Provider;
use crate::traits::MemoryProvider;

pub struct MemosProvider {
    inner: Mem0Provider,
}

impl MemosProvider {
    pub fn from_config(config: &serde_json::Value) -> Result<Self> {
        Ok(Self {
            inner: Mem0Provider::from_config(config)?,
        })
    }
}

#[async_trait]
impl MemoryProvider for MemosProvider {
    async fn add_message(&self, agent_id: &str, msg: ChatMessage) -> Result<()> {
        self.inner.add_message(agent_id, msg).await
    }
    async fn get_messages(&self, agent_id: &str, count: usize) -> Result<Vec<ChatMessage>> {
        self.inner.get_messages(agent_id, count).await
    }
    async fn get_context(&self, agent_id: &str, max_token: usize) -> Result<String> {
        self.inner.get_context(agent_id, max_token).await
    }
    async fn search(
        &self,
        agent_id: &str,
        query: &str,
        top_k: usize,
        time_range_days: i64,
    ) -> Result<String> {
        self.inner
            .search(agent_id, query, top_k, time_range_days)
            .await
    }
    async fn flush(&self, agent_id: &str) -> Result<()> {
        self.inner.flush(agent_id).await
    }
    async fn reset_memory(&self, agent_id: &str) -> Result<()> {
        self.inner.reset_memory(agent_id).await
    }
}
