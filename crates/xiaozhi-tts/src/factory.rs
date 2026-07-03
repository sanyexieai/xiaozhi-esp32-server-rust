use std::sync::Arc;

use xiaozhi_core::{Error, Result, tts as tts_const};

use crate::traits::TtsProvider;
use crate::{edge, minimax, openai, providers, zhipu};

pub fn create_tts(provider: &str, config: &serde_json::Value) -> Result<Arc<dyn TtsProvider>> {
    let effective = config
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or(provider);

    // 资源池 key 格式 "provider:voiceID"
    let effective = effective.split(':').next().unwrap_or(effective);

    match effective {
        tts_const::EDGE => Ok(Arc::new(edge::EdgeTtsProvider::from_config(config)?)),
        tts_const::OPENAI => Ok(Arc::new(openai::OpenAiTtsProvider::from_config(config)?)),
        tts_const::DOUBAO => Ok(Arc::new(providers::DoubaoTtsProvider::from_config(config)?)),
        tts_const::DOUBAO_WS => Ok(Arc::new(providers::DoubaoWsTtsProvider::from_config(config)?)),
        tts_const::COSYVOICE => Ok(Arc::new(providers::CosyVoiceTtsProvider::from_config(config)?)),
        tts_const::EDGE_OFFLINE => {
            Ok(Arc::new(providers::EdgeOfflineTtsProvider::from_config(config)?))
        }
        tts_const::XIAOZHI => Ok(Arc::new(providers::XiaozhiTtsProvider::from_config(config)?)),
        tts_const::XUNFEI => Ok(Arc::new(providers::XunfeiTtsProvider::from_config(config)?)),
        tts_const::XUNFEI_SUPER => {
            Ok(Arc::new(providers::XunfeiSuperTtsProvider::from_config(config)?))
        }
        tts_const::ZHIPU => Ok(Arc::new(zhipu::ZhipuTtsProvider::from_config(config)?)),
        tts_const::MINIMAX => Ok(Arc::new(minimax::MinimaxTtsProvider::from_config(config)?)),
        tts_const::ALIYUN_QWEN => Ok(Arc::new(providers::QwenTtsProvider::from_config(config)?)),
        tts_const::INDEXTTS_VLLM => {
            Ok(Arc::new(openai::OpenAiTtsProvider::from_index_config(config)?))
        }
        other => Err(Error::Unsupported(format!("不支持的 TTS 类型: {other}"))),
    }
}
