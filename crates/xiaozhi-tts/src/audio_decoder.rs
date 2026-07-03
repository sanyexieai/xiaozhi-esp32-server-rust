//! 对齐 Go `util.CreateAudioDecoderWithSampleRate`：MP3/PCM/WAV → 重采样 → Opus 帧

use std::io::{self, Read};
use std::sync::mpsc as std_mpsc;

use opus::{Application, Bitrate, Channels, Encoder};
use tokio::sync::mpsc;
use xiaozhi_core::{Error, Result};

#[derive(Clone, Debug)]
struct AudioDecoderConfig {
    audio_format: String,
    target_sample_rate: u32,
    source_sample_rate: u32,
    channels: u8,
    frame_duration_ms: u32,
}

/// 将 Provider 原始音频流包装为设备 Opus 帧流（Go TTS Provider 出口格式）
pub fn wrap_tts_audio_stream(
    raw_rx: mpsc::Receiver<Vec<u8>>,
    audio_format: &str,
    target_sample_rate: u32,
    channels: u8,
    frame_duration_ms: u32,
) -> mpsc::Receiver<Vec<u8>> {
    wrap_tts_audio_stream_with_source(
        raw_rx,
        audio_format,
        target_sample_rate,
        0,
        channels,
        frame_duration_ms,
    )
}

/// `source_sample_rate` 仅对 raw PCM 有效；0 表示与目标采样率相同或从 MP3/WAV 头解析
pub fn wrap_tts_audio_stream_with_source(
    mut raw_rx: mpsc::Receiver<Vec<u8>>,
    audio_format: &str,
    target_sample_rate: u32,
    source_sample_rate: u32,
    channels: u8,
    frame_duration_ms: u32,
) -> mpsc::Receiver<Vec<u8>> {
    let fmt = audio_format.trim().to_ascii_lowercase();
    if fmt == "opus" || fmt == "ogg_opus" {
        return raw_rx;
    }

    let (out_tx, out_rx) = mpsc::channel(1000);
    let cfg = AudioDecoderConfig {
        audio_format: fmt,
        target_sample_rate: target_sample_rate.max(1),
        source_sample_rate,
        channels: channels.max(1),
        frame_duration_ms: frame_duration_ms.max(1),
    };

    tokio::spawn(async move {
        let (sync_tx, sync_rx) = std_mpsc::channel::<Vec<u8>>();
        let bridge = tokio::spawn(async move {
            while let Some(chunk) = raw_rx.recv().await {
                if sync_tx.send(chunk).is_err() {
                    break;
                }
            }
        });
        let decode = tokio::task::spawn_blocking(move || {
            if let Err(e) = run_decoder(sync_rx, cfg, out_tx) {
                tracing::error!("TTS audio decoder 失败: {e}");
            }
        });
        let _ = bridge.await;
        let _ = decode.await;
    });

    out_rx
}

struct SyncPipe {
    rx: std_mpsc::Receiver<Vec<u8>>,
    buf: Vec<u8>,
    pos: usize,
    closed: bool,
}

impl SyncPipe {
    fn new(rx: std_mpsc::Receiver<Vec<u8>>) -> Self {
        Self {
            rx,
            buf: Vec::new(),
            pos: 0,
            closed: false,
        }
    }
}

impl Read for SyncPipe {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        loop {
            if self.pos < self.buf.len() {
                let n = out.len().min(self.buf.len() - self.pos);
                out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
                self.pos += n;
                return Ok(n);
            }
            if self.closed {
                return Ok(0);
            }
            match self.rx.recv() {
                Ok(chunk) => self.buf.extend_from_slice(&chunk),
                Err(_) => self.closed = true,
            }
        }
    }
}

fn run_decoder(
    raw_rx: std_mpsc::Receiver<Vec<u8>>,
    cfg: AudioDecoderConfig,
    out_tx: mpsc::Sender<Vec<u8>>,
) -> Result<()> {
    match cfg.audio_format.as_str() {
        "mp3" => run_mp3_decoder(raw_rx, &cfg, out_tx),
        "pcm" | "raw" => run_pcm_decoder(raw_rx, &cfg, out_tx),
        "wav" => run_wav_decoder(raw_rx, &cfg, out_tx),
        other => Err(Error::Audio(format!("不支持的 TTS 音频格式: {other}"))),
    }
}

