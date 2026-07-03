//! 豆包 TTS 模型与 ResourceID 解析（对齐 Golang `internal/domain/tts/doubao/model.go`）

use xiaozhi_core::{Error, Result};

pub const DEFAULT_DOUBAO_TTS_MODEL: &str = "seed-tts-1.1";
pub const RESOURCE_SEED_TTS_10: &str = "seed-tts-1.0";
pub const RESOURCE_SEED_TTS_20: &str = "seed-tts-2.0";
pub const RESOURCE_SEED_ICL_10: &str = "seed-icl-1.0";
pub const RESOURCE_SEED_ICL_20: &str = "seed-icl-2.0";
pub const MODEL_SEED_TTS_11: &str = "seed-tts-1.1";
pub const MODEL_SEED_TTS_20_STANDARD: &str = "seed-tts-2.0-standard";
pub const MODEL_SEED_TTS_20_EXPR: &str = "seed-tts-2.0-expressive";
pub const MODEL_SEED_ICL_10: &str = "seed-icl-1.0";
pub const MODEL_SEED_ICL_20_STANDARD: &str = "seed-icl-2.0-standard";
pub const MODEL_SEED_ICL_20_EXPR: &str = "seed-icl-2.0-expressive";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTtsModel {
    pub config_model: String,
    pub request_model: String,
    pub resource_id: String,
    pub is_clone_voice: bool,
    pub voice_family: String,
}

pub fn normalize_doubao_voice(voice: &str) -> String {
    let voice = voice.trim();
    if voice.is_empty() || voice.eq_ignore_ascii_case("default") {
        String::new()
    } else {
        voice.to_string()
    }
}

pub fn normalize_doubao_model(model: &str) -> String {
    let model = model.trim().to_ascii_lowercase();
    match model.as_str() {
        "" | "default" => String::new(),
        m if m == MODEL_SEED_TTS_11 => MODEL_SEED_TTS_11.to_string(),
        m if m == MODEL_SEED_TTS_20_STANDARD || m == "seed-tts-2.0" => {
            MODEL_SEED_TTS_20_STANDARD.to_string()
        }
        m if m == MODEL_SEED_TTS_20_EXPR => MODEL_SEED_TTS_20_EXPR.to_string(),
        m if m == MODEL_SEED_ICL_10 => MODEL_SEED_ICL_10.to_string(),
        m if m == MODEL_SEED_ICL_20_STANDARD => MODEL_SEED_ICL_20_STANDARD.to_string(),
        m if m == MODEL_SEED_ICL_20_EXPR => MODEL_SEED_ICL_20_EXPR.to_string(),
        other => other.to_string(),
    }
}

fn infer_doubao_voice_family(voice: &str) -> &'static str {
    let voice = voice.trim().to_ascii_lowercase();
    if voice.is_empty() {
        return "unknown";
    }
    if voice.starts_with("saturn_") {
        return "tts2";
    }
    if voice.starts_with("s_") || voice.starts_with("icl_") {
        return "icl1";
    }
    if voice.contains("_bigtts") {
        return "tts2";
    }
    "tts1"
}

pub fn is_doubao_clone_voice(voice: &str) -> bool {
    matches!(infer_doubao_voice_family(voice), "icl1" | "icl2")
}

