//! Coze LLM 集成（对齐 Go `coze_llm`）

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use xiaozhi_core::{Error, Result};

use crate::llm_common::{
    build_provider_query, build_stable_user_id, get_conversation_id, normalize_api_token,
    send_llm_error, set_conversation_id, ConversationMap,
};
use crate::message::{ChatMessage, ToolInfo};
use crate::sse_stream::iter_sse_events;
use crate::traits::LlmProvider;

const DEFAULT_COZE_BASE_URL: &str = "https://api.coze.com";
const DEFAULT_CONNECTOR_ID: &str = "1024";
const DEFAULT_USER_PREFIX: &str = "xiaozhi";
const STREAM_CREATE_PATH: &str = "/v3/chat";

pub struct CozeLlmProvider {
    api_key: String,
    base_url: String,
    bot_id: String,
    user_prefix: String,
    connector_id: String,
    client: Client,
    conversation_ids: Arc<ConversationMap>,
}

#[derive(Debug, Deserialize)]
struct CozeStreamEvent {
    #[serde(default)]
    event: String,
    #[serde(default)]
    msg: String,
    #[serde(default)]
    conversation_id: String,
    message: Option<CozeEventMessage>,
    chat: Option<CozeEventChat>,
}

#[derive(Debug, Deserialize)]
struct CozeEventMessage {
    #[serde(default)]
    content: String,
}

#[derive(Debug, Deserialize)]
struct CozeEventChat {
    #[serde(default)]
    conversation_id: String,
    last_error: Option<CozeLastError>,
}

#[derive(Debug, Deserialize)]
struct CozeLastError {
    #[serde(default)]
    msg: String,
}

impl CozeLlmProvider {
    pub fn from_config(config: &serde_json::Value) -> Result<Self> {
        let api_key = normalize_api_token(
            config
                .get("api_key")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
        );
        if api_key.is_empty() {
            return Err(Error::Config("coze api_key不能为空".into()));
        }
        let bot_id = config
            .get("bot_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if bot_id.is_empty() {
            return Err(Error::Config("coze bot_id不能为空".into()));
        }
        let base_url = config
            .get("base_url")
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_COZE_BASE_URL)
            .trim()
            .trim_end_matches('/')
            .to_string();
        let user_prefix = config
            .get("user_prefix")
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_USER_PREFIX)
            .trim()
            .to_string();
        let connector_id = config
            .get("connector_id")
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_CONNECTOR_ID)
            .trim()
            .to_string();

        Ok(Self {
            api_key,
            base_url,
            bot_id,
            user_prefix,
            connector_id,
            client: Client::new(),
            conversation_ids: Arc::new(Mutex::new(std::collections::HashMap::new())),
        })
    }

    fn build_request_bodies(
        &self,
        session_id: &str,
        query: &str,
    ) -> Result<Vec<Value>> {
        let conversation_id = get_conversation_id(&self.conversation_ids, session_id);
        let base = json!({
            "bot_id": self.bot_id,
            "user_id": build_stable_user_id(&self.user_prefix, session_id),
            "stream": true,
            "connector_id": self.connector_id,
            "conversation_id": conversation_id,
            "additional_messages": [{
                "role": "user",
                "type": "question",
                "content": query,
                "content_type": "text",
            }],
        });

        let mut bodies = vec![base.clone()];
        if !self.connector_id.is_empty() {
            let mut no_connector = base.clone();
            if let Some(obj) = no_connector.as_object_mut() {
                obj.remove("connector_id");
            }
            bodies.push(no_connector);
        }
        if !conversation_id.is_empty() {
            let mut no_conv = base.clone();
            if let Some(obj) = no_conv.as_object_mut() {
                obj.insert("conversation_id".into(), json!(""));
            }
            bodies.push(no_conv.clone());
            if !self.connector_id.is_empty() {
                if let Some(obj) = no_conv.as_object_mut() {
                    obj.remove("connector_id");
                }
                bodies.push(no_conv);
            }
        }

        let mut unique = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for mut body in bodies {
            if let Some(obj) = body.as_object_mut() {
                obj.insert("stream".into(), json!(true));
            }
            let key = body.to_string();
            if seen.insert(key) {
                unique.push(body);
            }
        }
        Ok(unique)
    }

    async fn open_stream(&self, body: &Value) -> Result<String> {
        let url = format!("{}{}", self.base_url, STREAM_CREATE_PATH);
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .header("Accept", "text/event-stream")
            .json(body)
            .send()
            .await
            .map_err(|e| Error::Http(format!("coze请求失败: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::Http(format!(
                "coze请求失败 status={status} body={text}"
            )));
        }
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !content_type.contains("text/event-stream") {
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::Http(format!(
                "coze响应不是SSE流 content_type={content_type} body={text}"
            )));
        }
        resp.text()
            .await
            .map_err(|e| Error::Http(format!("读取 coze 流失败: {e}")))
    }
}

