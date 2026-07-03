//! OpenClaw 管理后台对话测试辅助（对齐 Go `buildOpenClawTestDeviceID`）

pub const OPENCLAW_CHAT_TEST_SESSION_ID: &str = "openclaw-chat-test-global";
pub const OPENCLAW_TEST_DEVICE_PREFIX: &str = "__openclaw_test__:";

pub fn openclaw_test_device_id(agent_id: &str) -> String {
    let trimmed = agent_id.trim();
    if trimmed.is_empty() {
        format!("{OPENCLAW_TEST_DEVICE_PREFIX}unknown")
    } else {
        format!("{OPENCLAW_TEST_DEVICE_PREFIX}{trimmed}")
    }
}

pub fn is_openclaw_test_device(device_id: &str) -> bool {
    device_id.trim().starts_with(OPENCLAW_TEST_DEVICE_PREFIX)
}

pub fn parse_openclaw_timeout_ms(raw: Option<&serde_json::Value>) -> u64 {
    let mut timeout = 10 * 60 * 1000u64;
    if let Some(v) = raw {
        if let Some(n) = v.as_u64() {
            timeout = n;
        } else if let Some(n) = v.as_i64() {
            timeout = n.max(0) as u64;
        } else if let Some(n) = v.as_f64() {
            timeout = n as u64;
        }
    }
    timeout.clamp(1000, 10 * 60 * 1000)
}

pub fn parse_stream_events(raw: Option<&serde_json::Value>) -> bool {
    match raw {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::Number(n)) => n.as_i64().unwrap_or(0) != 0,
        Some(serde_json::Value::String(s)) => {
            matches!(
                s.trim().to_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        }
        _ => false,
    }
}
