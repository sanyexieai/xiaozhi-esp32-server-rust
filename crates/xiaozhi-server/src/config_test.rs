//! 配置连通性测试（与 Go `configtest.go` 对齐）

use std::f64::consts::PI;
use std::time::{Duration, Instant};

use serde_json::Value;
use xiaozhi_asr::create_asr;
use xiaozhi_llm::{create_llm, ChatMessage};
use xiaozhi_memory::create_memory;
use xiaozhi_tts::create_tts;
use xiaozhi_vad::create_vad;

const DEFAULT_TEST_TEXT: &str = "配置测试";
const DEFAULT_LLM_TIMEOUT: Duration = Duration::from_secs(15);
const THINKING_LLM_TIMEOUT: Duration = Duration::from_secs(30);
const ASR_TEST_TIMEOUT: Duration = Duration::from_secs(15);
const TTS_TEST_TIMEOUT: Duration = Duration::from_secs(15);
const MEMORY_TEST_TIMEOUT: Duration = Duration::from_secs(10);
const MEMORY_TEST_AGENT_ID: &str = "xiaozhi_config_test";

#[derive(Debug, Clone)]
pub struct ConfigTestResult {
    pub ok: bool,
    pub message: String,
    pub first_packet_ms: Option<u64>,
}

impl ConfigTestResult {
    fn pass(ms: u64) -> Self {
        Self {
            ok: true,
            message: "通过".into(),
            first_packet_ms: Some(ms),
        }
    }

    fn fail(message: impl Into<String>, ms: Option<u64>) -> Self {
        Self {
            ok: false,
            message: message.into(),
            first_packet_ms: ms,
        }
    }
}

pub async fn run_config_test(kind: &str, provider: &str, config: &Value) -> ConfigTestResult {
    let pcm = fallback_pcm();
    match kind {
        "vad" => test_vad(provider, config, &pcm),
        "asr" => test_asr(provider, config, pcm).await,
        "llm" => test_llm(provider, config).await,
        "tts" => test_tts(provider, config).await,
        "memory" => test_memory(provider, config).await,
        other => ConfigTestResult::fail(format!("不支持的测试类型: {other}"), None),
    }
}

fn fallback_pcm() -> Vec<f32> {
    let mut pcm = vec![0.0f32; 16000];
    for (i, sample) in pcm.iter_mut().enumerate() {
        let t = i as f64 / 16000.0;
        let mut s = 0.5 * (2.0 * PI * t * 400.0).sin();
        s += 0.25 * (2.0 * PI * t * 800.0).sin();
        s += 0.15 * (2.0 * PI * t * 1200.0).sin();
        s += 0.1 * (2.0 * PI * t * 2000.0).sin();
        s += (i % 100) as f64 / 2000.0 - 0.025;
        let env = if i < 1000 {
            i as f64 / 1000.0
        } else if i > 15000 {
            (16000 - i) as f64 / 1000.0
        } else {
            1.0
        };
        *sample = (s * env) as f32;
    }
    pcm
}

fn float_pcm_to_i16(pcm: &[f32]) -> Vec<i16> {
    pcm.iter()
        .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
        .collect()
}

fn vad_provider_for_test(provider: &str, config: &Value) -> String {
    if let Some(p) = config.get("provider").and_then(|v| v.as_str()) {
        let p = p.trim();
        if !p.is_empty() {
            return p.to_string();
        }
    }
    if config.get("silero_vad").is_some() {
        return "silero_vad".to_string();
    }
    if config.get("ten_vad").is_some() {
        return "ten_vad".to_string();
    }
    let p = provider.trim();
    if !p.is_empty() {
        return p.to_string();
    }
    "webrtc".to_string()
}

fn vad_test_sample_count(provider: &str, config: &Value) -> usize {
    match vad_provider_for_test(provider, config).as_str() {
        "silero_vad" => {
            if int_from_config(config, "sample_rate", 16000) == 8000 {
                256
            } else {
                512
            }
        }
        "ten_vad" => int_from_config(config, "hop_size", 512),
        _ => 320,
    }
}

fn int_from_config(config: &Value, key: &str, fallback: usize) -> usize {
    config
        .get(key)
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .filter(|&v| v > 0)
        .unwrap_or(fallback)
}

