use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::message::ChatMessage;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Default)]
pub struct LlmCompletion {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
}

impl LlmCompletion {
    pub fn has_tool_calls(&self) -> bool {
        !self.tool_calls.is_empty()
    }

    pub fn to_assistant_message(&self) -> ChatMessage {
        let mut msg = ChatMessage::assistant(&self.content);
        if !self.tool_calls.is_empty() {
            let calls: Vec<Value> = self
                .tool_calls
                .iter()
                .map(|c| {
                    serde_json::json!({
                        "id": c.id,
                        "type": "function",
                        "function": {
                            "name": c.name,
                            "arguments": c.arguments.to_string(),
                        }
                    })
                })
                .collect();
            msg = msg.with_tool_calls(Value::Array(calls));
        }
        msg
    }
}
