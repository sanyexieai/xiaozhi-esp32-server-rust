//! Mem0 记忆服务

use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;
use xiaozhi_core::Result;
use xiaozhi_llm::{ChatMessage, MessageRole};

use crate::traits::MemoryProvider;

pub struct Mem0Provider {
    base_url: String,
    api_key: String,
    client: Client,
}

impl Mem0Provider {
    pub fn from_config(config: &serde_json::Value) -> Result<Self> {
        Ok(Self {
            base_url: config
                .get("base_url")
                .and_then(|v| v.as_str())
                .unwrap_or("https://api.mem0.ai")
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

fn parse_mem0_messages(body: Value, count: usize) -> Vec<ChatMessage> {
    let items = body
        .as_array()
        .cloned()
        .or_else(|| body.get("results").and_then(|v| v.as_array()).cloned())
        .or_else(|| body.get("memories").and_then(|v| v.as_array()).cloned())
        .unwrap_or_default();

    let mut messages = Vec::new();
    for item in items {
        if messages.len() >= count {
            break;
        }
        let memory = item
            .get("memory")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let metadata = item.get("metadata").and_then(|v| v.as_object());
        let role_str = metadata
            .and_then(|m| m.get("role"))
            .and_then(|v| v.as_str())
            .unwrap_or("assistant");
        let content = metadata
            .and_then(|m| m.get("content"))
            .and_then(|v| v.as_str())
            .unwrap_or(memory)
            .trim();
        if content.is_empty() {
            continue;
        }
        let role = match role_str {
            "user" => MessageRole::User,
            "system" => MessageRole::System,
            "tool" => MessageRole::Tool,
            _ => MessageRole::Assistant,
        };
        messages.push(ChatMessage {
            role,
            content: content.to_string(),
            name: None,
            tool_call_id: None,
            extra: Default::default(),
        });
    }
    messages
}

#[async_trait]
impl MemoryProvider for Mem0Provider {
    async fn add_message(&self, agent_id: &str, msg: ChatMessage) -> Result<()> {
        let url = format!("{}/v1/memories/", self.base_url.trim_end_matches('/'));
        let role = match msg.role {
            MessageRole::User => "user",
            MessageRole::System => "system",
            MessageRole::Tool => "tool",
            MessageRole::Assistant => "assistant",
        };
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Token {}", self.api_key))
            .json(&serde_json::json!({
                "user_id": agent_id,
                "messages": [{"role": role, "content": msg.content}]
            }))
            .send()
            .await
            .map_err(|e| xiaozhi_core::Error::Http(format!("Mem0 写入失败: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(xiaozhi_core::Error::Http(format!(
                "Mem0 写入失败: HTTP {status} {body}"
            )));
        }
        Ok(())
    }

    async fn get_messages(&self, agent_id: &str, count: usize) -> Result<Vec<ChatMessage>> {
        let agent_id = agent_id.trim();
        if agent_id.is_empty() {
            return Ok(vec![]);
        }
        let limit = count.clamp(1, 100);
        let url = format!(
            "{}/v1/memories/?user_id={agent_id}&limit={limit}",
            self.base_url.trim_end_matches('/')
        );
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Token {}", self.api_key))
            .send()
            .await
            .map_err(|e| xiaozhi_core::Error::Http(format!("Mem0 请求失败: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(xiaozhi_core::Error::Http(format!(
                "Mem0 获取记忆失败: HTTP {status} {body}"
            )));
        }
        let body: Value = resp
            .json()
            .await
            .map_err(|e| xiaozhi_core::Error::Http(format!("Mem0 响应解析失败: {e}")))?;
        Ok(parse_mem0_messages(body, limit))
    }

    async fn get_context(&self, agent_id: &str, _: usize) -> Result<String> {
        let url = format!(
            "{}/v1/memories/?user_id={}",
            self.base_url.trim_end_matches('/'),
            agent_id
        );
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Token {}", self.api_key))
            .send()
            .await
            .map_err(|e| xiaozhi_core::Error::Http(format!("Mem0 请求失败: {e}")))?;
        Ok(resp.text().await.unwrap_or_default())
    }

    async fn search(
        &self,
        agent_id: &str,
        query: &str,
        _: usize,
        _: i64,
    ) -> Result<String> {
        let url = format!("{}/v1/memories/search/", self.base_url.trim_end_matches('/'));
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Token {}", self.api_key))
            .json(&serde_json::json!({"user_id": agent_id, "query": query}))
            .send()
            .await
            .map_err(|e| xiaozhi_core::Error::Http(format!("Mem0 搜索失败: {e}")))?;
        Ok(resp.text().await.unwrap_or_default())
    }

    async fn flush(&self, _: &str) -> Result<()> {
        Ok(())
    }

    async fn reset_memory(&self, agent_id: &str) -> Result<()> {
        let url = format!(
            "{}/v1/memories/?user_id={}",
            self.base_url.trim_end_matches('/'),
            agent_id
        );
        let _ = self
            .client
            .delete(&url)
            .header("Authorization", format!("Token {}", self.api_key))
            .send()
            .await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_mem0_memory_items() {
        let body = json!([
            {
                "memory": "fallback text",
                "metadata": { "role": "user", "content": "你好" }
            },
            {
                "memory": "assistant reply",
                "metadata": { "role": "assistant" }
            }
        ]);
        let msgs = parse_mem0_messages(body, 10);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, MessageRole::User);
        assert_eq!(msgs[0].content, "你好");
        assert_eq!(msgs[1].role, MessageRole::Assistant);
        assert_eq!(msgs[1].content, "assistant reply");
    }
}
