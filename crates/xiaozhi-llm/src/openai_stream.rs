use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::completion::ToolCall;

#[derive(Debug, Default)]
struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
}

/// 合并 OpenAI 流式 `delta.tool_calls` 分片。
#[derive(Debug, Default)]
pub struct StreamingToolCallAccumulator {
    calls: BTreeMap<usize, PartialToolCall>,
}

impl StreamingToolCallAccumulator {
    pub fn merge_delta(&mut self, delta: &Value) {
        let Some(items) = delta.get("tool_calls").and_then(|v| v.as_array()) else {
            return;
        };
        for item in items {
            let index = item.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let entry = self.calls.entry(index).or_default();
            if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                if !id.is_empty() {
                    entry.id = id.to_string();
                }
            }
            if let Some(func) = item.get("function") {
                if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
                    if !name.is_empty() {
                        entry.name = name.to_string();
                    }
                }
                if let Some(args) = func.get("arguments").and_then(|v| v.as_str()) {
                    entry.arguments.push_str(args);
                }
            }
        }
    }

    pub fn into_tool_calls(self) -> Vec<ToolCall> {
        self.calls
            .into_values()
            .filter_map(|partial| {
                if partial.id.is_empty() || partial.name.is_empty() {
                    return None;
                }
                let arguments = if partial.arguments.trim().is_empty() {
                    json!({})
                } else {
                    serde_json::from_str(&partial.arguments).unwrap_or(json!({}))
                };
                Some(ToolCall {
                    id: partial.id,
                    name: partial.name,
                    arguments,
                })
            })
            .collect()
    }

    pub fn to_tool_calls_json(&self) -> Option<Value> {
        let calls = self
            .calls
            .values()
            .filter(|p| !p.id.is_empty() && !p.name.is_empty())
            .map(|p| {
                json!({
                    "id": p.id,
                    "type": "function",
                    "function": {
                        "name": p.name,
                        "arguments": p.arguments,
                    }
                })
            })
            .collect::<Vec<_>>();
        if calls.is_empty() {
            None
        } else {
            Some(Value::Array(calls))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_streaming_tool_call_fragments() {
        let mut acc = StreamingToolCallAccumulator::default();
        acc.merge_delta(&json!({
            "tool_calls": [{
                "index": 0,
                "id": "call_1",
                "function": { "name": "get_weather", "arguments": "" }
            }]
        }));
        acc.merge_delta(&json!({
            "tool_calls": [{
                "index": 0,
                "function": { "arguments": "{\"city\":" }
            }]
        }));
        acc.merge_delta(&json!({
            "tool_calls": [{
                "index": 0,
                "function": { "arguments": "\"上海\"}" }
            }]
        }));
        let calls = acc.into_tool_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "get_weather");
        assert_eq!(calls[0].arguments["city"], "上海");
    }
}