fn test_vad(provider: &str, config: &Value, pcm: &[f32]) -> ConfigTestResult {
    let effective = vad_provider_for_test(provider, config);
    let mut vad = match create_vad(&effective, config) {
        Ok(v) => v,
        Err(e) => return ConfigTestResult::fail(e.to_string(), None),
    };
    let samples = vad_test_sample_count(provider, config).min(pcm.len());
    let pcm_i16 = float_pcm_to_i16(&pcm[..samples]);
    let start = Instant::now();
    match vad.is_vad(&pcm_i16) {
        Ok(_) => ConfigTestResult::pass(start.elapsed().as_millis() as u64),
        Err(e) => ConfigTestResult::fail(e.to_string(), Some(start.elapsed().as_millis() as u64)),
    }
}

fn asr_engine_type(provider: &str, config: &Value) -> String {
    config
        .get("provider")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            let p = provider.trim();
            if p.is_empty() {
                Some("funasr".to_string())
            } else {
                Some(p.to_string())
            }
        })
        .unwrap_or_else(|| "funasr".to_string())
}

async fn test_asr(provider: &str, config: &Value, pcm: Vec<f32>) -> ConfigTestResult {
    let engine = asr_engine_type(provider, config);
    let asr = match create_asr(&engine, config) {
        Ok(v) => v,
        Err(e) => return ConfigTestResult::fail(e.to_string(), None),
    };
    if !asr.is_valid() {
        let hint = if engine == xiaozhi_core::asr::ALIYUN_QWEN3 {
            xiaozhi_core::dashscope_http_api_key_issue(&xiaozhi_core::dashscope_api_key(config))
                .unwrap_or("ASR 配置不完整（缺少 api_key 等）")
        } else {
            "ASR 配置不完整（缺少 api_key 等）"
        };
        return ConfigTestResult::fail(hint, None);
    }

    let (audio_tx, audio_rx) = tokio::sync::mpsc::channel(8);
    let pcm_clone = pcm.clone();
    tokio::spawn(async move {
        const CHUNK: usize = 3200;
        for i in (0..pcm_clone.len()).step_by(CHUNK) {
            let end = (i + CHUNK).min(pcm_clone.len());
            if audio_tx.send(pcm_clone[i..end].to_vec()).await.is_err() {
                return;
            }
        }
    });

    let start = Instant::now();
    let result = tokio::time::timeout(ASR_TEST_TIMEOUT, async {
        let mut result_rx = asr.streaming_recognize(audio_rx).await?;
        while let Some(r) = result_rx.recv().await {
            if let Some(err) = r.error {
                return Err(xiaozhi_core::Error::Other(err));
            }
        }
        Ok::<(), xiaozhi_core::Error>(())
    })
    .await;

    let elapsed = start.elapsed().as_millis() as u64;
    match result {
        Ok(Ok(())) => ConfigTestResult::pass(elapsed),
        Ok(Err(e)) => ConfigTestResult::fail(e.to_string(), Some(elapsed)),
        Err(_) => ConfigTestResult::fail("超时", Some(elapsed)),
    }
}

fn llm_thinking_enabled(config: &Value) -> bool {
    let Some(thinking) = config.get("thinking").and_then(|v| v.as_object()) else {
        return false;
    };
    if thinking.is_empty() {
        return false;
    }
    let mode = thinking
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    !mode.is_empty() && mode != "default"
}

async fn test_llm(pool_key: &str, config: &Value) -> ConfigTestResult {
    if pool_key.trim().is_empty() {
        return ConfigTestResult::fail("LLM provider 不能为空", None);
    }
    let llm = match create_llm(pool_key, config) {
        Ok(v) => v,
        Err(e) => return ConfigTestResult::fail(e.to_string(), None),
    };

    let timeout = if llm_thinking_enabled(config) {
        THINKING_LLM_TIMEOUT
    } else {
        DEFAULT_LLM_TIMEOUT
    };

    let start = Instant::now();
    let result = tokio::time::timeout(timeout, async {
        let mut rx = llm
            .response_with_context(
                "config_test",
                &[ChatMessage::user(DEFAULT_TEST_TEXT)],
                &[],
            )
            .await?;
        while let Some(msg) = rx.recv().await {
            if !msg.content.is_empty() {
                return Ok(start.elapsed().as_millis() as u64);
            }
        }
        Err(xiaozhi_core::Error::Http(
            "未收到响应或调用失败".into(),
        ))
    })
    .await;

    match result {
        Ok(Ok(ms)) => ConfigTestResult::pass(ms),
        Ok(Err(e)) => ConfigTestResult::fail(e.to_string(), Some(start.elapsed().as_millis() as u64)),
        Err(_) => ConfigTestResult::fail("超时", Some(start.elapsed().as_millis() as u64)),
    }
}

