use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsRequest {
    pub id: String,
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub body: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsResponse {
    pub id: String,
    pub status: i32,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub body: Value,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
}

impl WsResponse {
    pub fn ok(id: String, body: Value) -> Self {
        Self {
            id,
            status: 200,
            headers: HashMap::new(),
            body,
            error: String::new(),
        }
    }

    pub fn err(id: String, status: i32, message: impl Into<String>) -> Self {
        Self {
            id,
            status,
            headers: HashMap::new(),
            body: Value::Null,
            error: message.into(),
        }
    }
}
