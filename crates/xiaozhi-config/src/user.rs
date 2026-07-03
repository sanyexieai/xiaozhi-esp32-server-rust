use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderConfig {
    pub provider: String,
    #[serde(default)]
    pub config: HashMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SpeakerGroupInfo {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub uuids: Vec<String>,
    pub tts_config_id: Option<String>,
    pub voice: Option<String>,
    pub voice_model_override: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KnowledgeBaseRef {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub provider: String,
    pub external_kb_id: String,
    #[serde(default)]
    pub external_doc_id: String,
    pub retrieval_threshold: Option<f64>,
    #[serde(default)]
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpenClawConfig {
    #[serde(default)]
    pub allowed: bool,
    #[serde(default)]
    pub enter_keywords: Vec<String>,
    #[serde(default)]
    pub exit_keywords: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UConfig {
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub asr: ProviderConfig,
    #[serde(default)]
    pub tts: ProviderConfig,
    #[serde(default)]
    pub llm: ProviderConfig,
    #[serde(default)]
    pub vad: ProviderConfig,
    #[serde(default)]
    pub memory: ProviderConfig,
    #[serde(default)]
    pub voice_identify: HashMap<String, SpeakerGroupInfo>,
    /// 声纹组引用的额外 TTS 配置（key = config_id）
    #[serde(default)]
    pub tts_configs: HashMap<String, ProviderConfig>,
    #[serde(default)]
    pub memory_mode: String,
    #[serde(default)]
    pub speaker_chat_mode: String,
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub mcp_service_names: String,
    #[serde(default)]
    pub openclaw: OpenClawConfig,
    #[serde(default)]
    pub knowledge_bases: Vec<KnowledgeBaseRef>,
}

impl UConfig {
    pub fn from_system_defaults(system_prompt: &str, vad: &str, asr: &str, tts: &str, llm: &str) -> Self {
        Self {
            system_prompt: system_prompt.to_string(),
            asr: ProviderConfig {
                provider: asr.to_string(),
                config: HashMap::new(),
            },
            tts: ProviderConfig {
                provider: tts.to_string(),
                config: HashMap::new(),
            },
            llm: ProviderConfig {
                provider: llm.to_string(),
                config: HashMap::new(),
            },
            vad: ProviderConfig {
                provider: vad.to_string(),
                config: HashMap::new(),
            },
            memory: ProviderConfig {
                provider: "nomemo".to_string(),
                config: HashMap::new(),
            },
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ActivationPayload {
    #[serde(default)]
    pub algorithm: String,
    #[serde(default)]
    pub serial_number: String,
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub challenge: String,
    #[serde(default)]
    pub hmac: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KnowledgeSearchHit {
    pub content: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub score: f64,
}
