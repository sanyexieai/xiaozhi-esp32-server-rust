use jsonwebtoken::{decode, DecodingKey, Validation};
use serde::Deserialize;
use xiaozhi_core::{Error, Result};

#[derive(Debug, Deserialize)]
pub struct OpenClawClaims {
    pub user_id: Option<u64>,
    pub agent_id: String,
    #[serde(default)]
    pub endpoint_id: String,
    #[serde(default)]
    pub purpose: String,
}

pub fn parse_openclaw_token(token: &str, secret: &str) -> Result<OpenClawClaims> {
    let token = token
        .trim()
        .strip_prefix("Bearer ")
        .unwrap_or(token)
        .trim();
    if token.is_empty() {
        return Err(Error::Auth("missing token".into()));
    }
    let mut validation = Validation::default();
    validation.validate_exp = false;
    let data = decode::<OpenClawClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map_err(|e| Error::Auth(format!("invalid token: {e}")))?;
    let claims = data.claims;
    if claims.agent_id.trim().is_empty() {
        return Err(Error::Auth("invalid token: missing agent_id".into()));
    }
    if !claims.purpose.is_empty() && claims.purpose != "openclaw-endpoint" {
        return Err(Error::Auth("invalid token purpose".into()));
    }
    Ok(claims)
}
