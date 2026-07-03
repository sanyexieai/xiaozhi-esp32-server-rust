//! Dify LLM 集成（对齐 Go `dify_llm`）

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;
use xiaozhi_core::{Error, Result};

use crate::llm_common::{
    build_provider_query, build_stable_user_id, get_conversation_id, send_llm_error,
    set_conversation_id, ConversationMap,
};
use crate::message::{ChatMessage, ToolInfo};
use crate::sse_stream::iter_sse_events;
use crate::traits::LlmProvider;

const DEFAULT_DIFY_BASE_URL: &str = "https://api.dify.ai/v1";
const DEFAULT_USER_PREFIX: &str = "xiaozhi";

pub struct DifyLlmProvider {
    api_key: String,
    base_url: String,
    user_prefix: String,
    client: Client,
    conversation_ids: Arc<ConversationMap>,
}

#[derive(Debug, Deserialize)]
struct DifyStreamEvent {
    #[serde(default)]
    event: String,
    #[serde(default)]
    answer: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    conversation_id: String,
}

impl DifyLlmProvider {
    pub fn from_config(config: &serde_json::Value) -> Result<Self> {
        let api_key = config
            .get("api_key")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if api_key.is_empty() {
            return Err(Error::Config("dify api_key不能为空".into()));
        }

        let mut base_url = config
            .get("base_url")
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_DIFY_BASE_URL)
            .trim()
            .trim_end_matches('/')
            .to_string();
        if !base_url.to_ascii_lowercase().ends_with("/v1") {
            base_url.push_str("/v1");
        }

        let user_prefix = config
            .get("user_prefix")
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_USER_PREFIX)
            .trim()
            .to_string();

        Ok(Self {
            api_key,
            base_url,
            user_prefix,
            client: Client::new(),
            conversation_ids: Arc::new(Mutex::new(std::collections::HashMap::new())),
        })
    }
}

#[async_trait]
impl LlmProvider for DifyLlmProvider {
    async fn response_with_context(
        &self,
        session_id: &str,
        dialogue: &[ChatMessage],
        _tools: &[ToolInfo],
    ) -> Result<mpsc::Receiver<ChatMessage>> {
        let (tx, rx) = mpsc::channel(64);
        let query = build_provider_query(dialogue);
        if query.is_empty() {
            send_llm_error(&tx, "dify query不能为空");
            return Ok(rx);
        }

        let user_id = build_stable_user_id(&self.user_prefix, session_id);
        let conversation_id = get_conversation_id(&self.conversation_ids, session_id);
        let mut body = json!({
            "inputs": {},
            "query": query,
            "response_mode": "streaming",
            "user": user_id,
        });
        if !conversation_id.is_empty() {
            body["conversation_id"] = json!(conversation_id);
        }

        let url = format!("{}/chat-messages", self.base_url);
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .header("Accept", "text/event-stream")
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Http(format!("Dify 请求失败: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            send_llm_error(&tx, format!("Dify 请求失败 status={status} body={text}"));
            return Ok(rx);
        }

        let text = resp.text().await.unwrap_or_default();
        let conversation_ids = Arc::clone(&self.conversation_ids);
        let session_id = session_id.to_string();
        tokio::spawn(async move {
            for event in iter_sse_events(&text) {
                let data = event.data.trim();
                if data.is_empty() || data == "[DONE]" {
                    continue;
                }
                let Ok(stream_event) = serde_json::from_str::<DifyStreamEvent>(data) else {
                    continue;
                };
                if !stream_event.conversation_id.is_empty() {
                    set_conversation_id(
                        &conversation_ids,
                        &session_id,
                        &stream_event.conversation_id,
                    );
                }
                match stream_event.event.as_str() {
                    "error" => {
                        let msg = if stream_event.message.is_empty() {
                            "dify返回错误".to_string()
                        } else {
                            stream_event.message
                        };
                        send_llm_error(&tx, msg);
                        return;
                    }
                    "message" | "agent_message" => {
                        if !stream_event.answer.is_empty() {
                            let _ = tx.send(ChatMessage::assistant(&stream_event.answer)).await;
                        }
                    }
                    "message_end" => return,
                    _ => {
                        if !stream_event.answer.is_empty() {
                            let _ = tx.send(ChatMessage::assistant(&stream_event.answer)).await;
                        }
                    }
                }
            }
        });

        Ok(rx)
    }

    async fn response_with_vllm(&self, _file: &[u8], _text: &str, _mime_type: &str) -> Result<String> {
        Err(Error::Unsupported("dify provider不支持vllm能力".into()))
    }

    fn get_model_info(&self) -> serde_json::Value {
        json!({
            "type": "dify",
            "provider": "dify",
            "base_url": self.base_url,
            "user_prefix": self.user_prefix,
        })
    }

    async fn close(&self) -> Result<()> {
        Ok(())
    }

    fn is_valid(&self) -> bool {
        !self.api_key.is_empty() && !self.base_url.is_empty()
    }
}
