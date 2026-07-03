//! Memobase 记忆服务

use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;
use xiaozhi_core::Result;
use xiaozhi_llm::{ChatMessage, MessageRole};

use crate::traits::MemoryProvider;

pub struct MemobaseProvider {
    base_url: String,
    api_key: String,
    client: Client,
}

impl MemobaseProvider {
    pub fn from_config(config: &serde_json::Value) -> Result<Self> {
        Ok(Self {
            base_url: config
                .get("base_url")
                .and_then(|v| v.as_str())
                .unwrap_or("http://localhost:8019")
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

fn role_to_str(role: MessageRole) -> &'static str {
    match role {
        MessageRole::User => "user",
        MessageRole::System => "system",
        MessageRole::Tool => "tool",
        MessageRole::Assistant => "assistant",
    }
}

fn parse_role(role: &str) -> MessageRole {
    match role.trim().to_ascii_lowercase().as_str() {
        "user" => MessageRole::User,
        "system" => MessageRole::System,
        "tool" => MessageRole::Tool,
        _ => MessageRole::Assistant,
    }
}

fn parse_memobase_messages(body: Value, count: usize) -> Vec<ChatMessage> {
    let items = body
        .as_array()
        .cloned()
        .or_else(|| body.get("data").and_then(|v| v.as_array()).cloned())
        .or_else(|| body.get("results").and_then(|v| v.as_array()).cloned())
        .or_else(|| body.get("memories").and_then(|v| v.as_array()).cloned())
        .unwrap_or_default();

    let mut messages = Vec::new();
    for item in items {
        if messages.len() >= count {
            break;
        }
        let role = item
            .get("role")
            .and_then(|v| v.as_str())
            .map(parse_role)
            .unwrap_or(MessageRole::Assistant);
        let content = item
            .get("content")
            .and_then(|v| v.as_str())
            .or_else(|| item.get("memory").and_then(|v| v.as_str()))
            .unwrap_or("")
            .trim();
        if content.is_empty() {
            continue;
        }
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
impl MemoryProvider for MemobaseProvider {
    async fn add_message(&self, agent_id: &str, msg: ChatMessage) -> Result<()> {
        let url = format!("{}/v1/memories", self.base_url.trim_end_matches('/'));
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&serde_json::json!({
                "user_id": agent_id,
                "content": msg.content,
                "role": role_to_str(msg.role),
            }))
            .send()
            .await
            .map_err(|e| xiaozhi_core::Error::Http(format!("Memobase 写入失败: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(xiaozhi_core::Error::Http(format!(
                "Memobase 写入失败: HTTP {status} {body}"
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
            "{}/v1/memories?user_id={agent_id}&limit={limit}",
            self.base_url.trim_end_matches('/')
        );
        let resp = self
            .client
            .get(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await
            .map_err(|e| xiaozhi_core::Error::Http(format!("Memobase 请求失败: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(xiaozhi_core::Error::Http(format!(
                "Memobase 获取记忆失败: HTTP {status} {body}"
            )));
        }
        let body: Value = resp
            .json()
            .await
            .map_err(|e| xiaozhi_core::Error::Http(format!("Memobase 响应解析失败: {e}")))?;
        Ok(parse_memobase_messages(body, limit))
    }

    async fn get_context(&self, agent_id: &str, _: usize) -> Result<String> {
        let url = format!(
            "{}/v1/context/{}",
            self.base_url.trim_end_matches('/'),
            agent_id
        );
        let resp = self
            .client
            .get(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await
            .map_err(|e| xiaozhi_core::Error::Http(format!("Memobase 请求失败: {e}")))?;
        Ok(resp.text().await.unwrap_or_default())
    }

    async fn search(
        &self,
        agent_id: &str,
        query: &str,
        top_k: usize,
        _: i64,
    ) -> Result<String> {
        let url = format!("{}/v1/search", self.base_url.trim_end_matches('/'));
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&serde_json::json!({
                "user_id": agent_id,
                "query": query,
                "top_k": top_k,
            }))
            .send()
            .await
            .map_err(|e| xiaozhi_core::Error::Http(format!("Memobase 搜索失败: {e}")))?;
        Ok(resp.text().await.unwrap_or_default())
    }

    async fn flush(&self, _: &str) -> Result<()> {
        Ok(())
    }

    async fn reset_memory(&self, agent_id: &str) -> Result<()> {
        let url = format!(
            "{}/v1/memories/{}",
            self.base_url.trim_end_matches('/'),
            agent_id
        );
        let _ = self
            .client
            .delete(&url)
            .bearer_auth(&self.api_key)
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
    fn parses_memobase_message_items() {
        let body = json!([
            { "role": "user", "content": "你好" },
            { "role": "assistant", "content": "嗨" }
        ]);
        let msgs = parse_memobase_messages(body, 10);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, MessageRole::User);
        assert_eq!(msgs[1].role, MessageRole::Assistant);
    }

    #[test]
    fn role_to_str_uses_lowercase() {
        assert_eq!(role_to_str(MessageRole::User), "user");
        assert_eq!(role_to_str(MessageRole::Assistant), "assistant");
    }
}
