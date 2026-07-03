//! 火山引擎 V3 二进制协议（TTS WebSocket）

use xiaozhi_core::{Error, Result};

pub const EVENT_SESSION_FINISHED: i32 = 152;
pub const EVENT_TTS_RESPONSE: i32 = 352;

const VERSION1: u8 = 1;
const HEADER_SIZE4: u8 = 1;
const MSG_FULL_CLIENT: u8 = 0x1;
const MSG_FULL_SERVER: u8 = 0x9;
const MSG_AUDIO_ONLY_SERVER: u8 = 0xB;
const MSG_ERROR: u8 = 0xF;
const FLAG_NO_SEQ: u8 = 0;
const FLAG_WITH_EVENT: u8 = 0x4;
const FLAG_NEGATIVE_SEQ: u8 = 0x2;
const SER_JSON: u8 = 0x1;
const COMP_NONE: u8 = 0;
const COMP_GZIP: u8 = 0x1;

#[derive(Debug, Clone)]
pub struct VolcMessage {
    pub msg_type: u8,
    pub flags: u8,
    pub event: i32,
    pub sequence: i32,
    pub error_code: u32,
    pub payload: Vec<u8>,
}

pub fn marshal_full_client(payload: &[u8]) -> Vec<u8> {
    marshal_message(MSG_FULL_CLIENT, FLAG_NO_SEQ, 0, 0, 0, payload)
}

/// 豆包 WebSocket TTS 请求帧（gzip 压缩，对齐 Golang `buildDoubaoWSBinaryRequest`）
pub fn marshal_doubao_ws_binary_request(payload: &[u8]) -> Result<Vec<u8>> {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(payload)
        .map_err(|e| Error::Protocol(format!("gzip 压缩失败: {e}")))?;
    let compressed = encoder
        .finish()
        .map_err(|e| Error::Protocol(format!("gzip 压缩失败: {e}")))?;

    let mut frame = vec![0x11, 0x10, 0x11, 0x00];
    frame.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
    frame.extend_from_slice(&compressed);
    Ok(frame)
}

pub fn marshal_finish_connection() -> Vec<u8> {
    marshal_message(MSG_FULL_CLIENT, FLAG_WITH_EVENT, 2, 0, 0, &[])
}

fn marshal_message(
    msg_type: u8,
    flags: u8,
    event: i32,
    sequence: i32,
    error_code: u32,
    payload: &[u8],
) -> Vec<u8> {
    let mut out = vec![
        (VERSION1 << 4) | HEADER_SIZE4,
        (msg_type << 4) | flags,
        (SER_JSON << 4) | COMP_NONE,
        0,
    ];

    if flags == FLAG_WITH_EVENT {
        out.extend_from_slice(&event.to_be_bytes());
        if event != 1 && event != 2 && event != 50 && event != 51 && event != 52 {
            out.extend_from_slice(&(0u32).to_be_bytes());
        }
    }

    if flags == FLAG_NEGATIVE_SEQ || flags == 1 {
        out.extend_from_slice(&sequence.to_be_bytes());
    }

    if msg_type == MSG_ERROR {
        out.extend_from_slice(&error_code.to_be_bytes());
    }

    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

pub fn parse_message(data: &[u8]) -> Result<VolcMessage> {
    if data.len() < 4 {
        return Err(Error::Protocol("火山协议帧过短".into()));
    }

    let header_size = (data[0] & 0x0F) as usize * 4;
    if data.len() < header_size {
        return Err(Error::Protocol("火山协议头不完整".into()));
    }

    let msg_type = data[1] >> 4;
    let flags = data[1] & 0x0F;
    let compression = data[2] & 0x0F;
    let mut pos = header_size;

    let mut event = 0i32;
    let mut sequence = 0i32;
    let mut error_code = 0u32;

    if flags == FLAG_WITH_EVENT {
        if data.len() < pos + 4 {
            return Err(Error::Protocol("火山协议缺少 event".into()));
        }
        event = i32::from_be_bytes(data[pos..pos + 4].try_into().unwrap());
        pos += 4;

        if !matches!(event, 1 | 2 | 50 | 51 | 52) {
            if data.len() < pos + 4 {
                return Err(Error::Protocol("火山协议缺少 session_id 长度".into()));
            }
            let sid_len = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
            pos += 4;
            if data.len() < pos + sid_len {
                return Err(Error::Protocol("火山协议 session_id 不完整".into()));
            }
            pos += sid_len;
        }

        if matches!(event, 50 | 51 | 52) {
            if data.len() >= pos + 4 {
                let cid_len = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
                pos += 4;
                if data.len() >= pos + cid_len {
                    pos += cid_len;
                }
            }
        }
    }

    if flags == 1 || flags == FLAG_NEGATIVE_SEQ {
        if data.len() < pos + 4 {
            return Err(Error::Protocol("火山协议缺少 sequence".into()));
        }
        sequence = i32::from_be_bytes(data[pos..pos + 4].try_into().unwrap());
        pos += 4;
    }

    if msg_type == MSG_ERROR {
        if data.len() < pos + 4 {
            return Err(Error::Protocol("火山协议缺少 error_code".into()));
        }
        error_code = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap());
        pos += 4;
    }

    if data.len() < pos + 4 {
        return Err(Error::Protocol("火山协议缺少 payload 长度".into()));
    }
    let payload_len = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
    pos += 4;
    if data.len() < pos + payload_len {
        return Err(Error::Protocol("火山协议 payload 不完整".into()));
    }
    let mut payload = data[pos..pos + payload_len].to_vec();

    if compression == COMP_GZIP && !payload.is_empty() {
        payload = gunzip(&payload)?;
    }

    Ok(VolcMessage {
        msg_type,
        flags,
        event,
        sequence,
        error_code,
        payload,
    })
}

fn gunzip(data: &[u8]) -> Result<Vec<u8>> {
    use flate2::read::GzDecoder;
    use std::io::Read;
    let mut dec = GzDecoder::new(data);
    let mut out = Vec::new();
    dec.read_to_end(&mut out)
        .map_err(|e| Error::Protocol(format!("gzip 解压失败: {e}")))?;
    Ok(out)
}

pub fn extract_audio_from_event(msg: &VolcMessage) -> Option<Vec<u8>> {
    use base64::Engine;

    if msg.msg_type == MSG_AUDIO_ONLY_SERVER && !msg.payload.is_empty() {
        return Some(msg.payload.clone());
    }

    if msg.event == EVENT_TTS_RESPONSE && !msg.payload.is_empty() {
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&msg.payload) {
            for key in ["data", "audio", "audio_data"] {
                if let Some(b64) = v.get(key).and_then(|x| x.as_str()) {
                    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) {
                        return Some(bytes);
                    }
                }
            }
        }
    }
    None
}

pub fn is_stream_end(msg: &VolcMessage) -> bool {
    msg.event == EVENT_SESSION_FINISHED
        || (msg.msg_type == MSG_AUDIO_ONLY_SERVER && msg.sequence < 0)
        || msg.msg_type == MSG_ERROR
}
