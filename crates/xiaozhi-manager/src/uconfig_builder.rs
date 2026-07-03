use std::collections::HashMap;

use serde_json::Value;
use xiaozhi_config::user::{KnowledgeBaseRef, OpenClawConfig, ProviderConfig, SpeakerGroupInfo, UConfig};
use xiaozhi_config::AppConfig;
use xiaozhi_llm::normalize_llm_provider;

use crate::db::{AgentRow, Database, DeviceRow, RoleRow};

fn value_to_map(v: Value) -> HashMap<String, Value> {
    match v {
        Value::Object(m) => m.into_iter().collect(),
        _ => HashMap::new(),
    }
}

fn merge_maps(base: &mut HashMap<String, Value>, overlay: HashMap<String, Value>) {
    for (k, v) in overlay {
        base.insert(k, v);
    }
}

fn yaml_provider_map(app: &AppConfig, kind: &str, config_id: &str) -> HashMap<String, Value> {
    let providers = match kind {
        "llm" => &app.llm.providers,
        "asr" => &app.asr.providers,
        "tts" => &app.tts.providers,
        "vad" => &app.vad.providers,
        "memory" => &app.memory.providers,
        _ => return HashMap::new(),
    };
    providers
        .get(config_id)
        .cloned()
        .map(value_to_map)
        .unwrap_or_default()
}

fn is_known_provider(kind: &str, value: &str) -> bool {
    let v = value.trim().to_lowercase();
    match kind {
        "vad" => matches!(v.as_str(), "ten_vad" | "webrtc_vad" | "silero_vad" | "webrtc"),
        "asr" => matches!(
            v.as_str(),
            "funasr" | "aliyun_funasr" | "doubao" | "aliyun_qwen3" | "xunfei"
        ),
        "tts" => matches!(
            v.as_str(),
            "doubao"
                | "doubao_ws"
                | "cosyvoice"
                | "edge"
                | "edge_offline"
                | "xiaozhi"
                | "xunfei"
                | "xunfei_super_tts"
                | "openai"
                | "zhipu"
                | "minimax"
                | "aliyun_qwen"
                | "indextts_vllm"
        ),
        "memory" => matches!(v.as_str(), "nomemo" | "memobase" | "mem0" | "memos"),
        _ => false,
    }
}

