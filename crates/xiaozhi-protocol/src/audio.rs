use serde::{Deserialize, Serialize};

/// 默认音频参数，与 Go 版 AudioFormat 对齐
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AudioParams {
    pub format: String,
    pub sample_rate: u32,
    pub channels: u8,
    pub frame_duration: u32,
}

impl Default for AudioParams {
    fn default() -> Self {
        Self {
            format: "opus".to_string(),
            sample_rate: 16000,
            channels: 1,
            frame_duration: 60,
        }
    }
}

impl AudioParams {
    pub fn frame_size_samples(&self) -> usize {
        (self.sample_rate as usize * self.frame_duration as usize) / 1000
    }
}
