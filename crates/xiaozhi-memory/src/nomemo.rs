use async_trait::async_trait;
use xiaozhi_core::Result;
use xiaozhi_llm::ChatMessage;

use crate::traits::MemoryProvider;

pub struct NoMemoProvider;

#[async_trait]
impl MemoryProvider for NoMemoProvider {
    async fn add_message(&self, _: &str, _: ChatMessage) -> Result<()> {
        Ok(())
    }
    async fn get_messages(&self, _: &str, _: usize) -> Result<Vec<ChatMessage>> {
        Ok(vec![])
    }
    async fn get_context(&self, _: &str, _: usize) -> Result<String> {
        Ok(String::new())
    }
    async fn search(&self, _: &str, _: &str, _: usize, _: i64) -> Result<String> {
        Ok(String::new())
    }
    async fn flush(&self, _: &str) -> Result<()> {
        Ok(())
    }
    async fn reset_memory(&self, _: &str) -> Result<()> {
        Ok(())
    }
}