fn normalize_coze_stream_data(data: &str) -> String {
    let mut data = data.trim().to_string();
    for _ in 0..2 {
        if let Ok(decoded) = serde_json::from_str::<String>(&data) {
            data = decoded.trim().to_string();
        } else {
            break;
        }
    }
    data
}

fn is_coze_done_marker(data: &str) -> bool {
    let d = data.trim();
    d.eq_ignore_ascii_case("[DONE]") || d.eq_ignore_ascii_case("done")
}

fn extract_coze_message_content(data: &str, event: &CozeStreamEvent) -> String {
    if let Some(msg) = &event.message {
        let content = msg.content.trim();
        if !content.is_empty() {
            return content.to_string();
        }
    }
    if let Ok(v) = serde_json::from_str::<Value>(data) {
        if let Some(content) = v.get("content").and_then(|x| x.as_str()) {
            let content = content.trim();
            if !content.is_empty() {
                return content.to_string();
            }
        }
        if let Some(msg) = v.get("message") {
            if let Some(content) = msg.get("content").and_then(|x| x.as_str()) {
                let content = content.trim();
                if !content.is_empty() {
                    return content.to_string();
                }
            }
        }
        if let Some(delta) = v.get("delta") {
            if let Some(content) = delta.get("content").and_then(|x| x.as_str()) {
                let content = content.trim();
                if !content.is_empty() {
                    return content.to_string();
                }
            }
        }
    }
    String::new()
}

fn extract_coze_conversation_id(event: &CozeStreamEvent, data: &str) -> String {
    if !event.conversation_id.trim().is_empty() {
        return event.conversation_id.trim().to_string();
    }
    if let Some(chat) = &event.chat {
        if !chat.conversation_id.trim().is_empty() {
            return chat.conversation_id.trim().to_string();
        }
    }
    if let Ok(v) = serde_json::from_str::<Value>(data) {
        if let Some(cid) = v.get("conversation_id").and_then(|x| x.as_str()) {
            let cid = cid.trim();
            if !cid.is_empty() {
                return cid.to_string();
            }
        }
    }
    String::new()
}

fn extract_coze_error(event: &CozeStreamEvent, data: &str) -> String {
    if let Some(chat) = &event.chat {
        if let Some(err) = &chat.last_error {
            if !err.msg.trim().is_empty() {
                return err.msg.trim().to_string();
            }
        }
    }
    if !event.msg.trim().is_empty() {
        return event.msg.trim().to_string();
    }
    if let Ok(v) = serde_json::from_str::<Value>(data) {
        if let Some(msg) = v.get("msg").and_then(|x| x.as_str()) {
            if !msg.trim().is_empty() {
                return msg.trim().to_string();
            }
        }
    }
    "coze返回错误".to_string()
}

