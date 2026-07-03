use std::collections::HashMap;

use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

use crate::message::{ChatMessage, MessageRole};
use crate::traits::LLM_EXTRA_ERROR_KEY;

const DEFAULT_USER_PREFIX: &str = "xiaozhi";
const MAX_STABLE_USER_ID_LEN: usize = 64;

pub fn build_stable_user_id(prefix: &str, session_id: &str) -> String {
    let mut safe_prefix = sanitize_user_id_part(prefix);
    if safe_prefix.is_empty() {
        safe_prefix = DEFAULT_USER_PREFIX.to_string();
    }
    let mut safe_session = sanitize_user_id_part(session_id);
    if safe_session.is_empty() {
        safe_session = "anonymous".to_string();
    }
    let candidate = format!("{safe_prefix}_{safe_session}");
    if candidate.len() <= MAX_STABLE_USER_ID_LEN {
        return candidate;
    }
    let mut hasher = Sha256::new();
    hasher.update(format!("{prefix}:{session_id}").as_bytes());
    let suffix = hex::encode(&hasher.finalize()[..8]);
    let max_prefix_len = MAX_STABLE_USER_ID_LEN.saturating_sub(suffix.len() + 1);
    if max_prefix_len == 0 {
        return suffix;
    }
    if safe_prefix.len() > max_prefix_len {
        safe_prefix.truncate(max_prefix_len);
    }
    format!("{safe_prefix}_{suffix}")
}

fn sanitize_user_id_part(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(trimmed.len());
    let mut last_sep = false;
    for ch in trimmed.chars() {
        let ok = ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.';
        if ok {
            out.push(ch);
            last_sep = false;
        } else if !last_sep {
            out.push('_');
            last_sep = true;
        }
    }
    out.trim_matches(&['.', '_', '-'][..]).to_string()
}

/// Dify/Coze 会话模式：取最近一条 user 消息作为 query。
pub fn build_provider_query(dialogue: &[ChatMessage]) -> String {
    for msg in dialogue.iter().rev() {
        if msg.role == MessageRole::User {
            let text = msg.content.trim();
            if !text.is_empty() {
                return text.to_string();
            }
        }
    }
    dialogue
        .iter()
        .rev()
        .find_map(|m| {
            let text = m.content.trim();
            if text.is_empty() {
                None
            } else {
                Some(text.to_string())
            }
        })
        .unwrap_or_default()
}

pub fn normalize_api_token(token: &str) -> String {
    let token = token.trim();
    if token.len() >= 7 && token[..7].eq_ignore_ascii_case("bearer ") {
        return token[7..].trim().to_string();
    }
    token.to_string()
}

pub fn send_llm_error(tx: &mpsc::Sender<ChatMessage>, err: impl Into<String>) {
    let mut msg = ChatMessage::system("");
    msg.extra
        .insert(LLM_EXTRA_ERROR_KEY.into(), json!(err.into()));
    let _ = tx.try_send(msg);
}

pub type ConversationMap = std::sync::Mutex<HashMap<String, String>>;

pub fn get_conversation_id(map: &ConversationMap, session_id: &str) -> String {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return String::new();
    }
    map.lock()
        .ok()
        .and_then(|g| g.get(session_id).cloned())
        .unwrap_or_default()
}

pub fn set_conversation_id(map: &ConversationMap, session_id: &str, conversation_id: &str) {
    let session_id = session_id.trim();
    let conversation_id = conversation_id.trim();
    if session_id.is_empty() || conversation_id.is_empty() {
        return;
    }
    if let Ok(mut guard) = map.lock() {
        guard.insert(session_id.to_string(), conversation_id.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_stable_user_id() {
        let id = build_stable_user_id("xiaozhi", "session-1");
        assert!(id.starts_with("xiaozhi_"));
        assert!(id.len() <= MAX_STABLE_USER_ID_LEN);
    }

    #[test]
    fn picks_last_user_query() {
        let dialogue = vec![
            ChatMessage::user("hello"),
            ChatMessage::assistant("hi"),
            ChatMessage::user("  world  "),
        ];
        assert_eq!(build_provider_query(&dialogue), "world");
    }
}
