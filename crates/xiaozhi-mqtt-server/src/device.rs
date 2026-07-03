//! 设备 ACL 与 topic 重写（对齐 Go `device_hook.go`）

use xiaozhi_protocol::mqtt;

pub fn is_admin_user(username: &str, admin_username: &str) -> bool {
    username == admin_username
}

pub fn acl_allow_subscribe(client_id: &str, topic: &str, is_admin: bool) -> bool {
    if is_admin {
        return true;
    }
    let Some(mac) = mqtt::mac_from_client_id(client_id) else {
        return false;
    };
    topic == mqtt::device_sub_topic_mac(&mac)
}

pub fn acl_allow_publish(client_id: &str, topic: &str, is_admin: bool) -> bool {
    if is_admin {
        return true;
    }
    if topic == mqtt::DEVICE_MOCK_PUB_TOPIC {
        return true;
    }
    // RMQTT 可能在 MessagePublish 重写 topic 之后才执行 ACL，需同时允许 device_public
    if let Some(mac) = mqtt::mac_from_client_id(client_id) {
        return topic == mqtt::device_public_topic_mac(&mac);
    }
    false
}

pub fn rewrite_device_publish_topic(client_id: &str, topic: &str, is_admin: bool) -> String {
    if is_admin {
        return topic.to_string();
    }
    if topic != mqtt::DEVICE_MOCK_PUB_TOPIC {
        return topic.to_string();
    }
    let Some(mac) = mqtt::mac_from_client_id(client_id) else {
        return topic.to_string();
    };
    mqtt::device_public_topic_mac(&mac)
}

pub fn auto_subscribe_topic(client_id: &str) -> Option<String> {
    let mac = mqtt::mac_from_client_id(client_id)?;
    Some(mqtt::device_sub_topic_mac(&mac))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrite_device_server_topic() {
        let client_id = "GID_test@@@e8_f6_0a_89_b4_0c@@@uuid";
        assert_eq!(
            rewrite_device_publish_topic(client_id, mqtt::DEVICE_MOCK_PUB_TOPIC, false),
            "/p2p/device_public/e8_f6_0a_89_b4_0c"
        );
    }

    #[test]
    fn acl_allows_rewritten_public_topic() {
        let client_id = "GID_test@@@ota-test-device@@@ota-test-client";
        assert!(acl_allow_publish(
            client_id,
            "/p2p/device_public/ota-test-device",
            false
        ));
    }
}
