use std::sync::Arc;

use tokio::sync::mpsc;
use xiaozhi_core::{Error, Result, asr as asr_const};

use crate::traits::AsrProvider;
use crate::{aliyun, aliyun_qwen3, doubao, funasr, xunfei};

pub fn create_asr(provider: &str, config: &serde_json::Value) -> Result<Arc<dyn AsrProvider>> {
    let effective = config
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or(provider);

    match effective {
        asr_const::FUNASR => Ok(Arc::new(funasr::FunasrProvider::from_config(config)?)),
        asr_const::ALIYUN_FUNASR => Ok(Arc::new(aliyun::AliyunFunAsrProvider::from_config(
            config,
        )?)),
        asr_const::DOUBAO => Ok(Arc::new(doubao::DoubaoAsrProvider::from_config(config)?)),
        asr_const::ALIYUN_QWEN3 => Ok(Arc::new(aliyun_qwen3::AliyunQwen3AsrProvider::from_config(
            config,
        )?)),
        asr_const::XUNFEI => Ok(Arc::new(xunfei::XunfeiAsrProvider::from_config(config)?)),
        other => Err(Error::Unsupported(format!("不支持的 ASR 类型: {other}"))),
    }
}

/// 收集流式识别结果
pub async fn collect_streaming(
    provider: &dyn AsrProvider,
    audio_rx: mpsc::Receiver<Vec<f32>>,
) -> Result<String> {
    let mut result_rx = provider.streaming_recognize(audio_rx).await?;
    let mut final_text = String::new();
    while let Some(result) = result_rx.recv().await {
        if let Some(err) = result.error {
            return Err(Error::Http(err));
        }
        if result.is_final {
            final_text = result.text;
        }
    }
    Ok(final_text)
}
