use jsonwebtoken::{encode, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use xiaozhi_core::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagerWsClaims {
    pub purpose: String,
    pub uuid: String,
    pub exp: usize,
}

pub fn create_manager_ws_token(secret: &str, uuid: &str, ttl_secs: i64) -> Result<String> {
    let exp = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::seconds(ttl_secs))
        .ok_or_else(|| Error::Auth("过期时间计算失败".into()))?
        .timestamp() as usize;

    let claims = ManagerWsClaims {
        purpose: "manager-ws-client".into(),
        uuid: uuid.to_string(),
        exp,
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| Error::Auth(format!("Manager WS JWT 生成失败: {e}")))
}
