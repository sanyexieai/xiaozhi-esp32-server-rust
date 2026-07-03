use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Debug, Clone, Deserialize)]
struct VoiceOptionRaw {
    value: String,
    label: String,
}

#[derive(Debug, Clone)]
pub struct VoiceOption {
    pub value: String,
    pub label: String,
}

impl VoiceOption {
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "value": self.value,
            "label": self.label,
        })
    }
}

fn voice_maps() -> &'static HashMap<String, Vec<VoiceOption>> {
    static MAP: OnceLock<HashMap<String, Vec<VoiceOption>>> = OnceLock::new();
    MAP.get_or_init(|| {
        let raw: HashMap<String, Vec<VoiceOptionRaw>> =
            serde_json::from_str(include_str!("../data/voice_options.json"))
                .expect("voice_options.json");
        raw.into_iter()
            .map(|(provider, items)| {
                let voices = items
                    .into_iter()
                    .map(|v| VoiceOption {
                        value: v.value,
                        label: v.label,
                    })
                    .collect();
                (provider, voices)
            })
            .collect()
    })
}

fn qwen_model_maps() -> &'static HashMap<String, Vec<VoiceOption>> {
    static MAP: OnceLock<HashMap<String, Vec<VoiceOption>>> = OnceLock::new();
    MAP.get_or_init(|| {
        let raw: HashMap<String, Vec<VoiceOptionRaw>> =
            serde_json::from_str(include_str!("../data/qwen_voices_by_model.json"))
                .expect("qwen_voices_by_model.json");
        raw.into_iter()
            .map(|(model, items)| {
                let voices = items
                    .into_iter()
                    .map(|v| VoiceOption {
                        value: v.value,
                        label: v.label,
                    })
                    .collect();
                (model, voices)
            })
            .collect()
    })
}

pub fn get_voice_options_by_provider(provider: &str) -> Vec<VoiceOption> {
    voice_maps()
        .get(provider)
        .cloned()
        .unwrap_or_default()
}

fn normalize_qwen_model(model: &str) -> String {
    let model = model.trim();
    if model.is_empty() {
        return String::new();
    }
    if model.starts_with("qwen3-tts-flash") {
        return "qwen3-tts-flash".to_string();
    }
    if model.starts_with("qwen-tts") {
        return "qwen-tts".to_string();
    }
    model.to_string()
}

pub fn get_aliyun_qwen_voices_by_model(model: &str) -> Vec<VoiceOption> {
    let key = normalize_qwen_model(model);
    if key.is_empty() {
        return get_voice_options_by_provider("aliyun_qwen");
    }
    qwen_model_maps()
        .get(&key)
        .cloned()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| get_voice_options_by_provider("aliyun_qwen"))
}
