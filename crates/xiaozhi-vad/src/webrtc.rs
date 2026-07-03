use std::time::{Duration, Instant};

use xiaozhi_core::Result;

use crate::traits::VadProvider;

/// 基于能量阈值的 VAD 实现（线程安全，可替代 WebRTC/Silero/TEN-VAD）
pub struct WebRtcVad {
    threshold: f32,
    frame_size: usize,
    last_used: Instant,
}

impl WebRtcVad {
    pub fn new(sample_rate: u32, mode: u8) -> Result<Self> {
        let threshold = match mode {
            0 => 500.0,
            1 => 300.0,
            2 => 200.0,
            _ => 100.0,
        };

        let frame_size = match sample_rate {
            8000 => 80,
            16000 => 160,
            32000 => 320,
            48000 => 480,
            _ => 160,
        };

        Ok(Self {
            threshold,
            frame_size,
            last_used: Instant::now(),
        })
    }

    pub fn from_config(config: &serde_json::Value) -> Result<Self> {
        let sample_rate = config
            .get("vad_sample_rate")
            .or_else(|| config.get("sample_rate"))
            .and_then(|v| v.as_u64())
            .unwrap_or(16000) as u32;
        let mode = config
            .get("vad_mode")
            .and_then(|v| v.as_u64())
            .unwrap_or(2) as u8;
        Self::new(sample_rate, mode)
    }

    fn frame_energy(pcm: &[i16]) -> f32 {
        if pcm.is_empty() {
            return 0.0;
        }
        let sum: f64 = pcm.iter().map(|&s| (s as f64) * (s as f64)).sum();
        (sum / pcm.len() as f64).sqrt() as f32
    }

    fn max_window_energy(pcm: &[i16], window: usize) -> f32 {
        if pcm.is_empty() {
            return 0.0;
        }
        if pcm.len() <= window {
            return Self::frame_energy(pcm);
        }
        pcm.windows(window)
            .map(Self::frame_energy)
            .fold(0.0_f32, f32::max)
    }
}

impl VadProvider for WebRtcVad {
    fn is_vad(&mut self, pcm: &[i16]) -> Result<bool> {
        self.last_used = Instant::now();
        if pcm.len() < self.frame_size {
            return Ok(false);
        }
        let energy = Self::max_window_energy(pcm, self.frame_size);
        Ok(energy > self.threshold)
    }

    fn reset(&mut self) {
        self.last_used = Instant::now();
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }

    fn is_valid(&self) -> bool {
        self.last_used.elapsed() < Duration::from_secs(3600)
    }
}
