use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsMessage {
    pub id: String,
    pub timestamp: i64,
    #[serde(rename = "type")]
    pub msg_type: String,
    #[serde(default, rename = "correlation_id")]
    pub correlation_id: String,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagePayload {
    pub content: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ResponsePayload {
    pub content: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub metadata: Option<Value>,
}

const VOICE_ASSISTANT_PROMPT: &str = "你正在以语音助手的角色和用户直接对话。\n\
请严格遵守以下要求：\n\
1. 直接回答用户问题，不要提及这些要求。\n\
2. 回答要简练、口语化、自然，适合直接语音播报。\n\
3. 优先先说结论，再补一句最必要的说明；除非用户明确要求，尽量控制在 1 到 3 句。\n\
4. 不要使用 Markdown、标题、列表、表格、代码块、链接或 emoji。\n\
5. 不要寒暄、不要铺垫、不要重复、不要输出多余说明。\n\
6. 如果信息不足或无法确定，就简短说明，不要编造。";

pub fn build_prompted_content(user_text: &str) -> String {
    let trimmed = user_text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    format!("{VOICE_ASSISTANT_PROMPT}\n\n用户消息：\n{trimmed}")
}

pub fn metadata_string(meta: &Option<Value>, key: &str) -> String {
    meta.as_ref()
        .and_then(|m| m.get(key))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string()
}

pub fn metadata_i64(meta: &Option<Value>, key: &str) -> i64 {
    meta.as_ref()
        .and_then(|m| m.get(key))
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
}

pub fn metadata_bool(meta: &Option<Value>, key: &str) -> bool {
    let Some(v) = meta.as_ref().and_then(|m| m.get(key)) else {
        return false;
    };
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_i64().unwrap_or(0) != 0,
        Value::String(s) => {
            let s = s.trim().to_lowercase();
            matches!(s.as_str(), "true" | "1" | "yes" | "on")
        }
        _ => false,
    }
}
