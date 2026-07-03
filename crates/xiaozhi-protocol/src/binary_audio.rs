//! ESP32 WebSocket 二进制音频帧（BinaryProtocol v1/v2/v3）

const BINARY_PROTOCOL2_HEADER: usize = 16;
const BINARY_PROTOCOL3_HEADER: usize = 4;

/// 从设备上行二进制帧中提取 Opus 载荷。
/// `protocol_version`: hello / Protocol-Version 协商值（1=裸 Opus，2/3=带帧头）。
///
/// 与 Go `websocket_conn.go` 一致：普通 WebSocket 在 v1 下**不做**帧头猜测剥离，
/// 仅当协商版本为 2/3 且帧长度与 header 完全匹配时才剥离。
pub fn unpack_device_audio<'a>(data: &'a [u8], protocol_version: u8) -> &'a [u8] {
    match protocol_version {
        2 => unpack_v2(data).unwrap_or(data),
        3 => unpack_v3(data).unwrap_or(data),
        _ => data,
    }
}

/// 将 Opus 载荷打包为设备可识别的二进制帧（与 ESP32 `WebsocketProtocol::SendAudio` 对齐）。
pub fn pack_device_audio(opus: &[u8], protocol_version: u8) -> Vec<u8> {
    match protocol_version {
        2 => pack_v2(opus, 0),
        3 => pack_v3(opus),
        _ => opus.to_vec(),
    }
}

fn unpack_v2(data: &[u8]) -> Option<&[u8]> {
    if data.len() < BINARY_PROTOCOL2_HEADER {
        return None;
    }
    let version = u16::from_be_bytes([data[0], data[1]]);
    if version != 2 {
        return None;
    }
    let payload_size = u32::from_be_bytes(data[12..16].try_into().ok()?) as usize;
    let end = BINARY_PROTOCOL2_HEADER.checked_add(payload_size)?;
    if end != data.len() {
        return None;
    }
    Some(&data[BINARY_PROTOCOL2_HEADER..end])
}

fn unpack_v3(data: &[u8]) -> Option<&[u8]> {
    if data.len() < BINARY_PROTOCOL3_HEADER {
        return None;
    }
    let payload_size = u16::from_be_bytes(data[2..4].try_into().ok()?) as usize;
    let end = BINARY_PROTOCOL3_HEADER.checked_add(payload_size)?;
    if end != data.len() {
        return None;
    }
    if data[0] != 0 {
        return None;
    }
    Some(&data[BINARY_PROTOCOL3_HEADER..end])
}

fn pack_v2(opus: &[u8], timestamp_ms: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(BINARY_PROTOCOL2_HEADER + opus.len());
    out.extend_from_slice(&2u16.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&timestamp_ms.to_be_bytes());
    out.extend_from_slice(&(opus.len() as u32).to_be_bytes());
    out.extend_from_slice(opus);
    out
}

fn pack_v3(opus: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(BINARY_PROTOCOL3_HEADER + opus.len());
    out.push(0);
    out.push(0);
    out.extend_from_slice(&(opus.len() as u16).to_be_bytes());
    out.extend_from_slice(opus);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpack_v2_roundtrip() {
        let opus = vec![0xF8u8, 0xFF, 0xFE];
        let packed = pack_v2(&opus, 1234);
        assert_eq!(unpack_device_audio(&packed, 2), opus.as_slice());
    }

    #[test]
    fn unpack_v3_roundtrip() {
        let opus = vec![0xF8u8, 0xFF, 0xFE];
        let packed = pack_v3(&opus);
        assert_eq!(unpack_device_audio(&packed, 3), opus.as_slice());
    }

    #[test]
    fn v1_passthrough_raw_opus() {
        let opus = vec![0x58u8, 0x01, 0x02, 0x03];
        assert_eq!(unpack_device_audio(&opus, 1), opus.as_slice());
    }

    #[test]
    fn v2_rejects_length_mismatch() {
        let opus = vec![0xF8u8, 0xFF, 0xFE];
        let mut packed = pack_v2(&opus, 0);
        packed.push(0xFF);
        assert_eq!(unpack_device_audio(&packed, 2), packed.as_slice());
    }
}
