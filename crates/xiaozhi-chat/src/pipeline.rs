use xiaozhi_core::{Error, Result};
use xiaozhi_protocol::audio::AudioParams;

use crate::opus_codec;

/// Opus 解码为 PCM（16-bit LE）；raw/pcm 直通
pub fn decode_opus_to_pcm(data: &[u8], params: &AudioParams) -> Result<Vec<u8>> {
    if data.is_empty() {
        return Ok(Vec::new());
    }
    if params.format == "raw" || params.format == "pcm" {
        return Ok(data.to_vec());
    }
    if opus_codec::is_opus_format(&params.format) {
        return opus_codec::decode_opus_packet(data, params);
    }
    Ok(data.to_vec())
}

/// 将 TTS 原始音频（PCM 或 MP3）转为设备二进制帧
pub fn tts_audio_to_device_frames(raw: &[u8], params: &AudioParams) -> Result<Vec<Vec<u8>>> {
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    let pcm_i16 = if is_mp3(raw) {
        decode_mp3_to_pcm_i16(raw)?
    } else {
        bytes_to_pcm_i16(raw)?
    };
    if opus_codec::is_opus_format(&params.format) {
        return opus_codec::encode_pcm_i16_to_opus_frames(&pcm_i16, params);
    }
    Ok(pcm_i16_to_frames(&pcm_i16, params))
}

pub fn is_mp3(data: &[u8]) -> bool {
    data.starts_with(b"ID3")
        || data.starts_with(&[0xFF, 0xFB])
        || data.starts_with(&[0xFF, 0xF3])
        || data.starts_with(&[0xFF, 0xF2])
}

fn bytes_to_pcm_i16(data: &[u8]) -> Result<Vec<i16>> {
    if data.len() % 2 != 0 {
        return Err(Error::Audio("PCM 数据长度必须为偶数".into()));
    }
    Ok(data
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect())
}

fn decode_mp3_to_pcm_i16(data: &[u8]) -> Result<Vec<i16>> {
    use std::io::Cursor;

    let mut decoder = minimp3::Decoder::new(Cursor::new(data));
    let mut pcm = Vec::new();
    loop {
        match decoder.next_frame() {
            Ok(frame) => pcm.extend_from_slice(&frame.data),
            Err(minimp3::Error::Eof) => break,
            Err(e) => {
                tracing::warn!("MP3 解码警告: {e}");
                break;
            }
        }
    }
    if pcm.is_empty() {
        return Err(Error::Audio("MP3 解码结果为空".into()));
    }
    Ok(pcm)
}

fn pcm_i16_to_frames(pcm: &[i16], params: &AudioParams) -> Vec<Vec<u8>> {
    let frame_samples = params.frame_size_samples().max(1);
    let frame_bytes = frame_samples * 2;
    let bytes: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
    bytes
        .chunks(frame_bytes)
        .map(|chunk| {
            let mut frame = chunk.to_vec();
            if frame.len() < frame_bytes {
                frame.resize(frame_bytes, 0);
            }
            frame
        })
        .collect()
}

/// 分句工具
pub fn split_sentences(text: &str) -> Vec<String> {
    let re = regex::Regex::new(r"[。！？；\n\.!\?;]").unwrap();
    let mut sentences = Vec::new();
    let mut last = 0;
    for mat in re.find_iter(text) {
        let sentence = text[last..mat.end()].trim().to_string();
        if !sentence.is_empty() {
            sentences.push(sentence);
        }
        last = mat.end();
    }
    if last < text.len() {
        let rest = text[last..].trim().to_string();
        if !rest.is_empty() {
            sentences.push(rest);
        }
    }
    if sentences.is_empty() && !text.trim().is_empty() {
        sentences.push(text.trim().to_string());
    }
    sentences
}

pub fn pcm_f32_to_i16(pcm: &[f32]) -> Vec<i16> {
    pcm.iter()
        .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
        .collect()
}

pub fn pcm_i16_to_f32(pcm: &[i16]) -> Vec<f32> {
    pcm.iter()
        .map(|&s| s as f32 / i16::MAX as f32)
        .collect()
}

pub fn validate_pcm(pcm: &[u8]) -> Result<()> {
    if pcm.len() % 2 != 0 {
        return Err(Error::Audio("PCM 数据长度必须为偶数".into()));
    }
    Ok(())
}