fn run_mp3_decoder(
    raw_rx: std_mpsc::Receiver<Vec<u8>>,
    cfg: &AudioDecoderConfig,
    out_tx: mpsc::Sender<Vec<u8>>,
) -> Result<()> {
    let mut pipe = SyncPipe::new(raw_rx);
    let mut decoder = minimp3::Decoder::new(&mut pipe);

    let frame_duration = cfg.frame_duration_ms;
    let target_rate = cfg.target_sample_rate;
    let mut source_rate = 0u32;
    let mut source_frame_size = 0usize;
    let mut pcm_buffer: Vec<i16> = Vec::new();
    let mut encoder: Option<Encoder> = None;

    loop {
        match decoder.next_frame() {
            Ok(frame) => {
                if source_rate == 0 {
                    source_rate = frame.sample_rate.max(0) as u32;
                    if source_rate == 0 {
                        source_rate = 24000;
                    }
                    source_frame_size = (source_rate as usize * frame_duration as usize) / 1000;
                    encoder = Some(create_opus_encoder(target_rate, cfg.channels)?);
                    pcm_buffer.reserve(source_frame_size);
                }
                append_mp3_frame_mono(&mut pcm_buffer, &frame);
                while pcm_buffer.len() >= source_frame_size {
                    let frame_pcm: Vec<i16> = pcm_buffer.drain(..source_frame_size).collect();
                    emit_opus_frame(
                        encoder.as_mut().unwrap(),
                        &frame_pcm,
                        source_rate,
                        target_rate,
                        (target_rate as usize * frame_duration as usize) / 1000,
                        &out_tx,
                    )?;
                }
            }
            Err(minimp3::Error::Eof) => break,
            Err(e) => {
                tracing::warn!("MP3 解码警告: {e}");
                break;
            }
        }
    }

    if !pcm_buffer.is_empty() {
        if let Some(enc) = encoder.as_mut() {
            let mut padded = vec![0i16; source_frame_size.max(pcm_buffer.len())];
            padded[..pcm_buffer.len()].copy_from_slice(&pcm_buffer);
            emit_opus_frame(
                enc,
                &padded,
                source_rate,
                target_rate,
                (target_rate as usize * frame_duration as usize) / 1000,
                &out_tx,
            )?;
        }
    }
    Ok(())
}

fn run_pcm_decoder(
    raw_rx: std_mpsc::Receiver<Vec<u8>>,
    cfg: &AudioDecoderConfig,
    out_tx: mpsc::Sender<Vec<u8>>,
) -> Result<()> {
    let mut source_rate = cfg.source_sample_rate;
    if source_rate == 0 {
        source_rate = cfg.target_sample_rate;
    }
    let target_rate = cfg.target_sample_rate;
    let frame_duration = cfg.frame_duration_ms;
    let source_frame_size = (source_rate as usize * frame_duration as usize) / 1000;
    let mut encoder = create_opus_encoder(target_rate, cfg.channels)?;
    let mut pcm_buffer: Vec<i16> = Vec::with_capacity(source_frame_size);
    let mut byte_remainder: Vec<u8> = Vec::new();

    while let Ok(chunk) = raw_rx.recv() {
        byte_remainder.extend_from_slice(&chunk);

        let usable = byte_remainder.len() - (byte_remainder.len() % 2);
        if usable == 0 {
            continue;
        }
        let pcm_bytes: Vec<u8> = byte_remainder.drain(..usable).collect();

        for pair in pcm_bytes.chunks_exact(2) {
            pcm_buffer.push(i16::from_le_bytes([pair[0], pair[1]]));
            if pcm_buffer.len() >= source_frame_size {
                let frame_pcm: Vec<i16> = pcm_buffer.drain(..source_frame_size).collect();
                emit_opus_frame(
                    &mut encoder,
                    &frame_pcm,
                    source_rate,
                    target_rate,
                    (target_rate as usize * frame_duration as usize) / 1000,
                    &out_tx,
                )?;
            }
        }
    }

    if !pcm_buffer.is_empty() {
        let mut padded = vec![0i16; source_frame_size.max(pcm_buffer.len())];
        padded[..pcm_buffer.len()].copy_from_slice(&pcm_buffer);
        emit_opus_frame(
            &mut encoder,
            &padded,
            source_rate,
            target_rate,
            (target_rate as usize * frame_duration as usize) / 1000,
            &out_tx,
        )?;
    }
    Ok(())
}

fn run_wav_decoder(
    raw_rx: std_mpsc::Receiver<Vec<u8>>,
    cfg: &AudioDecoderConfig,
    out_tx: mpsc::Sender<Vec<u8>>,
) -> Result<()> {
    let mut all = Vec::new();
    while let Ok(chunk) = raw_rx.recv() {
        all.extend_from_slice(&chunk);
    }
    let (source_rate, pcm) = parse_wav_pcm(&all)?;
    let target_rate = cfg.target_sample_rate;
    let frame_duration = cfg.frame_duration_ms;
    let source_frame_size = (source_rate as usize * frame_duration as usize) / 1000;
    let mut encoder = create_opus_encoder(target_rate, cfg.channels)?;
    let mut offset = 0usize;
    while offset + source_frame_size <= pcm.len() {
        let frame_pcm = pcm[offset..offset + source_frame_size].to_vec();
        offset += source_frame_size;
        emit_opus_frame(
            &mut encoder,
            &frame_pcm,
            source_rate,
            target_rate,
            (target_rate as usize * frame_duration as usize) / 1000,
            &out_tx,
        )?;
    }
    if offset < pcm.len() {
        let mut padded = vec![0i16; source_frame_size];
        let tail = &pcm[offset..];
        padded[..tail.len()].copy_from_slice(tail);
        emit_opus_frame(
            &mut encoder,
            &padded,
            source_rate,
            target_rate,
            (target_rate as usize * frame_duration as usize) / 1000,
            &out_tx,
        )?;
    }
    Ok(())
}

