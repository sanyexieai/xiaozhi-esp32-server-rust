use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::sync::mpsc;
use xiaozhi_core::{cloud, Error, Result};

use crate::completion::{LlmCompletion, ToolCall};
use crate::message::{ChatMessage, MessageRole, ToolInfo};
use crate::openai_stream::StreamingToolCallAccumulator;
use crate::traits::LlmProvider;

fn build_http_client(base_url: &str) -> Client {
    let mut builder = Client::builder().connect_timeout(Duration::from_secs(30));
    if cloud::should_bypass_proxy(base_url) {
        builder = builder.no_proxy();
    }
    builder
        .build()
        .unwrap_or_else(|_| Client::new())
}

pub struct OpenAiLlmProvider {
    model_name: String,
    api_key: String,
    base_url: String,
    max_tokens: u32,
    client: Client,
}

impl OpenAiLlmProvider {
    pub fn from_config(config: &serde_json::Value) -> Result<Self> {
        let model_name = config
            .get("model_name")
            .and_then(|v| v.as_str())
            .unwrap_or("gpt-4o-mini")
            .to_string();
        let base_url = config
            .get("base_url")
            .and_then(|v| v.as_str())
            .unwrap_or("https://api.openai.com/v1")
            .trim_end_matches('/')
            .to_string();
        let mut api_key = cloud::trimmed_config_string(config, "api_key");
        if api_key.is_empty() && base_url.contains("dashscope.aliyuncs.com") {
            api_key = cloud::dashscope_api_key(config);
        }
        let max_tokens = config
            .get("max_tokens")
            .or_else(|| config.get("max_token"))
            .and_then(|v| v.as_u64())
            .unwrap_or(500) as u32;

        Ok(Self {
            model_name,
            api_key,
            base_url: base_url.clone(),
            max_tokens,
            client: build_http_client(&base_url),
        })
    }

    fn build_messages(dialogue: &[ChatMessage]) -> Vec<serde_json::Value> {
        dialogue
            .iter()
            .map(|m| {
                let role = match m.role {
                    MessageRole::System => "system",
                    MessageRole::User => "user",
                    MessageRole::Assistant => "assistant",
                    MessageRole::Tool => "tool",
                };
                let mut obj = serde_json::Map::new();
                obj.insert("role".into(), json!(role));
                obj.insert("content".into(), json!(m.content));
                if let Some(id) = &m.tool_call_id {
                    obj.insert("tool_call_id".into(), json!(id));
                }
                if let Some(calls) = m.extra.get("tool_calls") {
                    obj.insert("tool_calls".into(), Self::normalize_tool_calls(calls));
                }
                Value::Object(obj)
            })
            .collect()
    }

    fn normalize_tool_calls(value: &Value) -> Value {
        let Some(arr) = value.as_array() else {
            return value.clone();
        };
        Value::Array(
            arr.iter()
                .map(|call| {
                    let mut call = call.clone();
                    if let Some(func) = call.get_mut("function").and_then(|v| v.as_object_mut()) {
                        if let Some(args) = func.get("arguments") {
                            if !args.is_string() {
                                func.insert(
                                    "arguments".into(),
                                    Value::String(args.to_string()),
                                );
                            }
                        }
                    }
                    call
                })
                .collect(),
        )
    }

    fn normalize_tool_parameters(params: &Value) -> Value {
        if let Some(text) = params.as_str() {
            if let Ok(parsed) = serde_json::from_str::<Value>(text) {
                return Self::normalize_tool_parameters(&parsed);
            }
            return json!({"type": "object", "properties": {}});
        }
        if let Some(obj) = params.as_object() {
            let mut normalized = params.clone();
            if normalized.get("type").is_none() {
                normalized
                    .as_object_mut()
                    .expect("tool parameters object")
                    .insert("type".into(), json!("object"));
            }
            return normalized;
        }
        json!({"type": "object", "properties": {}})
    }

