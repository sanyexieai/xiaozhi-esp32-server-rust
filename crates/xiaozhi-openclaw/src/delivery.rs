//! OpenClaw 下行投递（对齐 Go `openclaw.ResponseDelivery`）

#[derive(Debug, Clone, Default)]
pub struct ResponseDelivery {
    pub device_id: String,
    pub correlation_id: String,
    pub session_id: String,
    pub text: String,
    pub is_start: bool,
    pub is_end: bool,
    pub metadata: Option<serde_json::Value>,
}
