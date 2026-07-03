use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::audio::AudioParams;

/// 客户端 → 服务端消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_params: Option<AudioParams>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub udp_config: Option<SpeakReadyUdpConfig>,
    #[serde(flatten)]
    pub extra: Value,
}

/// `speak_ready` 附带的 UDP 状态（对齐 Go `SpeakReadyUDPConfig`）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeakReadyUdpConfig {
    pub ready: bool,
    #[serde(default)]
    pub reuse_existing: bool,
}

/// 服务端 → 客户端消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emotion: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_params: Option<AudioParams>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub udp: Option<UdpConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_listen: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    #[serde(flatten)]
    pub extra: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UdpConfig {
    pub server: String,
    pub port: u16,
    pub key: String,
    pub nonce: String,
}

impl ServerMessage {
    /// 服务端 hello，与 Go 版及 ESP32 `ParseServerHello` 对齐（必须含 `transport`）。
    pub fn hello(session_id: impl Into<String>, audio_params: AudioParams) -> Self {
        Self::hello_with_transport(session_id, audio_params, "websocket")
    }

    pub fn hello_with_transport(
        session_id: impl Into<String>,
        audio_params: AudioParams,
        transport: &str,
    ) -> Self {
        Self {
            msg_type: xiaozhi_core::message::HELLO.to_string(),
            session_id: Some(session_id.into()),
            state: None,
            text: Some("欢迎使用小智服务器".to_string()),
            transport: Some(transport.to_string()),
            version: Some(0),
            emotion: None,
            audio_params: Some(audio_params),
            udp: None,
            auto_listen: None,
            payload: None,
            extra: Value::Null,
        }
    }

    pub fn stt(text: impl Into<String>, session_id: Option<String>) -> Self {
        Self {
            msg_type: xiaozhi_core::message::STT.to_string(),
            session_id,
            state: None,
            text: Some(text.into()),
            transport: None,
            version: None,
            emotion: None,
            audio_params: None,
            udp: None,
            auto_listen: None,
            payload: None,
            extra: Value::Null,
        }
    }

    pub fn llm(text: impl Into<String>, session_id: Option<String>) -> Self {
        Self {
            msg_type: xiaozhi_core::message::LLM.to_string(),
            session_id,
            state: None,
            text: Some(text.into()),
            transport: None,
            version: None,
            emotion: None,
            audio_params: None,
            udp: None,
            auto_listen: None,
            payload: None,
            extra: Value::Null,
        }
    }

    pub fn tts(state: impl Into<String>, session_id: Option<String>) -> Self {
        Self {
            msg_type: xiaozhi_core::message::TTS.to_string(),
            session_id,
            state: Some(state.into()),
            text: None,
            transport: None,
            version: None,
            emotion: None,
            audio_params: None,
            udp: None,
            auto_listen: None,
            payload: None,
            extra: Value::Null,
        }
    }

    pub fn tts_sentence(text: impl Into<String>, state: &str, session_id: Option<String>) -> Self {
        Self {
            msg_type: xiaozhi_core::message::TTS.to_string(),
            session_id,
            state: Some(state.to_string()),
            text: Some(text.into()),
            transport: None,
            version: None,
            emotion: None,
            audio_params: None,
            udp: None,
            auto_listen: None,
            payload: None,
            extra: Value::Null,
        }
    }

    pub fn text(text: impl Into<String>) -> Self {
        Self {
            msg_type: xiaozhi_core::message::TEXT.to_string(),
            session_id: None,
            state: None,
            text: Some(text.into()),
            transport: None,
            version: None,
            emotion: None,
            audio_params: None,
            udp: None,
            auto_listen: None,
            payload: None,
            extra: Value::Null,
        }
    }

    pub fn goodbye(session_id: Option<String>) -> Self {
        Self {
            msg_type: xiaozhi_core::message::GOODBYE.to_string(),
            session_id,
            state: None,
            text: None,
            transport: None,
            version: None,
            emotion: None,
            audio_params: None,
            udp: None,
            auto_listen: None,
            payload: None,
            extra: Value::Null,
        }
    }

    pub fn speak_request(
        text: impl Into<String>,
        session_id: impl Into<String>,
        auto_listen: bool,
    ) -> Self {
        Self {
            msg_type: xiaozhi_core::message::SPEAK_REQUEST.to_string(),
            session_id: Some(session_id.into()),
            state: None,
            text: Some(text.into()),
            transport: None,
            version: None,
            emotion: None,
            audio_params: None,
            udp: None,
            auto_listen: Some(auto_listen),
            payload: None,
            extra: Value::Null,
        }
    }

    pub fn iot_success() -> Self {
        Self {
            msg_type: xiaozhi_core::message::IOT.to_string(),
            session_id: None,
            state: Some(xiaozhi_core::message::SUCCESS.to_string()),
            text: None,
            transport: None,
            version: None,
            emotion: None,
            audio_params: None,
            udp: None,
            auto_listen: None,
            payload: None,
            extra: Value::Null,
        }
    }

    pub fn mcp(session_id: impl Into<String>, payload: Value) -> Self {
        Self {
            msg_type: xiaozhi_core::message::MCP.to_string(),
            session_id: Some(session_id.into()),
            state: None,
            text: None,
            transport: None,
            version: None,
            emotion: None,
            audio_params: None,
            udp: None,
            auto_listen: None,
            payload: Some(payload),
            extra: Value::Null,
        }
    }
}

/// OTA 请求（设备 POST 的 system info JSON 字段较多，解析时仅提取常用字段，其余忽略）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OtaRequest {
    /// 设备上报为数字 `2`，兼容字符串
    #[serde(default)]
    pub version: Option<serde_json::Value>,
    pub mac: Option<String>,
    pub uuid: Option<String>,
    pub board: Option<Value>,
    pub application: Option<Value>,
    pub partition: Option<Value>,
    #[serde(flatten)]
    pub extra: Value,
}

/// OTA 响应（字段与 Go `OtaResponse` / ESP32 `CheckVersion` 对齐，避免下发 `null` 导致旧固件异常）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OtaResponse {
    pub websocket: OtaWebsocket,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mqtt: Option<OtaMqtt>,
    pub server_time: OtaServerTime,
    pub firmware: OtaFirmware,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activation: Option<OtaActivation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OtaWebsocket {
    pub url: String,
    #[serde(default)]
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OtaFirmware {
    pub version: String,
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OtaMqtt {
    pub endpoint: String,
    pub client_id: String,
    pub username: String,
    pub password: String,
    pub publish_topic: String,
    pub subscribe_topic: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OtaServerTime {
    /// UTC 毫秒时间戳
    pub timestamp: i64,
    /// 相对 UTC 的偏移，单位为分钟（UTC+8 → 480），与 Go / 官方 ESP32 协议一致
    pub timezone_offset: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OtaActivation {
    pub code: String,
    pub message: String,
    pub challenge: String,
}
