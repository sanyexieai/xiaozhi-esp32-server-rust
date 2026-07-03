/// 发往设备的出站帧：JSON 控制消息或二进制音频
#[derive(Debug, Clone)]
pub enum OutboundFrame {
    Command(Vec<u8>),
    Audio(Vec<u8>),
}

/// TTS 播报交付：控制消息 + 二进制音频帧（TTS 音频现由 TtsManager 发送）
#[derive(Debug, Default)]
pub struct SpeakDelivery {
    pub messages: Vec<xiaozhi_protocol::messages::ServerMessage>,
    pub audio_frames: Vec<Vec<u8>>,
}