fn infer_provider_from_config(kind: &str, config: &Value) -> Option<String> {
    let obj = config.as_object()?;
    match kind {
        "vad" => {
            if obj.contains_key("hop_size") {
                return Some("ten_vad".into());
            }
            if obj.contains_key("model_path") || obj.contains_key("min_silence_duration_ms") {
                return Some("silero_vad".into());
            }
            if obj.contains_key("vad_mode")
                || obj.contains_key("vad_sample_rate")
                || obj.contains_key("pool_min_size")
            {
                return Some("webrtc_vad".into());
            }
        }
        "asr" => {
            let model = obj.get("model").and_then(|v| v.as_str()).unwrap_or("");
            let ws_url = obj.get("ws_url").and_then(|v| v.as_str()).unwrap_or("");
            if obj.contains_key("appid") && obj.contains_key("api_secret") {
                return Some("xunfei".into());
            }
            if model.contains("qwen3-asr") || ws_url.contains("/realtime") {
                return Some("aliyun_qwen3".into());
            }
            if model.contains("fun-asr") || ws_url.contains("/inference") {
                return Some("aliyun_funasr".into());
            }
            if obj.contains_key("access_token")
                && obj.contains_key("resource_id")
                && obj.contains_key("end_window_size")
            {
                return Some("doubao".into());
            }
            if obj.contains_key("host") && obj.contains_key("port") {
                return Some("funasr".into());
            }
        }
        "tts" => {
            let ws_url = obj
                .get("ws_url")
                .or_else(|| obj.get("api_url"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if obj.contains_key("spk_id") {
                return Some("cosyvoice".into());
            }
            if obj.contains_key("server_url") {
                return Some("edge_offline".into());
            }
            if ws_url.contains("openspeech.bytedance.com") {
                return Some("doubao_ws".into());
            }
            if ws_url.contains("dashscope.aliyuncs.com") {
                return Some("aliyun_qwen".into());
            }
        }
        _ => {}
    }
    None
}

fn normalize_engine_provider(
    kind: &str,
    config_id: &str,
    stored_provider: &str,
    config: &Value,
) -> String {
    for candidate in [
        stored_provider,
        config
            .get("provider")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        config_id,
    ] {
        if is_known_provider(kind, candidate) {
            return candidate.trim().to_lowercase();
        }
    }
    infer_provider_from_config(kind, config).unwrap_or_else(|| {
        match kind {
            "vad" => "ten_vad".to_string(),
            "asr" => "funasr".to_string(),
            "tts" => "edge".to_string(),
            "memory" => "nomemo".to_string(),
            _ => stored_provider.to_string(),
        }
    })
}

fn resolve_provider(
    db: &Database,
    app: &AppConfig,
    kind: &str,
    config_id: &str,
    override_json: &str,
) -> ProviderConfig {
    let mut provider = config_id.to_string();
    let mut config = HashMap::new();

    if !config_id.is_empty() {
        if let Ok(Some(row)) = db.find_config_by_type_and_id(kind, config_id) {
            provider = if row.provider.is_empty() {
                row.config_id.clone()
            } else {
                row.provider.clone()
            };
            if let Ok(v) = serde_json::from_str::<Value>(&row.json_data) {
                config = value_to_map(v);
            }
        } else {
            config = yaml_provider_map(app, kind, config_id);
        }
    } else if let Ok(rows) = db.list_configs(kind) {
        if let Some(default) = rows.iter().find(|r| r.is_default && r.enabled) {
            provider = default.config_id.clone();
            if let Ok(v) = serde_json::from_str::<Value>(&default.json_data) {
                config = value_to_map(v);
            }
        }
    }

    if provider.is_empty() {
        provider = match kind {
            "llm" => app.llm.provider.clone(),
            "asr" => app.asr.provider.clone(),
            "tts" => app.tts.provider.clone(),
            "vad" => app.vad.provider.clone(),
            "memory" => app.memory.provider.clone(),
            _ => String::new(),
        };
        if config.is_empty() && !provider.is_empty() {
            config = yaml_provider_map(app, kind, &provider);
        }
    }

    if let Ok(v) = serde_json::from_str::<Value>(override_json) {
        merge_maps(&mut config, value_to_map(v));
    }

    let cfg_value = Value::Object(
        config
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    );
    if kind == "llm" {
        provider = normalize_llm_provider(config_id, &provider, &cfg_value);
    } else {
        provider = normalize_engine_provider(kind, config_id, &provider, &cfg_value);
    }
    config.insert("provider".into(), Value::String(provider.clone()));

    ProviderConfig { provider, config }
}

fn parse_agent_extra(agent: &AgentRow) -> Value {
    serde_json::from_str(&agent.extra_json).unwrap_or(Value::Object(Default::default()))
}

fn parse_openclaw(extra: &Value) -> OpenClawConfig {
    let raw = extra
        .get("openclaw")
        .or_else(|| extra.get("openclaw_config"))
        .cloned()
        .unwrap_or(Value::Null);
    if raw.is_string() {
        return serde_json::from_str(raw.as_str().unwrap_or("{}"))
            .unwrap_or_default();
    }
    serde_json::from_value(raw).unwrap_or_default()
}

fn knowledge_refs(db: &Database, user_id: i64, extra: &Value) -> Vec<KnowledgeBaseRef> {
    let ids: Vec<i64> = extra
        .get("knowledge_base_ids")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    if ids.is_empty() {
        return Vec::new();
    }
    let Ok(bases) = db.list_knowledge_bases(user_id) else {
        return Vec::new();
    };
    ids.iter()
        .filter_map(|id| bases.iter().find(|b| b.id == *id))
        .map(|kb| {
            let external_kb_id = external_kb_id_from_config(db, kb);
            KnowledgeBaseRef {
                id: kb.id as u64,
                name: kb.name.clone(),
                description: kb.description.clone(),
                provider: kb.provider.clone(),
                external_kb_id,
                external_doc_id: String::new(),
                retrieval_threshold: Some(0.5),
                status: kb.status.clone(),
            }
        })
        .collect()
}

fn external_kb_id_from_config(db: &Database, kb: &crate::db::KnowledgeBaseRow) -> String {
    if let Ok(Some(detail)) = db.get_knowledge_base(kb.id) {
        return knowledge_external_id(&detail.config_json, kb.id);
    }
    kb.id.to_string()
}

fn knowledge_external_id(config_json: &str, local_id: i64) -> String {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(config_json) else {
        return local_id.to_string();
    };
    for key in [
        "external_kb_id",
        "dataset_id",
        "knowledge_base_id",
        "kb_id",
    ] {
        if let Some(s) = v.get(key).and_then(|x| x.as_str()) {
            let s = s.trim();
            if !s.is_empty() {
                return s.to_string();
            }
        }
    }
    local_id.to_string()
}

fn collect_speaker_tts_configs(
    db: &Database,
    app: &AppConfig,
    voice_identify: &HashMap<String, SpeakerGroupInfo>,
) -> HashMap<String, ProviderConfig> {
    use std::collections::HashSet;
    let mut ids = HashSet::new();
    for g in voice_identify.values() {
        if let Some(ref id) = g.tts_config_id {
            if !id.is_empty() {
                ids.insert(id.clone());
            }
        }
    }
    let mut out = HashMap::new();
    for id in ids {
        out.insert(id.clone(), resolve_provider(db, app, "tts", &id, "{}"));
    }
    out
}

fn speaker_map(db: &Database, user_id: i64, agent_id: i64) -> HashMap<String, SpeakerGroupInfo> {
    let mut out = HashMap::new();
    let Ok(groups) = db.list_speaker_groups(user_id, Some(agent_id)) else {
        return out;
    };
    for g in groups {
        let Ok(samples) = db.list_speaker_samples(g.id) else {
            continue;
        };
        let uuids: Vec<String> = samples
            .iter()
            .map(|s| s.id.to_string())
            .collect();
        out.insert(
            g.name.clone(),
            SpeakerGroupInfo {
                id: g.id as u64,
                name: g.name,
                prompt: g.prompt,
                description: g.description,
                uuids,
                tts_config_id: g.tts_config_id,
                voice: g.voice,
                voice_model_override: None,
            },
        );
    }
    out
}

pub fn build_device_uconfig(
    db: &Database,
    app: &AppConfig,
    device: &DeviceRow,
    agent: Option<&AgentRow>,
    role: Option<&RoleRow>,
) -> UConfig {
    let sys = app.clone();
    let mut system_prompt = sys.system_prompt.clone();
    let mut llm_id = sys.llm.provider.clone();
    let mut tts_id = sys.tts.provider.clone();
    let mut asr_id = sys.asr.provider.clone();
    let mut vad_id = sys.vad.provider.clone();
    let mut llm_override = "{}".to_string();
    let mut tts_override = "{}".to_string();
    let mut asr_override = "{}".to_string();
    let mut vad_override = "{}".to_string();
    let mut memory_mode = "short".to_string();
    let mut speaker_chat_mode = "off".to_string();
    let mut mcp_service_names = String::new();
    let mut openclaw = OpenClawConfig::default();
    let mut knowledge_bases = Vec::new();
    let mut voice_identify = HashMap::new();
    let mut agent_id_str = String::new();

    if let Some(role) = role {
        if !role.prompt.is_empty() {
            system_prompt = role.prompt.clone();
        }
        if let Some(ref id) = role.llm_config_id {
            if !id.is_empty() {
                llm_id = id.clone();
            }
        }
        if let Some(ref id) = role.tts_config_id {
            if !id.is_empty() {
                tts_id = id.clone();
            }
        }
    }

    if let Some(agent) = agent {
        agent_id_str = agent.id.to_string();
        if !agent.system_prompt.is_empty() {
            system_prompt = agent.system_prompt.clone();
        }
        if !agent.llm_provider.is_empty() {
            llm_id = agent.llm_provider.clone();
        }
        if !agent.tts_provider.is_empty() {
            tts_id = agent.tts_provider.clone();
        }
        if !agent.asr_provider.is_empty() {
            asr_id = agent.asr_provider.clone();
        }
        if !agent.vad_provider.is_empty() {
            vad_id = agent.vad_provider.clone();
        }
        llm_override = agent.llm_config.clone();
        tts_override = agent.tts_config.clone();
        asr_override = agent.asr_config.clone();

        let extra = parse_agent_extra(agent);
        memory_mode = extra
            .get("memory_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("short")
            .to_string();
        speaker_chat_mode = extra
            .get("speaker_chat_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("off")
            .to_string();
        mcp_service_names = extra
            .get("mcp_service_names")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        openclaw = parse_openclaw(&extra);
        if let Some(uid) = device.user_id {
            knowledge_bases = knowledge_refs(db, uid, &extra);
            voice_identify = speaker_map(db, uid, agent.id);
        }

        if let Some(voice) = extra.get("voice").and_then(|v| v.as_str()) {
            if !voice.is_empty() {
                let mut tts_map = serde_json::from_str::<Value>(&tts_override)
                    .unwrap_or(Value::Object(Default::default()));
                if let Value::Object(ref mut m) = tts_map {
                    m.insert("voice".into(), Value::String(voice.to_string()));
                }
                tts_override = tts_map.to_string();
            }
        }
    }

    let memory_provider = match memory_mode.as_str() {
        "none" => "nomemo".to_string(),
        "long" => app.memory.provider.clone(),
        _ => app.memory.provider.clone(),
    };

    let tts_configs = collect_speaker_tts_configs(db, app, &voice_identify);

    UConfig {
        system_prompt,
        asr: resolve_provider(db, app, "asr", &asr_id, &asr_override),
        tts: resolve_provider(db, app, "tts", &tts_id, &tts_override),
        llm: resolve_provider(db, app, "llm", &llm_id, &llm_override),
        vad: resolve_provider(db, app, "vad", &vad_id, &vad_override),
        memory: resolve_provider(db, app, "memory", &memory_provider, "{}"),
        voice_identify,
        tts_configs,
        memory_mode,
        speaker_chat_mode,
        agent_id: agent_id_str,
        mcp_service_names,
        openclaw,
        knowledge_bases,
    }
}

pub fn build_for_device_id(db: &Database, app: &AppConfig, device_id: &str) -> UConfig {
    let physical_id =
        xiaozhi_core::constants::simulator::resolve_physical_device_id(device_id);
    let Ok(Some(device)) = db.find_device_by_device_id(physical_id) else {
        return UConfig::from_app_config(app);
    };

    if device.user_id.is_none() {
        return UConfig::from_app_config(app);
    }
    let user_id = device.user_id.unwrap_or(0);

    let role = if device.role_name != "default" && !device.role_name.is_empty() {
        db.find_role_by_name(user_id, &device.role_name)
            .ok()
            .flatten()
    } else {
        None
    };

    let agent = if let Some(agent_id) = device.agent_id {
        db.get_agent_by_id(agent_id).ok().flatten()
    } else {
        None
    };

    build_device_uconfig(db, app, &device, agent.as_ref(), role.as_ref())
}
