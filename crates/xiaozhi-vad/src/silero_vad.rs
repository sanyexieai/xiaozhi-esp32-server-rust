//! Silero VAD（ONNX Runtime，对齐 Go `silero_vad`）

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use silero::{SampleRate, Session, SessionOptions, StreamState};
use xiaozhi_core::{Error, Result};

use crate::traits::VadProvider;

const SOURCE_MODEL_PATH: &str = "config/models/vad/silero_vad.onnx";
const RELEASE_MODEL_PATH: &str = "models/vad/silero_vad.onnx";

static RUNTIME_CACHE: OnceLock<Mutex<HashMap<PathBuf, SharedRuntime>>> = OnceLock::new();

struct SharedRuntime {
    session: Arc<Mutex<Session>>,
    refs: usize,
}

/// Silero VAD：共享 ONNX Session + 每路音频独立 StreamState（对齐 Go）
pub struct SileroVad {
    runtime_key: PathBuf,
    session: Arc<Mutex<Session>>,
    stream: StreamState,
    threshold: f32,
    channels: usize,
    last_voice: bool,
    closed: bool,
}

impl SileroVad {
    pub fn from_config(config: &serde_json::Value) -> Result<Self> {
        let model_path = config
            .get("model_path")
            .and_then(|v| v.as_str())
            .unwrap_or(SOURCE_MODEL_PATH);
        let resolved = resolve_model_path(model_path)?;

        let threshold = config
            .get("threshold")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.5) as f32;
        let sample_rate = config
            .get("sample_rate")
            .and_then(|v| v.as_u64())
            .unwrap_or(16000) as u32;
        let channels = config
            .get("channels")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as usize;

        let sample_rate = sample_rate_from_u32(sample_rate)?;
        let session = acquire_runtime(&resolved)?;
        let stream = StreamState::new(sample_rate);

        tracing::info!(
            model = %resolved.display(),
            threshold,
            sample_rate = ?sample_rate,
            channels,
            "Silero VAD 实例创建成功"
        );

        Ok(Self {
            runtime_key: resolved,
            session,
            stream,
            threshold,
            channels,
            last_voice: false,
            closed: false,
        })
    }

    fn infer_pcm(&mut self, pcm_f32: &[f32]) -> Result<bool> {
        if pcm_f32.is_empty() {
            return Ok(false);
        }
        let mono = downmix_to_mono(pcm_f32, self.channels);
        let mut session = self
            .session
            .lock()
            .map_err(|_| Error::Audio("Silero VAD session 锁已中毒".into()))?;
        let probs = session
            .process_stream(&mut self.stream, &mono)
            .map_err(map_silero_err)?;
        if probs.is_empty() {
            return Ok(self.last_voice);
        }
        let have_voice = probs.iter().any(|p| *p >= self.threshold);
        self.last_voice = have_voice;
        Ok(have_voice)
    }
}

impl VadProvider for SileroVad {
    fn is_vad(&mut self, pcm: &[i16]) -> Result<bool> {
        if self.closed {
            return Err(Error::Audio("Silero VAD 实例已关闭".into()));
        }
        if pcm.is_empty() {
            return Ok(false);
        }
        let pcm_f32: Vec<f32> = pcm
            .iter()
            .map(|&s| s as f32 / i16::MAX as f32)
            .collect();
        self.infer_pcm(&pcm_f32)
    }

    fn reset(&mut self) {
        self.stream.reset();
        self.last_voice = false;
    }

    fn close(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        release_runtime(&self.runtime_key);
        Ok(())
    }

    fn is_valid(&self) -> bool {
        !self.closed
    }
}

impl Drop for SileroVad {
    fn drop(&mut self) {
        if !self.closed {
            release_runtime(&self.runtime_key);
        }
    }
}

