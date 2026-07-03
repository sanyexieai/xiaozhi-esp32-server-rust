//! OpenAI/DeepSeek 工具名规范化（对齐 Go `internal/domain/mcp/tool_name.go`）

use std::collections::HashMap;

use crate::types::McpTool;

const LLM_TOOL_NAME_MAX_LEN: usize = 64;

fn is_llm_tool_name_alnum(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
}

fn is_llm_tool_name_punct(ch: char) -> bool {
    ch == '_' || ch == '-'
}

fn short_stable_hash(value: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:08x}", hasher.finish() as u32)
}

fn truncate_llm_tool_name(name: &str, original: &str) -> String {
    if name.len() <= LLM_TOOL_NAME_MAX_LEN {
        return name.to_string();
    }
    let hash_suffix = format!("_{}", short_stable_hash(original));
    let max_base_len = LLM_TOOL_NAME_MAX_LEN.saturating_sub(hash_suffix.len()).max(1);
    let mut base: String = name.chars().take(max_base_len).collect();
    while base.ends_with('_') || base.ends_with('-') {
        base.pop();
    }
    if base.is_empty() {
        base = "tool".to_string();
    }
    format!("{base}{hash_suffix}")
}

/// 将 MCP 工具名转换为 OpenAI function name 允许的格式。
pub fn sanitize_llm_tool_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let mut out = String::with_capacity(trimmed.len());
    let mut has_alnum = false;
    let mut last_was_replacement = false;

    for ch in trimmed.chars() {
        if is_llm_tool_name_alnum(ch) {
            out.push(ch);
            has_alnum = true;
            last_was_replacement = false;
        } else if is_llm_tool_name_punct(ch) {
            out.push(ch);
            last_was_replacement = false;
        } else if !last_was_replacement {
            out.push('_');
            last_was_replacement = true;
        }
    }

    let mut sanitized = out.trim_matches('_').trim_matches('-').to_string();
    if sanitized.is_empty() || !has_alnum {
        sanitized = format!("tool_{}", short_stable_hash(trimmed));
    }
    truncate_llm_tool_name(&sanitized, trimmed)
}

/// 保证 LLM 工具名唯一，并记录 origin 映射。
pub fn unique_llm_tool_name(
    candidate: &str,
    original: &str,
    used: &mut HashMap<String, String>,
) -> String {
    let mut candidate = if candidate.is_empty() {
        format!("tool_{}", short_stable_hash(original))
    } else {
        candidate.to_string()
    };

    if used.get(&candidate).is_none_or(|prev| prev == original) {
        used.insert(candidate.clone(), original.to_string());
        return candidate;
    }

    let hash_suffix = format!("_{}", short_stable_hash(original));
    let mut base = candidate.trim_matches('_').trim_matches('-').to_string();
    if base.is_empty() {
        base = "tool".to_string();
    }

    let mut i = 0usize;
    loop {
        let extra_suffix = if i > 0 { format!("_{}", i + 1) } else { String::new() };
        let max_base_len = LLM_TOOL_NAME_MAX_LEN
            .saturating_sub(hash_suffix.len() + extra_suffix.len())
            .max(1);
        let mut trimmed_base: String = base.chars().take(max_base_len).collect();
        while trimmed_base.ends_with('_') || trimmed_base.ends_with('-') {
            trimmed_base.pop();
        }
        if trimmed_base.is_empty() {
            trimmed_base = "tool".to_string();
        }
        candidate = format!("{trimmed_base}{hash_suffix}{extra_suffix}");
        if used.get(&candidate).is_none_or(|prev| prev == original) {
            used.insert(candidate.clone(), original.to_string());
            return candidate;
        }
        i += 1;
    }
}

/// 规范化工具列表供 LLM 使用，返回 `(llm_tools, llm_name -> origin_name)`。
pub fn prepare_tools_for_llm(tools: Vec<McpTool>) -> (Vec<McpTool>, HashMap<String, String>) {
    let mut used = HashMap::new();
    let mut aliases = HashMap::new();
    let mut out = Vec::with_capacity(tools.len());

    for tool in tools {
        let origin = tool.name.trim().to_string();
        if origin.is_empty() {
            continue;
        }
        let sanitized = sanitize_llm_tool_name(&origin);
        let llm_name = unique_llm_tool_name(&sanitized, &origin, &mut used);
        if llm_name != origin {
            aliases.insert(llm_name.clone(), origin.clone());
        }
        out.push(McpTool {
            name: llm_name,
            description: tool.description,
            input_schema: tool.input_schema,
        });
    }

    (out, aliases)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sanitizes_device_tool_dots() {
        assert_eq!(
            sanitize_llm_tool_name("self.get_device_status"),
            "self_get_device_status"
        );
        assert_eq!(
            sanitize_llm_tool_name("self.audio_speaker.set_volume"),
            "self_audio_speaker_set_volume"
        );
    }

    #[test]
    fn keeps_valid_local_tool_names() {
        assert_eq!(sanitize_llm_tool_name("search_knowledge"), "search_knowledge");
    }

    #[test]
    fn prepare_tools_builds_alias_map() {
        let tools = vec![
            McpTool {
                name: "search_knowledge".into(),
                description: "search".into(),
                input_schema: json!({}),
            },
            McpTool {
                name: "self.get_device_status".into(),
                description: "status".into(),
                input_schema: json!({}),
            },
        ];
        let (normalized, aliases) = prepare_tools_for_llm(tools);
        assert_eq!(normalized[0].name, "search_knowledge");
        assert_eq!(normalized[1].name, "self_get_device_status");
        assert_eq!(
            aliases.get("self_get_device_status"),
            Some(&"self.get_device_status".to_string())
        );
    }
}