fn parse_wav_pcm(data: &[u8]) -> Result<(u32, Vec<i16>)> {
    if data.len() < 44 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(Error::Audio("无效的 WAV 数据".into()));
    }
    let channels =
        u16::from_le_bytes([data[22], data[23]]).max(1) as usize;
    let sample_rate = u32::from_le_bytes([data[24], data[25], data[26], data[27]]);
    let data_offset = find_wav_data_offset(data).ok_or_else(|| Error::Audio("WAV 缺少 data 块".into()))?;
    let pcm_bytes = &data[data_offset..];
    if pcm_bytes.len() % 2 != 0 {
        return Err(Error::Audio("WAV PCM 长度无效".into()));
    }
    let mut pcm = Vec::with_capacity(pcm_bytes.len() / 2 / channels);
    for frame in pcm_bytes.chunks(channels * 2) {
        if frame.len() < 2 {
            break;
        }
        if channels <= 1 {
            pcm.push(i16::from_le_bytes([frame[0], frame[1]]));
        } else {
            let mut sum = 0i32;
            for ch in frame.chunks_exact(2) {
                sum += i32::from(i16::from_le_bytes([ch[0], ch[1]]));
            }
            pcm.push((sum / channels as i32) as i16);
        }
    }
    Ok((sample_rate.max(1), pcm))
}

fn find_wav_data_offset(data: &[u8]) -> Option<usize> {
    let mut offset = 12usize;
    while offset + 8 <= data.len() {
        let chunk_size = u32::from_le_bytes(data[offset + 4..offset + 8].try_into().ok()?) as usize;
        if &data[offset..offset + 4] == b"data" {
            return Some(offset + 8);
        }
        offset += 8 + chunk_size;
    }
    None
}

fn append_mp3_frame_mono(pcm: &mut Vec<i16>, frame: &minimp3::Frame) {
    if frame.channels <= 1 {
        pcm.extend_from_slice(&frame.data);
        return;
    }
    for chunk in frame.data.chunks(frame.channels) {
        let sum: i32 = chunk.iter().map(|&s| i32::from(s)).sum();
        pcm.push((sum / frame.channels as i32) as i16);
    }
}

fn emit_opus_frame(
    encoder: &mut Encoder,
    frame_pcm: &[i16],
    source_rate: u32,
    target_rate: u32,
    target_frame_samples: usize,
    out_tx: &mpsc::Sender<Vec<u8>>,
) -> Result<()> {
    let mut opus_pcm = frame_pcm.to_vec();
    if source_rate > 0 && target_rate > 0 && source_rate != target_rate {
        opus_pcm = resample_linear_i16(&opus_pcm, source_rate, target_rate);
    }
    if target_frame_samples == 0 {
        return Ok(());
    }
    if opus_pcm.len() < target_frame_samples {
        opus_pcm.resize(target_frame_samples, 0);
    } else if opus_pcm.len() > target_frame_samples {
        opus_pcm.truncate(target_frame_samples);
    }
    let mut out = vec![0u8; 4000];
    let nbytes = encoder
        .encode(&opus_pcm, &mut out)
        .map_err(|e| Error::Audio(format!("Opus 编码失败: {e}")))?;
    out_tx
        .blocking_send(out[..nbytes].to_vec())
        .map_err(|_| Error::Audio("Opus 帧发送失败".into()))
}

fn create_opus_encoder(sample_rate: u32, channels: u8) -> Result<Encoder> {
    let ch = if channels <= 1 {
        Channels::Mono
    } else {
        Channels::Stereo
    };
    let mut encoder = Encoder::new(sample_rate, ch, Application::Audio)
        .map_err(|e| Error::Audio(format!("Opus 编码器初始化失败: {e}")))?;
    encoder
        .set_bitrate(Bitrate::Bits(16000))
        .map_err(|e| Error::Audio(format!("Opus 设置码率失败: {e}")))?;
    encoder
        .set_vbr(false)
        .map_err(|e| Error::Audio(format!("Opus 设置 CBR 失败: {e}")))?;
    Ok(encoder)
}

fn resample_linear_i16(input: &[i16], in_rate: u32, out_rate: u32) -> Vec<i16> {
    if input.is_empty() || in_rate == 0 || out_rate == 0 || in_rate == out_rate {
        return input.to_vec();
    }
    let ratio = f64::from(out_rate) / f64::from(in_rate);
    let out_len = (input.len() as f64 * ratio).floor() as usize;
    if out_len == 0 {
        return Vec::new();
    }
    let mut output = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = f64::from(i as u32) / ratio;
        let index = pos.floor() as usize;
        if index >= input.len().saturating_sub(1) {
            output.push(input[input.len() - 1]);
        } else {
            let frac = (pos - index as f64) as f32;
            let a = input[index] as f32;
            let b = input[index + 1] as f32;
            output.push((a * (1.0 - frac) + b * frac) as i16);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_24k_to_16k_shortens_pcm() {
        let input: Vec<i16> = (0..2400).map(|i| (i % 100) as i16).collect();
        let output = resample_linear_i16(&input, 24000, 16000);
        assert_eq!(output.len(), 1600);
    }
}