fn acquire_runtime(model_path: &Path) -> Result<Arc<Mutex<Session>>> {
    let cache = RUNTIME_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = cache
        .lock()
        .map_err(|_| Error::Audio("Silero VAD runtime 缓存锁已中毒".into()))?;

    if let Some(entry) = guard.get_mut(model_path) {
        entry.refs += 1;
        tracing::debug!(
            model = %model_path.display(),
            refs = entry.refs,
            "Silero VAD 共享 Runtime 复用"
        );
        return Ok(Arc::clone(&entry.session));
    }

    let session = Session::from_file_with_options(model_path, SessionOptions::default())
        .map_err(map_silero_err)?;
    let session = Arc::new(Mutex::new(session));
    guard.insert(
        model_path.to_path_buf(),
        SharedRuntime {
            session: Arc::clone(&session),
            refs: 1,
        },
    );
    tracing::debug!(
        model = %model_path.display(),
        refs = 1,
        "Silero VAD 共享 Runtime 创建"
    );
    Ok(session)
}

fn release_runtime(model_path: &Path) {
    let Some(cache) = RUNTIME_CACHE.get() else {
        return;
    };
    let Ok(mut guard) = cache.lock() else {
        return;
    };
    let Some(entry) = guard.get_mut(model_path) else {
        return;
    };
    entry.refs = entry.refs.saturating_sub(1);
    if entry.refs == 0 {
        guard.remove(model_path);
        tracing::debug!(model = %model_path.display(), "Silero VAD 共享 Runtime 销毁");
    }
}

fn resolve_model_path(model_path: &str) -> Result<PathBuf> {
    let trimmed = model_path.trim();
    if trimmed.is_empty() {
        return Err(Error::Config("Silero VAD 缺少 model_path".into()));
    }
    let candidates = build_model_path_candidates(trimmed);
    for candidate in &candidates {
        if candidate.is_file() {
            return Ok(candidate.clone());
        }
    }
    let tried = candidates
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Err(Error::Config(format!(
        "Silero VAD 模型文件不存在: {trimmed} (已尝试: {tried})"
    )))
}

fn build_model_path_candidates(model_path: &str) -> Vec<PathBuf> {
    let cleaned = PathBuf::from(model_path.replace('/', std::path::MAIN_SEPARATOR_STR));
    if cleaned.is_absolute() {
        return vec![cleaned];
    }

    let mut variants = vec![cleaned.clone()];
    let source = PathBuf::from(SOURCE_MODEL_PATH.replace('/', std::path::MAIN_SEPARATOR_STR));
    let release = PathBuf::from(RELEASE_MODEL_PATH.replace('/', std::path::MAIN_SEPARATOR_STR));
    if cleaned == source {
        variants.push(release);
    } else if cleaned == release {
        variants.push(source);
    }

    let mut roots = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            roots.push(parent.to_path_buf());
        }
    }
    if roots.is_empty() {
        roots.push(PathBuf::new());
    }

    let mut seen = std::collections::HashSet::new();
    let mut candidates = Vec::new();
    for root in roots {
        for variant in &variants {
            let candidate = if root.as_os_str().is_empty() {
                variant.clone()
            } else {
                root.join(variant)
            };
            if seen.insert(candidate.clone()) {
                candidates.push(candidate);
            }
        }
    }
    candidates
}

fn sample_rate_from_u32(rate: u32) -> Result<SampleRate> {
    match rate {
        8000 => Ok(SampleRate::Rate8k),
        16000 => Ok(SampleRate::Rate16k),
        _ => Err(Error::Config(format!(
            "Silero VAD 不支持的采样率: {rate}（仅支持 8000/16000）"
        ))),
    }
}

fn downmix_to_mono(pcm: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return pcm.to_vec();
    }
    let frame_count = pcm.len() / channels;
    let mut mono = Vec::with_capacity(frame_count);
    for frame in 0..frame_count {
        let offset = frame * channels;
        let sum: f32 = (0..channels).map(|c| pcm[offset + c]).sum();
        mono.push(sum / channels as f32);
    }
    mono
}

fn map_silero_err(err: silero::Error) -> Error {
    Error::Audio(format!("Silero VAD: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_path_candidates_include_release_fallback() {
        let candidates = build_model_path_candidates(SOURCE_MODEL_PATH);
        assert!(candidates.iter().any(|p| p.ends_with("silero_vad.onnx")));
    }
}
