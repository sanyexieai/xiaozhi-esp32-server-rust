use async_trait::async_trait;
use tokio::sync::mpsc;
use xiaozhi_core::Result;

use crate::completion::LlmCompletion;
use crate::message::{ChatMessage, ToolInfo};

pub const LLM_EXTRA_ERROR_KEY: &str = "error";

#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// 流式对话，返回消息 channel
    async fn response_with_context(
        &self,
        session_id: &str,
        dialogue: &[ChatMessage],
        tools: &[ToolInfo],
    ) -> Result<mpsc::Receiver<ChatMessage>>;

    /// 非流式对话，支持 tool_calls（默认回退为流式聚合）
    async fn complete_with_context(
        &self,
        session_id: &str,
        dialogue: &[ChatMessage],
        tools: &[ToolInfo],
    ) -> Result<LlmCompletion> {
        let mut rx = self
            .response_with_context(session_id, dialogue, tools)
            .await?;
        let mut content = String::new();
        while let Some(msg) = rx.recv().await {
            content.push_str(&msg.content);
        }
        Ok(LlmCompletion {
            content,
            tool_calls: Vec::new(),
        })
    }

    /// 视觉多模态
    async fn response_with_vllm(
        &self,
        file: &[u8],
        text: &str,
        mime_type: &str,
    ) -> Result<String>;

    fn get_model_info(&self) -> serde_json::Value;

    async fn close(&self) -> Result<()>;

    fn is_valid(&self) -> bool;
}
