use async_trait::async_trait;
use xiaozhi_core::Result;

#[async_trait]
pub trait VadProvider: Send + Sync {
    /// 检测 PCM 音频帧是否包含语音 (16-bit LE PCM)
    fn is_vad(&mut self, pcm: &[i16]) -> Result<bool>;

    fn reset(&mut self);

    fn close(&mut self) -> Result<()>;

    fn is_valid(&self) -> bool;
}
