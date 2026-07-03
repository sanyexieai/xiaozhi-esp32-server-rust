//! Opus 编解码（libopus，与 ESP32 / Go 版服务端一致）

use std::sync::Mutex;

use opus::{Application, Bitrate, Channels, Decoder, Encoder};
use xiaozhi_core::{Error, Result};
use xiaozhi_protocol::audio::AudioParams;

fn channels_from(params: &AudioParams) -> Channels {
    if params.channels <= 1 {
        Channels::Mono
    } else {
        Channels::Stereo
    }
}

/// 会话级 Opus 解码器（必须复用，逐包 new 会导致状态错误）
pub struct OpusStreamDecoder {
    decoder: Mutex<Decoder>,
}

impl OpusStreamDecoder {
    pub fn new(params: &AudioParams) -> Result<Self> {
        let decoder = Decoder::new(params.sample_rate, channels_from(params))
            .map_err(|e| Error::Audio(format!("Opus 解码器初始化失败: {e}")))?;
        Ok(Self {
            decoder: Mutex::new(decoder),
        })
    }

    pub fn decode_to_pcm_i16_le(&self, data: &[u8]) -> Result<Vec<u8>> {
        if data.is_empty() {
            return Ok(Vec::new());
        }
        let mut pcm_i16 = vec![0i16; 5760];
        let samples = self
            .decoder
            .lock()
            .map_err(|e| Error::Audio(format!("Opus 解码器锁失败: {e}")))?
            .decode(data, &mut pcm_i16, false)
            .map_err(|e| Error::Audio(format!("Opus 解码失败: {e}")))?;
        let mut out = Vec::with_capacity(samples * 2);
        for &s in &pcm_i16[..samples] {
            out.extend_from_slice(&s.to_le_bytes());
        }
        Ok(out)
    }
}

pub fn decode_opus_packet(data: &[u8], params: &AudioParams) -> Result<Vec<u8>> {
    let dec = OpusStreamDecoder::new(params)?;
    dec.decode_to_pcm_i16_le(data)
}

pub fn encode_pcm_i16_to_opus_frames(pcm: &[i16], params: &AudioParams) -> Result<Vec<Vec<u8>>> {
    if pcm.is_empty() {
        return Ok(Vec::new());
    }
    let frame_samples = params.frame_size_samples().max(1);
    let mut encoder = Encoder::new(
        params.sample_rate,
        channels_from(params),
        Application::Voip,
    )
    .map_err(|e| Error::Audio(format!("Opus 编码器初始化失败: {e}")))?;
    encoder
        .set_bitrate(Bitrate::Bits(16000))
        .map_err(|e| Error::Audio(format!("Opus 设置码率失败: {e}")))?;
    encoder
        .set_vbr(false)
        .map_err(|e| Error::Audio(format!("Opus 设置 CBR 失败: {e}")))?;

    let mut frames = Vec::new();
    for chunk in pcm.chunks(frame_samples) {
        let mut frame = vec![0i16; frame_samples];
        frame[..chunk.len()].copy_from_slice(chunk);
        let mut out = vec![0u8; 4000];
        let nbytes = encoder
            .encode(&frame, &mut out)
            .map_err(|e| Error::Audio(format!("Opus 编码失败: {e}")))?;
        frames.push(out[..nbytes].to_vec());
    }
    Ok(frames)
}

pub fn is_opus_format(format: &str) -> bool {
    format == "opus" || format == "ogg_opus"
}
