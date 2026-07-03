use std::sync::Arc;

use xiaozhi_core::{Error, Result, vad as vad_const};

use crate::silero_vad::SileroVad;
use crate::ten_vad::TenVad;
use crate::traits::VadProvider;
use crate::webrtc::WebRtcVad;

pub fn create_vad(provider: &str, config: &serde_json::Value) -> Result<Box<dyn VadProvider>> {
    let effective = config
        .get("provider")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(provider);
    let effective = if effective.is_empty() {
        vad_const::WEBRTC
    } else {
        effective
    };

    match effective {
        vad_const::WEBRTC => Ok(Box::new(WebRtcVad::from_config(config)?)),
        vad_const::SILERO => Ok(Box::new(SileroVad::from_config(config)?)),
        vad_const::TEN => Ok(Box::new(TenVad::from_config(config)?)),
        other => Err(Error::Unsupported(format!("不支持的 VAD 类型: {other}"))),
    }
}

pub type SharedVad = Arc<std::sync::Mutex<Box<dyn VadProvider>>>;
