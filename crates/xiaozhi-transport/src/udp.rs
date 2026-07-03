//! UDP AES-CTR 加密音频传输（对齐 ESP32 `mqtt_protocol.cc`）
//!
//! 包格式（16 字节头 + 密文）：
//! | type 1u | flags 1u | payload_len 2u BE | ssrc 4u BE | timestamp 4u BE | sequence 4u BE | payload |

use aes::Aes128;
use ctr::cipher::{KeyIvInit, StreamCipher};
use rand::Rng;
use xiaozhi_core::{Error, Result};

type Aes128Ctr = ctr::Ctr128BE<Aes128>;

const UDP_PACKET_TYPE_AUDIO: u8 = 0x01;

pub struct UdpCrypto {
    key: [u8; 16],
    nonce_template: [u8; 16],
}

impl UdpCrypto {
    pub fn new(key: [u8; 16], nonce_template: [u8; 16]) -> Self {
        Self {
            key,
            nonce_template,
        }
    }

    pub fn nonce_template(&self) -> &[u8; 16] {
        &self.nonce_template
    }

    /// 生成会话密钥；hello nonce 布局对齐 Go `GetAesKeyAndNonce`：
    /// `[0]=0x01, [4..8]=conn_id, [8..12]=创建时 unix 秒, [12..16]=0`。
    pub fn generate_session_keys() -> ([u8; 16], [u8; 16], u32) {
        let mut rng = rand::thread_rng();
        let mut key = [0u8; 16];
        rng.fill(&mut key);
        let mut nonce = [0u8; 16];
        nonce[0] = UDP_PACKET_TYPE_AUDIO;
        let conn_id = rng.gen::<u32>() & 0x7FFF_FFFF;
        nonce[4..8].copy_from_slice(&conn_id.to_be_bytes());
        let created = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as u32)
            .unwrap_or(0);
        nonce[8..12].copy_from_slice(&created.to_be_bytes());
        (key, nonce, conn_id)
    }

    pub fn conn_id_from_nonce(nonce: &[u8; 16]) -> u32 {
        u32::from_be_bytes(nonce[4..8].try_into().expect("ssrc slice"))
    }

    /// 服务端下行加密（TTS → 设备），对齐 Go `UdpSession.Encrypt`：
    /// `[4..12]` 固定为会话 nonce 模板，`[12..16]` 为递增 sequence。
    pub fn encrypt(&self, sequence: u32, payload: &[u8]) -> Vec<u8> {
        let header = self.build_downlink_header(payload.len(), sequence);
        let mut cipher =
            Aes128Ctr::new_from_slices(&self.key, &header).expect("AES key/nonce length");
        let mut encrypted = payload.to_vec();
        cipher.apply_keystream(&mut encrypted);

        let mut packet = header.to_vec();
        packet.extend_from_slice(&encrypted);
        packet
    }

    /// 设备上行解密（设备 → 服务端）
    pub fn decrypt(&self, packet: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
        if packet.len() < 16 {
            return Err(Error::Protocol("UDP 包太短".into()));
        }
        if packet[0] != UDP_PACKET_TYPE_AUDIO {
            return Err(Error::Protocol(format!(
                "UDP 包 type 无效: {}",
                packet[0]
            )));
        }

        let timestamp = u32::from_be_bytes(packet[8..12].try_into().unwrap());
        let sequence = u32::from_be_bytes(packet[12..16].try_into().unwrap());
        let header: [u8; 16] = packet[..16].try_into().unwrap();

        let mut cipher = Aes128Ctr::new_from_slices(&self.key, &header)
            .map_err(|e| Error::Protocol(format!("AES 初始化失败: {e}")))?;
        let mut payload = packet[16..].to_vec();
        cipher.apply_keystream(&mut payload);

        Ok((timestamp, sequence, payload))
    }

    fn build_downlink_header(&self, payload_len: usize, sequence: u32) -> [u8; 16] {
        let mut header = self.nonce_template;
        header[2..4].copy_from_slice(&(payload_len as u16).to_be_bytes());
        header[12..16].copy_from_slice(&sequence.to_be_bytes());
        header
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_matches_esp32_header_layout() {
        let (key, nonce, conn_id) = UdpCrypto::generate_session_keys();
        assert_eq!(nonce[0], UDP_PACKET_TYPE_AUDIO);
        assert_eq!(UdpCrypto::conn_id_from_nonce(&nonce), conn_id);

        let crypto = UdpCrypto::new(key, nonce);
        let payload = vec![0xAB; 124];
        let packet = crypto.encrypt(1, &payload);

        assert_eq!(packet[0], UDP_PACKET_TYPE_AUDIO);
        assert_eq!(u16::from_be_bytes(packet[2..4].try_into().unwrap()) as usize, payload.len());
        assert_eq!(u32::from_be_bytes(packet[4..8].try_into().unwrap()), conn_id);
        assert_eq!(&packet[4..12], &nonce[4..12]);

        let (ts, seq, plain) = crypto.decrypt(&packet).unwrap();
        assert_eq!(ts, u32::from_be_bytes(nonce[8..12].try_into().unwrap()));
        assert_eq!(seq, 1);
        assert_eq!(plain, payload);
    }
}
