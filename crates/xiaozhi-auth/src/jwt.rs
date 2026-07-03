use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use xiaozhi_core::{Error, Result};

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub device_id: Option<String>,
    pub exp: usize,
}

pub fn create_token(subject: &str, device_id: Option<&str>, secret: &str, exp_hours: i64) -> Result<String> {
    let exp = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::hours(exp_hours))
        .ok_or_else(|| Error::Auth("过期时间计算失败".into()))?
        .timestamp() as usize;

    let claims = Claims {
        sub: subject.to_string(),
        device_id: device_id.map(String::from),
        exp,
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| Error::Auth(format!("JWT 生成失败: {e}")))
}

pub fn verify_token(token: &str, secret: &str) -> Result<Claims> {
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .map(|d| d.claims)
    .map_err(|e| Error::Auth(format!("JWT 验证失败: {e}")))
}