fn process_coze_sse(
    body: &str,
    session_id: &str,
    conversation_ids: &ConversationMap,
    tx: &mpsc::Sender<ChatMessage>,
) {
    let mut seen_delta = false;
    for event in iter_sse_events(body) {
        let mut event_type = event.event_type.trim().to_string();
        if event_type.eq_ignore_ascii_case("done") {
            return;
        }
        let data = normalize_coze_stream_data(&event.data);
        if data.is_empty() || is_coze_done_marker(&data) {
            return;
        }
        let Ok(stream_event) = serde_json::from_str::<CozeStreamEvent>(&data) else {
            continue;
        };
        let cid = extract_coze_conversation_id(&stream_event, &data);
        if !cid.is_empty() {
            set_conversation_id(conversation_ids, session_id, &cid);
        }
        if event_type.is_empty() {
            event_type = stream_event.event.trim().to_string();
        }
        match event_type.as_str() {
            "conversation.message.delta" => {
                let content = extract_coze_message_content(&data, &stream_event);
                if !content.is_empty() {
                    seen_delta = true;
                    let _ = tx.try_send(ChatMessage::assistant(content));
                }
            }
            "conversation.message.completed" => {
                let content = extract_coze_message_content(&data, &stream_event);
                if !content.is_empty() {
                    if !seen_delta {
                        let _ = tx.try_send(ChatMessage::assistant(&content));
                    }
                    seen_delta = true;
                }
            }
            "conversation.chat.completed" | "done" => return,
            "conversation.chat.failed" | "error" => {
                send_llm_error(tx, extract_coze_error(&stream_event, &data));
                return;
            }
            _ => {
                let content = extract_coze_message_content(&data, &stream_event);
                if !content.is_empty() {
                    seen_delta = true;
                    let _ = tx.try_send(ChatMessage::assistant(content));
                }
            }
        }
    }
}

#[async_trait]
impl LlmProvider for CozeLlmProvider {
    async fn response_with_context(
        &self,
        session_id: &str,
        dialogue: &[ChatMessage],
        _tools: &[ToolInfo],
    ) -> Result<mpsc::Receiver<ChatMessage>> {
        let (tx, rx) = mpsc::channel(64);
        let query = build_provider_query(dialogue);
        if query.is_empty() {
            send_llm_error(&tx, "coze query不能为空");
            return Ok(rx);
        }

        let bodies = match self.build_request_bodies(session_id, &query) {
            Ok(v) => v,
            Err(e) => {
                send_llm_error(&tx, e.to_string());
                return Ok(rx);
            }
        };

        let mut body_text = None;
        let mut last_err = String::new();
        for (i, body) in bodies.iter().enumerate() {
            match self.open_stream(body).await {
                Ok(text) => {
                    body_text = Some(text);
                    break;
                }
                Err(e) => {
                    if i == 0 && bodies.len() > 1 {
                        tracing::warn!("coze首个请求失败，尝试回退重试: {e}");
                    }
                    last_err = e.to_string();
                }
            }
        }

        let Some(text) = body_text else {
            send_llm_error(&tx, last_err);
            return Ok(rx);
        };

        let conversation_ids = Arc::clone(&self.conversation_ids);
        let session_id = session_id.to_string();
        tokio::spawn(async move {
            process_coze_sse(&text, &session_id, &conversation_ids, &tx);
        });

        Ok(rx)
    }

    async fn response_with_vllm(&self, _file: &[u8], _text: &str, _mime_type: &str) -> Result<String> {
        Err(Error::Unsupported("coze provider不支持vllm能力".into()))
    }

    fn get_model_info(&self) -> serde_json::Value {
        json!({
            "type": "coze",
            "provider": "coze",
            "base_url": self.base_url,
            "bot_id": self.bot_id,
            "user_prefix": self.user_prefix,
            "connector_id": self.connector_id,
        })
    }

    async fn close(&self) -> Result<()> {
        Ok(())
    }

    fn is_valid(&self) -> bool {
        !self.api_key.is_empty() && !self.bot_id.is_empty()
    }
}
