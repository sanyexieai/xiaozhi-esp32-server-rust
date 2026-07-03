//! 复刻音频本地校验（对齐 Go `validateCloneAudioForProvider`）

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const MIN_MINIMAX_CLONE_AUDIO_SECONDS: f64 = 10.0;
const MAX_ALIYUN_QWEN_CLONE_AUDIO_BYTES: u64 = 10 * 1024 * 1024;
const MAX_ALIYUN_QWEN_CLONE_AUDIO_SECONDS: f64 = 60.0;

pub fn validate_clone_audio_for_provider(provider: &str, file_path: &str) -> Result<(), String> {
    let provider = provider.trim().to_lowercase();
    let ext = Path::new(file_path)
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| format!(".{}", s.to_lowercase()))
        .unwrap_or_default();

    match provider.as_str() {
        "doubao" => Ok(()),
        "minimax" => {
            if ext != ".wav" {
                return Err(format!("Minimax 仅支持 WAV 音频，检测到扩展名: {ext}"));
            }
            let seconds = wav_duration_seconds(file_path)?;
            if seconds < MIN_MINIMAX_CLONE_AUDIO_SECONDS {
                return Err(format!(
                    "Minimax 声音复刻要求音频时长至少 {:.0} 秒，当前 {:.2} 秒",
                    MIN_MINIMAX_CLONE_AUDIO_SECONDS, seconds
                ));
            }
            Ok(())
        }
        "cosyvoice" => {
            if ext != ".wav" {
                return Err(format!("CosyVoice 仅支持 WAV 音频，检测到扩展名: {ext}"));
            }
            let _ = wav_duration_seconds(file_path)?;
            Ok(())
        }
        "aliyun_qwen" => {
            let mime = aliyun_qwen_clone_audio_mime_type(&ext)
                .ok_or_else(|| format!("千问声音复刻仅支持 WAV/MP3/M4A，检测到扩展名: {ext}"))?;
            let meta = std::fs::metadata(file_path)
                .map_err(|e| format!("读取音频文件信息失败: {e}"))?;
            if meta.len() == 0 {
                return Err("音频文件不能为空".to_string());
            }
            if meta.len() > MAX_ALIYUN_QWEN_CLONE_AUDIO_BYTES {
                return Err(format!(
                    "千问声音复刻音频大小不能超过{}MB，当前{:.2}MB",
                    MAX_ALIYUN_QWEN_CLONE_AUDIO_BYTES / 1024 / 1024,
                    meta.len() as f64 / 1024.0 / 1024.0
                ));
            }
            if ext == ".wav" {
                let seconds = wav_duration_seconds(file_path)?;
                if seconds > MAX_ALIYUN_QWEN_CLONE_AUDIO_SECONDS {
                    return Err(format!(
                        "千问声音复刻音频时长不能超过 {:.0} 秒，当前 {:.2} 秒",
                        MAX_ALIYUN_QWEN_CLONE_AUDIO_SECONDS, seconds
                    ));
                }
            } else {
                let _ = mime;
            }
            Ok(())
        }
        "indextts_vllm" => {
            if !matches!(ext.as_str(), ".wav" | ".mp3" | ".flac" | ".m4a" | ".ogg") {
                return Err(format!(
                    "IndexTTS 声音复刻仅支持 WAV/MP3/FLAC/M4A/OGG，检测到扩展名: {ext}"
                ));
            }
            let meta = std::fs::metadata(file_path)
                .map_err(|e| format!("读取音频文件信息失败: {e}"))?;
            if meta.len() == 0 {
                return Err("音频文件不能为空".to_string());
            }
            Ok(())
        }
        other => Err(format!("暂不支持提供商 {other} 的音频校验")),
    }
}

fn aliyun_qwen_clone_audio_mime_type(ext: &str) -> Option<&'static str> {
    match ext.to_lowercase().as_str() {
        ".wav" => Some("audio/wav"),
        ".mp3" => Some("audio/mpeg"),
        ".m4a" => Some("audio/mp4"),
        _ => None,
    }
}

fn wav_duration_seconds(file_path: &str) -> Result<f64, String> {
    let mut f = File::open(file_path).map_err(|e| format!("打开音频文件失败: {e}"))?;
    let mut header = [0u8; 12];
    f.read_exact(&mut header)
        .map_err(|e| format!("读取 WAV 头失败: {e}"))?;
    if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" {
        return Err("不是有效的 WAV 文件".to_string());
    }

    let mut sample_rate = 0u32;
    let mut channels = 0u16;
    let mut bits_per_sample = 0u16;
    let mut data_bytes = 0u64;

    loop {
        let mut chunk_header = [0u8; 8];
        match f.read_exact(&mut chunk_header) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(format!("读取 WAV 分块头失败: {e}")),
        }
        let chunk_id = &chunk_header[0..4];
        let chunk_size = u32::from_le_bytes(chunk_header[4..8].try_into().unwrap());
        let chunk_size_i64 = i64::from(chunk_size);

        match chunk_id {
            b"fmt " => {
                if chunk_size < 16 {
                    return Err(format!("WAV fmt 分块长度无效: {chunk_size}"));
                }
                let mut fmt_data = vec![0u8; chunk_size as usize];
                f.read_exact(&mut fmt_data)
                    .map_err(|e| format!("读取 WAV fmt 分块失败: {e}"))?;
                let audio_format = u16::from_le_bytes(fmt_data[0..2].try_into().unwrap());
                if audio_format != 1 && audio_format != 3 {
                    return Err(format!("不支持的 WAV 编码格式: {audio_format}"));
                }
                channels = u16::from_le_bytes(fmt_data[2..4].try_into().unwrap());
                sample_rate = u32::from_le_bytes(fmt_data[4..8].try_into().unwrap());
                bits_per_sample = u16::from_le_bytes(fmt_data[14..16].try_into().unwrap());
            }
            b"data" => {
                data_bytes = u64::from(chunk_size);
                f.seek(SeekFrom::Current(chunk_size_i64))
                    .map_err(|e| format!("跳过 WAV data 分块失败: {e}"))?;
            }
            _ => {
                f.seek(SeekFrom::Current(chunk_size_i64))
                    .map_err(|e| format!("跳过 WAV 分块失败: {e}"))?;
            }
        }
        if chunk_size % 2 == 1 {
            f.seek(SeekFrom::Current(1))
                .map_err(|e| format!("跳过 WAV 对齐字节失败: {e}"))?;
        }
    }

    if sample_rate == 0 || channels == 0 || bits_per_sample == 0 || data_bytes == 0 {
        return Err(format!(
            "WAV 信息不完整(sample_rate={sample_rate} channels={channels} bits={bits_per_sample} data_bytes={data_bytes})"
        ));
    }
    let bytes_per_second =
        (f64::from(sample_rate) * f64::from(channels) * f64::from(bits_per_sample)) / 8.0;
    if bytes_per_second <= 0.0 {
        return Err("WAV 每秒字节数无效".to_string());
    }
    Ok(data_bytes as f64 / bytes_per_second)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_provider() {
        let err = validate_clone_audio_for_provider("unknown", "x.wav").unwrap_err();
        assert!(err.contains("暂不支持"));
    }
}