    fn build_tools(tools: &[ToolInfo]) -> Vec<serde_json::Value> {
        tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": Self::normalize_tool_parameters(&t.parameters),
                    }
                })
            })
            .collect()
    }

    fn parse_tool_calls(v: &serde_json::Value) -> Vec<ToolCall> {
        v.get("tool_calls")
            .and_then(|x| x.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|call| {
                        let id = call.get("id")?.as_str()?.to_string();
                        let func = call.get("function")?;
                        let name = func.get("name")?.as_str()?.to_string();
                        let args = func.get("arguments").cloned().unwrap_or(json!({}));
                        let arguments = if args.is_string() {
                            serde_json::from_str(args.as_str()?).unwrap_or(json!({}))
                        } else {
                            args
                        };
                        Some(ToolCall { id, name, arguments })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[async_trait]
impl LlmProvider for OpenAiLlmProvider {
    async fn complete_with_context(
        &self,
        _session_id: &str,
        dialogue: &[ChatMessage],
        tools: &[ToolInfo],
    ) -> Result<LlmCompletion> {
        let url = format!("{}/chat/completions", self.base_url);
        let mut body = json!({
            "model": self.model_name,
            "messages": Self::build_messages(dialogue),
            "max_tokens": self.max_tokens,
            "stream": false,
        });
        if !tools.is_empty() {
            body["tools"] = json!(Self::build_tools(tools));
        }

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Http(format!("LLM 请求失败 ({url}): {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::Http(format!("LLM HTTP {status}: {text}")));
        }

        let v: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Http(format!("LLM 解析失败: {e}")))?;
        let message = &v["choices"][0]["message"];
        Ok(LlmCompletion {
            content: message["content"].as_str().unwrap_or("").to_string(),
            tool_calls: Self::parse_tool_calls(message),
        })
    }

    async fn response_with_context(
        &self,
        _session_id: &str,
        dialogue: &[ChatMessage],
        _tools: &[ToolInfo],
    ) -> Result<mpsc::Receiver<ChatMessage>> {
        let (tx, rx) = mpsc::channel(64);
        let url = format!("{}/chat/completions", self.base_url);
        let mut body = json!({
            "model": self.model_name,
            "messages": Self::build_messages(dialogue),
            "max_tokens": self.max_tokens,
            "stream": true,
        });
        if !_tools.is_empty() {
            body["tools"] = json!(Self::build_tools(_tools));
        }

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Http(format!("LLM 请求失败 ({url}): {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::Http(format!("LLM HTTP {status}: {text}")));
        }

        let mut stream = resp.bytes_stream();
        tokio::spawn(async move {
            let mut buffer = String::new();
            let mut tool_acc = StreamingToolCallAccumulator::default();
            let mut content = String::new();
            while let Some(chunk) = stream.next().await {
                let Ok(bytes) = chunk else { break };
                buffer.push_str(&String::from_utf8_lossy(&bytes));

                while let Some(line_end) = buffer.find('\n') {
                    let line = buffer[..line_end].trim().to_string();
                    buffer = buffer[line_end + 1..].to_string();

                    if !line.starts_with("data: ") {
                        continue;
                    }
                    let data = &line[6..];
                    if data == "[DONE]" {
                        if let Some(calls) = tool_acc.to_tool_calls_json() {
                            let msg = ChatMessage::assistant(&content).with_tool_calls(calls);
                            let _ = tx.send(msg).await;
                        }
                        return;
                    }
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                        let delta = &v["choices"][0]["delta"];
                        tool_acc.merge_delta(delta);
                        if let Some(piece) = delta["content"].as_str() {
                            if !piece.is_empty() {
                                content.push_str(piece);
                                let _ = tx.send(ChatMessage::assistant(piece)).await;
                            }
                        }
                    }
                }
            }
            if let Some(calls) = tool_acc.to_tool_calls_json() {
                let msg = ChatMessage::assistant(&content).with_tool_calls(calls);
                let _ = tx.send(msg).await;
            }
        });

        Ok(rx)
    }

    async fn response_with_vllm(&self, file: &[u8], text: &str, mime_type: &str) -> Result<String> {
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(file);
        let url = format!("{}/chat/completions", self.base_url);
        let body = json!({
            "model": self.model_name,
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": text},
                    {"type": "image_url", "image_url": {"url": format!("data:{};base64,{}", mime_type, b64)}}
                ]
            }],
            "max_tokens": self.max_tokens,
        });

        let resp: serde_json::Value = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Http(format!("VLLM 请求失败: {e}")))?
            .json()
            .await
            .map_err(|e| Error::Http(format!("VLLM 解析失败: {e}")))?;

        Ok(resp["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string())
    }

    fn get_model_info(&self) -> serde_json::Value {
        json!({
            "model_name": self.model_name,
            "base_url": self.base_url,
            "max_tokens": self.max_tokens,
        })
    }

    async fn close(&self) -> Result<()> {
        Ok(())
    }

    fn is_valid(&self) -> bool {
        !self.api_key.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{ChatMessage, MessageRole, ToolInfo};

    #[test]
    fn normalize_tool_calls_serializes_arguments_as_string() {
        let calls = json!([{
            "id": "call_1",
            "type": "function",
            "function": {
                "name": "demo_tool",
                "arguments": {"city": "上海"}
            }
        }]);
        let normalized = OpenAiLlmProvider::normalize_tool_calls(&calls);
        assert_eq!(
            normalized[0]["function"]["arguments"],
            json!("{\"city\":\"上海\"}")
        );
    }

    #[test]
    fn build_messages_keeps_user_content_as_string() {
        let dialogue = vec![
            ChatMessage::system("你是助手"),
            ChatMessage::user("麦当劳今天有什么吃的"),
        ];
        let messages = OpenAiLlmProvider::build_messages(&dialogue);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1]["role"], "user");
        assert!(messages[1]["content"].is_string());
    }

    #[test]
    fn normalize_tool_parameters_adds_object_type() {
        let params = json!({"properties": {"q": {"type": "string"}}});
        let normalized = OpenAiLlmProvider::normalize_tool_parameters(&params);
        assert_eq!(normalized["type"], "object");
    }

    #[test]
    fn build_tools_uses_normalized_parameters() {
        let tools = vec![ToolInfo {
            name: "demo".into(),
            description: "demo".into(),
            parameters: json!({"properties": {"q": {"type": "string"}}}),
        }];
        let built = OpenAiLlmProvider::build_tools(&tools);
        assert_eq!(built[0]["function"]["parameters"]["type"], "object");
    }
}
