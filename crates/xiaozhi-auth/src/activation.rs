use rand::Rng;
use xiaozhi_config::user::ActivationPayload;
use xiaozhi_core::Result;

/// 生成 6 位激活码
pub fn generate_activation_code() -> String {
    let mut rng = rand::thread_rng();
    format!("{:06}", rng.gen_range(0..1_000_000))
}

/// 生成 challenge
pub fn generate_challenge() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn verify_activation_payload(
    payload: &ActivationPayload,
    expected_code: &str,
    challenge: &str,
    secret: &str,
) -> Result<bool> {
    if payload.code != expected_code {
        return Ok(false);
    }
    if payload.challenge != challenge {
        return Ok(false);
    }
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .map_err(|e| xiaozhi_core::Error::Auth(format!("HMAC 失败: {e}")))?;
    mac.update(challenge.as_bytes());
    mac.update(expected_code.as_bytes());
    let expected = hex::encode(mac.finalize().into_bytes());
    Ok(expected == payload.hmac)
}
