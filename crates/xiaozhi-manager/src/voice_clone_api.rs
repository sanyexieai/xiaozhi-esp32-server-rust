//! 声音复刻 API 响应格式（对齐 Go / 前端 `VoiceClones.vue`）

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::db::{VoiceCloneRow, VoiceCloneTaskRow};

pub fn is_clone_active_status(status: &str) -> bool {
    matches!(
        status.trim().to_lowercase().as_str(),
        "active" | "ready" | "succeeded"
    )
}

pub fn normalize_clone_api_status(status: &str) -> &'static str {
    match status.trim().to_lowercase().as_str() {
        "active" | "ready" | "succeeded" => "active",
        "processing" | "queued" => "processing",
        "failed" => "failed",
        other if other.is_empty() => "unknown",
        _ => "processing",
    }
}

pub fn voice_clone_to_api_json(
    clone: &VoiceCloneRow,
    tts_config_name: Option<&str>,
    tts_provider: Option<&str>,
    task: Option<&VoiceCloneTaskRow>,
) -> Value {
    let provider_voice_id = clone
        .voice_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("");
    let api_status = normalize_clone_api_status(&clone.status);
    let task_status = task
        .map(|t| t.status.as_str())
        .unwrap_or_else(|| match clone.status.trim().to_lowercase().as_str() {
            "active" | "ready" | "succeeded" => "succeeded",
            "failed" => "failed",
            "queued" => "queued",
            "processing" => "processing",
            _ => "processing",
        });
    let mut item = json!({
        "id": clone.id,
        "user_id": clone.user_id,
        "name": clone.name,
        "provider": clone.provider,
        "provider_voice_id": provider_voice_id,
        "tts_config_id": clone.tts_config_id,
        "tts_config_name": tts_config_name.unwrap_or(&clone.tts_config_id),
        "shared_to_all": clone.shared_to_all,
        "status": api_status,
        "task_status": task_status,
        "transcript": clone.transcript,
        "error_message": clone.error_message,
        "created_at": clone.created_at,
        "updated_at": clone.updated_at,
    });
    if let Some(task) = task {
        item["task_id"] = json!(task.task_id);
        item["task_attempts"] = json!(task.attempts);
        if !task.last_error.trim().is_empty() {
            item["task_last_error"] = json!(task.last_error);
        }
        item["task_started_at"] = task.started_at.clone().map(Value::String).unwrap_or(Value::Null);
        item["task_finished_at"] = task
            .finished_at
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null);
    } else if let Some(err) = clone.error_message.as_deref().filter(|s| !s.trim().is_empty()) {
        item["task_last_error"] = json!(err);
    }
    if api_status == "failed" {
        item["last_error"] = clone
            .error_message
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null);
    }
    if let Some(provider) = tts_provider.filter(|s| !s.trim().is_empty()) {
        item["tts_provider"] = json!(provider);
    }
    item
}

pub fn voice_clones_to_api_list(
    clones: &[VoiceCloneRow],
    tts_configs: &[(String, String, String)],
    tasks: &HashMap<i64, VoiceCloneTaskRow>,
) -> Vec<Value> {
    clones
        .iter()
        .map(|clone| {
            let (name, provider) = tts_configs
                .iter()
                .find(|(id, _, _)| id == &clone.tts_config_id)
                .map(|(_, n, p)| (Some(n.as_str()), Some(p.as_str())))
                .unwrap_or((None, None));
            voice_clone_to_api_json(clone, name, provider, tasks.get(&clone.id))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_ready_to_active() {
        assert_eq!(normalize_clone_api_status("ready"), "active");
        assert!(is_clone_active_status("ready"));
    }
}
