//! MQTT Topic 约定（对齐 Go `internal/data/msg/message_types.go`）

use serde::{Deserialize, Serialize};

pub const DEVICE_MOCK_PUB_TOPIC: &str = "device-server";
pub const DEVICE_MOCK_SUB_TOPIC: &str = "null";
pub const DEVICE_SUB_TOPIC_PREFIX: &str = "/p2p/device_sub/";
pub const DEVICE_PUB_TOPIC_PREFIX: &str = "/p2p/device_public/";
pub const LIFECYCLE_TOPIC: &str = "/p2p/device_public/_server/lifecycle";
pub const SERVER_SUB_TOPIC: &str = "/p2p/device_public/#";
pub const SERVER_PUB_TOPIC_PREFIX: &str = DEVICE_SUB_TOPIC_PREFIX;

pub mod lifecycle {
    pub const TYPE: &str = "mqtt_lifecycle";
    pub const ONLINE: &str = "online";
    pub const OFFLINE: &str = "offline";
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttLifecycleEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub device_id: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    pub ts: i64,
}

impl MqttLifecycleEvent {
    pub fn new(state: &str, device_id: &str, client_id: &str) -> Self {
        Self {
            event_type: lifecycle::TYPE.to_string(),
            device_id: device_id.to_string(),
            state: state.to_string(),
            client_id: if client_id.is_empty() {
                None
            } else {
                Some(client_id.to_string())
            },
            ts: chrono::Utc::now().timestamp_millis(),
        }
    }

    pub fn to_payload(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }
}

pub fn mac_underscore_from_device_id(device_id: &str) -> String {
    device_id.replace(':', "_")
}

pub fn device_id_from_mac_underscore(mac: &str) -> String {
    mac.replace('_', ":")
}

pub fn mac_from_client_id(client_id: &str) -> Option<String> {
    let parts: Vec<&str> = client_id.split("@@@").collect();
    if parts.len() >= 3 && !parts[1].is_empty() {
        Some(parts[1].to_string())
    } else {
        None
    }
}

/// 从 Go 风格 MQTT client_id 解析 device_id（冒号 MAC）
pub fn device_id_from_client_id(client_id: &str) -> Option<String> {
    mac_from_client_id(client_id).map(|mac| device_id_from_mac_underscore(&mac))
}

pub fn device_public_topic_mac(mac_underscore: &str) -> String {
    format!("{DEVICE_PUB_TOPIC_PREFIX}{mac_underscore}")
}

pub fn device_sub_topic_mac(mac_underscore: &str) -> String {
    format!("{DEVICE_SUB_TOPIC_PREFIX}{mac_underscore}")
}

pub fn device_public_topic(device_id: &str) -> String {
    device_public_topic_mac(&mac_underscore_from_device_id(device_id))
}

pub fn device_sub_topic(device_id: &str) -> String {
    device_sub_topic_mac(&mac_underscore_from_device_id(device_id))
}

/// 从 `/p2p/device_public/{segment}` 解析 device_id（对齐 Go `getDeviceIdByTopic`）
pub fn device_id_from_public_topic(topic: &str) -> Option<String> {
    let segment = topic.strip_prefix(DEVICE_PUB_TOPIC_PREFIX)?;
    if segment.is_empty() || segment.starts_with("_server/") {
        return None;
    }
    if segment.contains("@@@") {
        let parts: Vec<&str> = segment.split("@@@").collect();
        if parts.len() >= 2 {
            return Some(device_id_from_mac_underscore(parts[1]));
        }
        return None;
    }
    Some(device_id_from_mac_underscore(segment))
}

pub fn lifecycle_payload(state: &str, device_id: &str, client_id: &str) -> Vec<u8> {
    MqttLifecycleEvent::new(state, device_id, client_id).to_payload()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topic_mac_uses_underscores() {
        assert_eq!(
            device_sub_topic("e8:f6:0a:89:b4:0c"),
            "/p2p/device_sub/e8_f6_0a_89_b4_0c"
        );
    }

    #[test]
    fn parse_public_topic_with_gid() {
        let id = device_id_from_public_topic(
            "/p2p/device_public/GID_test@@@e8_f6_0a_89_b4_0c@@@uuid",
        )
        .unwrap();
        assert_eq!(id, "e8:f6:0a:89:b4:0c");
    }
}
