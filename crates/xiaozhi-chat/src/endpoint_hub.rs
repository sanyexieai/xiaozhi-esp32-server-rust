//! 同一逻辑设备的多端连接（Web / 硬件 MQTT+UDP / 工具链）出站路由。

use std::collections::HashMap;
use std::sync::Mutex;

use tokio::sync::mpsc;

use crate::outbound::OutboundFrame;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointKind {
    /// ESP32 等硬件，MQTT 信令 + UDP 音频
    Hardware,
    /// 管理台 Web 模拟器 / 浏览器端
    Web,
    /// MCP WebSocket 等辅助连接
    Tool,
}

impl EndpointKind {
    pub fn supports_audio_playback(self) -> bool {
        matches!(self, Self::Hardware | Self::Web)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hardware => "hardware",
            Self::Web => "web",
            Self::Tool => "tool",
        }
    }
}

#[derive(Debug, Clone)]
pub struct EndpointInfo {
    pub id: String,
    pub kind: EndpointKind,
}

impl TtsAudioRoute {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HardwareFirst => "hardware_first",
            Self::All => "all",
            Self::HardwareOnly => "hardware_only",
            Self::WebOnly => "web_only",
        }
    }
}

/// 解析 API `target` 参数：`web` / `hardware` / `all`，默认 HardwareFirst。
pub fn parse_tts_audio_route(target: &str) -> TtsAudioRoute {
    match target.trim().to_lowercase().as_str() {
        "web" | "web_only" => TtsAudioRoute::WebOnly,
        "hardware" | "hardware_only" => TtsAudioRoute::HardwareOnly,
        "all" => TtsAudioRoute::All,
        _ => TtsAudioRoute::HardwareFirst,
    }
}

#[derive(Debug, Clone)]
pub struct EndpointRegistration {
    pub id: String,
    pub kind: EndpointKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TtsAudioRoute {
    /// 优先硬件端，无硬件则 Web
    #[default]
    HardwareFirst,
    /// 所有支持音频的端点
    All,
    HardwareOnly,
    WebOnly,
}

pub struct EndpointHub {
    endpoints: Mutex<HashMap<String, (EndpointRegistration, mpsc::UnboundedSender<OutboundFrame>)>>,
    tts_audio_route: Mutex<TtsAudioRoute>,
}

impl Default for EndpointHub {
    fn default() -> Self {
        Self::new()
    }
}

impl EndpointHub {
    pub fn new() -> Self {
        Self {
            endpoints: Mutex::new(HashMap::new()),
            tts_audio_route: Mutex::new(TtsAudioRoute::default()),
        }
    }

    pub fn set_tts_audio_route(&self, route: TtsAudioRoute) {
        *self.tts_audio_route.lock().unwrap() = route;
    }

    pub fn tts_audio_route(&self) -> TtsAudioRoute {
        *self.tts_audio_route.lock().unwrap()
    }

    pub fn register(
        &self,
        reg: EndpointRegistration,
        tx: mpsc::UnboundedSender<OutboundFrame>,
    ) {
        self.endpoints
            .lock()
            .unwrap()
            .insert(reg.id.clone(), (reg, tx));
    }

    pub fn unregister(&self, endpoint_id: &str) -> bool {
        self.endpoints.lock().unwrap().remove(endpoint_id);
        self.is_empty()
    }

    pub fn is_empty(&self) -> bool {
        self.endpoints.lock().unwrap().is_empty()
    }

    pub fn endpoint_count(&self) -> usize {
        self.endpoints.lock().unwrap().len()
    }

    pub fn has_hardware(&self) -> bool {
        self.endpoints
            .lock()
            .unwrap()
            .values()
            .any(|(reg, _)| reg.kind == EndpointKind::Hardware)
    }

    pub fn has_web(&self) -> bool {
        self.endpoints
            .lock()
            .unwrap()
            .values()
            .any(|(reg, _)| reg.kind == EndpointKind::Web)
    }

    pub fn list_endpoints(&self) -> Vec<EndpointInfo> {
        self.endpoints
            .lock()
            .unwrap()
            .values()
            .map(|(reg, _)| EndpointInfo {
                id: reg.id.clone(),
                kind: reg.kind,
            })
            .collect()
    }

    /// 兼容旧代码：返回优先硬件端，否则任意一个端点。
    pub fn primary_sender(&self) -> Option<mpsc::UnboundedSender<OutboundFrame>> {
        let guard = self.endpoints.lock().unwrap();
        if let Some((_, tx)) = guard
            .values()
            .find(|(reg, _)| reg.kind == EndpointKind::Hardware)
        {
            return Some(tx.clone());
        }
        guard.values().next().map(|(_, tx)| tx.clone())
    }

    pub fn send_command_all(&self, data: Vec<u8>) -> usize {
        let senders: Vec<_> = self
            .endpoints
            .lock()
            .unwrap()
            .values()
            .map(|(_, tx)| tx.clone())
            .collect();
        let mut sent = 0;
        for tx in senders {
            if tx.send(OutboundFrame::Command(data.clone())).is_ok() {
                sent += 1;
            }
        }
        sent
    }

    pub fn send_command_to(&self, endpoint_id: &str, data: Vec<u8>) -> bool {
        let tx = self
            .endpoints
            .lock()
            .unwrap()
            .get(endpoint_id)
            .map(|(_, tx)| tx.clone());
        tx.is_some_and(|tx| tx.send(OutboundFrame::Command(data)).is_ok())
    }

    pub fn send_audio_routed(&self, data: Vec<u8>) -> usize {
        let route = self.tts_audio_route();
        let targets: Vec<mpsc::UnboundedSender<OutboundFrame>> = {
            let guard = self.endpoints.lock().unwrap();
            match route {
                TtsAudioRoute::All => guard
                    .values()
                    .filter(|(reg, _)| reg.kind.supports_audio_playback())
                    .map(|(_, tx)| tx.clone())
                    .collect(),
                TtsAudioRoute::HardwareOnly => guard
                    .values()
                    .filter(|(reg, _)| reg.kind == EndpointKind::Hardware)
                    .map(|(_, tx)| tx.clone())
                    .collect(),
                TtsAudioRoute::WebOnly => guard
                    .values()
                    .filter(|(reg, _)| reg.kind == EndpointKind::Web)
                    .map(|(_, tx)| tx.clone())
                    .collect(),
                TtsAudioRoute::HardwareFirst => {
                    let hardware: Vec<_> = guard
                        .values()
                        .filter(|(reg, _)| reg.kind == EndpointKind::Hardware)
                        .map(|(_, tx)| tx.clone())
                        .collect();
                    if !hardware.is_empty() {
                        hardware
                    } else {
                        guard
                            .values()
                            .filter(|(reg, _)| reg.kind == EndpointKind::Web)
                            .map(|(_, tx)| tx.clone())
                            .collect()
                    }
                }
            }
        };
        let mut sent = 0;
        for tx in targets {
            if tx.send(OutboundFrame::Audio(data.clone())).is_ok() {
                sent += 1;
            }
        }
        sent
    }

    pub fn clear(&self) {
        self.endpoints.lock().unwrap().clear();
    }
}