pub fn resolve_doubao_tts_model(model: &str, voice: &str) -> Result<ResolvedTtsModel> {
    let voice = normalize_doubao_voice(voice);
    if voice.is_empty() {
        return Err(Error::Config(
            "豆包 TTS 缺少有效 voice，请在配置中选择火山控制台已授权的音色".into(),
        ));
    }

    let voice_family = infer_doubao_voice_family(&voice);
    let is_clone = matches!(voice_family, "icl1" | "icl2");
    let mut normalized = normalize_doubao_model(model);
    if voice_family == "tts2" && normalized == MODEL_SEED_TTS_11 {
        normalized = MODEL_SEED_TTS_20_STANDARD.to_string();
    }

    if normalized.is_empty() {
        return Ok(match voice_family {
            "icl2" => ResolvedTtsModel {
                config_model: MODEL_SEED_ICL_20_EXPR.to_string(),
                request_model: MODEL_SEED_TTS_20_EXPR.to_string(),
                resource_id: RESOURCE_SEED_ICL_20.to_string(),
                is_clone_voice: true,
                voice_family: voice_family.to_string(),
            },
            "icl1" => ResolvedTtsModel {
                config_model: MODEL_SEED_ICL_10.to_string(),
                request_model: String::new(),
                resource_id: RESOURCE_SEED_ICL_10.to_string(),
                is_clone_voice: true,
                voice_family: voice_family.to_string(),
            },
            "tts2" => ResolvedTtsModel {
                config_model: MODEL_SEED_TTS_20_STANDARD.to_string(),
                request_model: String::new(),
                resource_id: RESOURCE_SEED_TTS_20.to_string(),
                is_clone_voice: false,
                voice_family: voice_family.to_string(),
            },
            _ => ResolvedTtsModel {
                config_model: DEFAULT_DOUBAO_TTS_MODEL.to_string(),
                request_model: MODEL_SEED_TTS_11.to_string(),
                resource_id: RESOURCE_SEED_TTS_10.to_string(),
                is_clone_voice: false,
                voice_family: voice_family.to_string(),
            },
        });
    }

    let mut resolved = ResolvedTtsModel {
        config_model: normalized.clone(),
        request_model: String::new(),
        resource_id: String::new(),
        is_clone_voice: is_clone,
        voice_family: voice_family.to_string(),
    };

    match normalized.as_str() {
        MODEL_SEED_TTS_11 => {
            if voice_family == "tts2" {
                resolved.resource_id = RESOURCE_SEED_TTS_20.to_string();
            } else {
                resolved.request_model = MODEL_SEED_TTS_11.to_string();
                resolved.resource_id = RESOURCE_SEED_TTS_10.to_string();
            }
        }
        MODEL_SEED_TTS_20_STANDARD => {
            if voice_family != "tts2" {
                resolved.request_model = MODEL_SEED_TTS_20_STANDARD.to_string();
            }
            resolved.resource_id = RESOURCE_SEED_TTS_20.to_string();
        }
        MODEL_SEED_TTS_20_EXPR => {
            if voice_family != "tts2" {
                resolved.request_model = MODEL_SEED_TTS_20_EXPR.to_string();
            }
            resolved.resource_id = RESOURCE_SEED_TTS_20.to_string();
        }
        MODEL_SEED_ICL_10 => {
            resolved.resource_id = RESOURCE_SEED_ICL_10.to_string();
        }
        MODEL_SEED_ICL_20_STANDARD => {
            resolved.request_model = MODEL_SEED_TTS_20_STANDARD.to_string();
            resolved.resource_id = RESOURCE_SEED_ICL_20.to_string();
        }
        MODEL_SEED_ICL_20_EXPR => {
            resolved.request_model = MODEL_SEED_TTS_20_EXPR.to_string();
            resolved.resource_id = RESOURCE_SEED_ICL_20.to_string();
        }
        other => {
            return Err(Error::Config(format!("不支持的豆包 TTS 模型: {other}")));
        }
    }

    if voice_family == "icl1" && resolved.resource_id != RESOURCE_SEED_ICL_10 {
        return Err(Error::Config(
            "豆包复刻 1.0 音色需要匹配 seed-icl-1.0 模型族".into(),
        ));
    }
    if voice_family == "icl2" && resolved.resource_id != RESOURCE_SEED_ICL_20 {
        return Err(Error::Config(
            "豆包复刻 2.0 音色需要匹配 seed-icl-2.0 模型族".into(),
        ));
    }
    if voice_family == "tts1" && resolved.resource_id.starts_with("seed-icl-") {
        return Err(Error::Config(
            "豆包公版音色不能使用 ICL 复刻模型族".into(),
        ));
    }
    if voice_family == "tts1" && resolved.resource_id == RESOURCE_SEED_TTS_20 {
        return Err(Error::Config(
            "豆包 1.0 公版音色需要匹配 seed-tts-1.0 模型族".into(),
        ));
    }

    Ok(resolved)
}

fn build_public_fallback_model(resolved: &ResolvedTtsModel, voice: &str) -> Option<ResolvedTtsModel> {
    if is_doubao_clone_voice(voice) {
        return None;
    }
    match resolved.resource_id.as_str() {
        RESOURCE_SEED_TTS_10 => Some(ResolvedTtsModel {
            config_model: MODEL_SEED_TTS_20_STANDARD.to_string(),
            request_model: MODEL_SEED_TTS_20_STANDARD.to_string(),
            resource_id: RESOURCE_SEED_TTS_20.to_string(),
            is_clone_voice: false,
            voice_family: resolved.voice_family.clone(),
        }),
        RESOURCE_SEED_TTS_20 => Some(ResolvedTtsModel {
            config_model: MODEL_SEED_TTS_11.to_string(),
            request_model: MODEL_SEED_TTS_11.to_string(),
            resource_id: RESOURCE_SEED_TTS_10.to_string(),
            is_clone_voice: false,
            voice_family: resolved.voice_family.clone(),
        }),
        _ => None,
    }
}

pub fn build_doubao_ws_attempt_models(
    derived: &ResolvedTtsModel,
    explicit_resource_id: &str,
    voice: &str,
) -> Vec<ResolvedTtsModel> {
    let mut models = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let mut push = |candidate: ResolvedTtsModel| {
        let key = format!("{}|{}", candidate.resource_id, candidate.request_model);
        if seen.insert(key) {
            models.push(candidate);
        }
    };

    let explicit = explicit_resource_id.trim();
    if !explicit.is_empty() {
        let mut override_model = derived.clone();
        override_model.resource_id = explicit.to_string();
        push(override_model);
    }
    push(derived.clone());
    if let Some(fallback) = build_public_fallback_model(derived, voice) {
        push(fallback);
    }
    models
}

pub fn is_doubao_retryable_resource_error(err: &str) -> bool {
    let err = err.to_ascii_lowercase();
    err.contains("resource id is mismatched")
        || err.contains("requested resource not granted")
        || err.contains("resource_id")
            && (err.contains("not granted") || err.contains("mismatched"))
}

pub fn effective_resource_id(cfg_resource_id: &str, resolved: &ResolvedTtsModel) -> String {
    let explicit = cfg_resource_id.trim();
    if explicit.is_empty() {
        resolved.resource_id.clone()
    } else {
        explicit.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_tts_11_maps_to_seed_tts_10_resource() {
        let resolved = resolve_doubao_tts_model(MODEL_SEED_TTS_11, "BV001_streaming").unwrap();
        assert_eq!(resolved.resource_id, RESOURCE_SEED_TTS_10);
        assert_eq!(resolved.request_model, MODEL_SEED_TTS_11);
    }

    #[test]
    fn tts2_voice_upgrades_model_family() {
        let resolved =
            resolve_doubao_tts_model(MODEL_SEED_TTS_11, "zh_female_vv_uranus_bigtts").unwrap();
        assert_eq!(resolved.resource_id, RESOURCE_SEED_TTS_20);
    }

    #[test]
    fn default_voice_is_rejected() {
        assert!(resolve_doubao_tts_model(MODEL_SEED_TTS_11, "default").is_err());
    }
}