async fn test_tts(provider: &str, config: &Value) -> ConfigTestResult {
    if provider.trim().is_empty() {
        return ConfigTestResult::fail("TTS provider 不能为空", None);
    }
    let tts = match create_tts(provider, config) {
        Ok(v) => v,
        Err(e) => return ConfigTestResult::fail(e.to_string(), None),
    };
    // 对齐 Go configtest.go：不做 IsValid 预检，直接走合成并在失败时返回 API 错误

    let result = tokio::time::timeout(TTS_TEST_TIMEOUT, async {
        let mut rx = tts
            .text_to_speech_stream(DEFAULT_TEST_TEXT, 24000, 1, 60)
            .await?;
        // 与 Go configtest.go 一致：计时从流式 channel 就绪后开始，不含 WS 连接/握手
        let stream_start = Instant::now();
        let mut total = 0usize;
        let mut first_ms = None;
        while let Some(frame) = rx.recv().await {
            if first_ms.is_none() {
                first_ms = Some(stream_start.elapsed().as_millis() as u64);
            }
            total += frame.len();
        }
        let ms = first_ms.unwrap_or_else(|| stream_start.elapsed().as_millis() as u64);
        if total == 0 {
            Err(xiaozhi_core::Error::Http(
                "未收到有效音频或合成失败".into(),
            ))
        } else {
            Ok(ms)
        }
    })
    .await;

    match result {
        Ok(Ok(ms)) => ConfigTestResult::pass(ms),
        Ok(Err(e)) => ConfigTestResult::fail(e.to_string(), None),
        Err(_) => ConfigTestResult::fail("超时", None),
    }
}

fn memory_provider_for_test(provider: &str, config: &Value) -> String {
    if let Some(p) = config.get("provider").and_then(|v| v.as_str()) {
        let p = p.trim();
        if !p.is_empty() {
            return p.to_string();
        }
    }
    let p = provider.trim();
    if p.is_empty() {
        "nomemo".to_string()
    } else {
        p.to_string()
    }
}

async fn test_memory(provider: &str, config: &Value) -> ConfigTestResult {
    let effective = memory_provider_for_test(provider, config);
    if effective == xiaozhi_core::memory::NOMEMO {
        return ConfigTestResult::pass(0);
    }

    if matches!(
        effective.as_str(),
        xiaozhi_core::memory::MEM0
            | xiaozhi_core::memory::MEMOBASE
            | xiaozhi_core::memory::MEMOS
    ) {
        let api_key = config
            .get("api_key")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if api_key.is_empty() {
            return ConfigTestResult::fail("缺少 api_key", None);
        }
    }

    let memory = match create_memory(&effective, config) {
        Ok(v) => v,
        Err(e) => return ConfigTestResult::fail(e.to_string(), None),
    };

    let start = Instant::now();
    let result = tokio::time::timeout(MEMORY_TEST_TIMEOUT, async {
        memory.get_context(MEMORY_TEST_AGENT_ID, 128).await
    })
    .await;

    let elapsed = start.elapsed().as_millis() as u64;
    match result {
        Ok(Ok(_)) => ConfigTestResult::pass(elapsed),
        Ok(Err(e)) => ConfigTestResult::fail(e.to_string(), Some(elapsed)),
        Err(_) => ConfigTestResult::fail("超时", Some(elapsed)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn nomemo_memory_test_passes_immediately() {
        let result = test_memory("nomemo", &json!({})).await;
        assert!(result.ok);
    }

    #[tokio::test]
    async fn mem0_without_api_key_fails_fast() {
        let result = test_memory("mem0", &json!({"base_url": "https://api.mem0.ai"})).await;
        assert!(!result.ok);
        assert!(result.message.contains("api_key"));
    }
}
