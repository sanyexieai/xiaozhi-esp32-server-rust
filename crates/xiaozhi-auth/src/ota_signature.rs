use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use hmac::{Hmac, Mac};
use serde_json::json;
use sha2::{Digest, Sha256};
use xiaozhi_core::Result;

type HmacSha256 = Hmac<Sha256>;

/// Go `util.GenerateMqttCredentials` 生成的 OTA MQTT 凭据
#[derive(Debug, Clone)]
pub struct GoMqttCredentials {
    pub client_id: String,
    pub username: String,
    pub password: String,
}

/// 生成 OTA MQTT 签名 password（旧版 hex 格式，仅用于简单校验）
pub fn generate_mqtt_password(device_id: &str, signature_key: &str) -> Result<String> {
    let mut mac = HmacSha256::new_from_slice(signature_key.as_bytes())
        .map_err(|e| xiaozhi_core::Error::Auth(format!("签名密钥无效: {e}")))?;
    mac.update(device_id.as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}

/// 验证 MQTT 连接凭据（旧版 hex 格式）
pub fn verify_mqtt_credentials(
    device_id: &str,
    password: &str,
    signature_key: &str,
) -> Result<bool> {
    let expected = generate_mqtt_password(device_id, signature_key)?;
    Ok(expected == password)
}

/// 与 Go `GeneratePasswordSignature` 一致：HMAC-SHA256 后 base64
pub fn generate_password_signature_base64(data: &str, signature_key: &str) -> Result<String> {
    let mut mac = HmacSha256::new_from_slice(signature_key.as_bytes())
        .map_err(|e| xiaozhi_core::Error::Auth(format!("签名密钥无效: {e}")))?;
    mac.update(data.as_bytes());
    Ok(BASE64.encode(mac.finalize().into_bytes()))
}

/// 与 Go `GenerateMqttCredentials` 对齐，供 OTA 与内置 MQTT Broker 使用
pub fn generate_go_mqtt_credentials(
    device_id: &str,
    client_id: &str,
    client_ip: &str,
    signature_key: &str,
) -> Result<GoMqttCredentials> {
    let device_id_norm = device_id.replace(':', "_");
    let username_json = json!({ "ip": client_ip }).to_string();
    let username = BASE64.encode(username_json.as_bytes());
    let mqtt_client_id = format!("GID_test@@@{device_id_norm}@@@{client_id}");

    let password = if signature_key.is_empty() {
        hex::encode(Sha256::digest(mqtt_client_id.as_bytes()))
    } else {
        let signature_data = format!("{mqtt_client_id}|{username}");
        generate_password_signature_base64(&signature_data, signature_key)?
    };

    Ok(GoMqttCredentials {
        client_id: mqtt_client_id,
        username,
        password,
    })
}

/// 与 Go `ValidateMqttCredentials` 对齐
pub fn verify_go_mqtt_credentials(
    client_id: &str,
    username: &str,
    password: &str,
    signature_key: &str,
) -> Result<bool> {
    if signature_key.is_empty() {
        return Ok(false);
    }
    if client_id.is_empty() || username.is_empty() {
        return Ok(false);
    }
    let parts: Vec<&str> = client_id.split("@@@").collect();
    if parts.len() != 3 {
        return Ok(false);
    }
    let decoded = BASE64
        .decode(username.as_bytes())
        .map_err(|e| xiaozhi_core::Error::Auth(format!("username base64 无效: {e}")))?;
    serde_json::from_slice::<serde_json::Value>(&decoded)
        .map_err(|e| xiaozhi_core::Error::Auth(format!("username JSON 无效: {e}")))?;
    let signature_data = format!("{client_id}|{username}");
    let expected = generate_password_signature_base64(&signature_data, signature_key)?;
    Ok(password == expected)
}

const MQTT_AES_KEY: &[u8; 16] = b"xiaozhi_aes_key_";

/// Go `validateWithAes` / `checkAesPassword` 回退鉴权
pub fn verify_mqtt_aes_credentials(username: &str, password: &str) -> Result<bool> {
    let decoded = BASE64
        .decode(username.as_bytes())
        .map_err(|e| xiaozhi_core::Error::Auth(format!("username base64 无效: {e}")))?;
    let user_info: serde_json::Value = serde_json::from_slice(&decoded)
        .map_err(|e| xiaozhi_core::Error::Auth(format!("username JSON 无效: {e}")))?;
    if user_info.get("ip").is_none() {
        return Ok(false);
    }
    Ok(aes_ecb_password_for_username(username)? == password)
}

fn aes_ecb_password_for_username(username: &str) -> Result<String> {
    use aes::cipher::{BlockEncrypt, KeyInit};
    use aes::Aes128;
    use aes::Block;

    let cipher = Aes128::new_from_slice(MQTT_AES_KEY)
        .map_err(|e| xiaozhi_core::Error::Auth(format!("AES 密钥无效: {e}")))?;
    let block_size = 16;
    let padding = block_size - username.len() % block_size;
    let mut data = username.as_bytes().to_vec();
    data.extend(std::iter::repeat_n(padding as u8, padding));
    let mut out = vec![0u8; data.len()];
    for (chunk_in, chunk_out) in data.chunks(16).zip(out.chunks_mut(16)) {
        let mut block = Block::clone_from_slice(chunk_in);
        cipher.encrypt_block(&mut block);
        chunk_out.copy_from_slice(block.as_slice());
    }
    Ok(BASE64.encode(out))
}

/// 内置 Broker CONNECT 鉴权（对齐 Go `AuthHook.OnConnectAuthenticate`）
pub fn verify_mqtt_broker_connect(
    client_id: &str,
    username: &str,
    password: &str,
    enable_auth: bool,
    signature_key: &str,
    admin_username: &str,
    admin_password: &str,
) -> Result<bool> {
    if !enable_auth {
        return Ok(true);
    }
    if username == admin_username && password == admin_password {
        return Ok(true);
    }
    if username == admin_username {
        return Ok(false);
    }
    if !signature_key.is_empty() {
        return verify_go_mqtt_credentials(client_id, username, password, signature_key);
    }
    verify_mqtt_aes_credentials(username, password)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_mqtt_password_matches_hmac_base64() {
        let creds = generate_go_mqtt_credentials(
            "e8:f6:0a:89:b4:0c",
            "test-client-id",
            "192.168.3.3",
            "test_key",
        )
        .unwrap();
        assert!(creds.client_id.starts_with("GID_test@@@"));
        assert!(verify_go_mqtt_credentials(
            &creds.client_id,
            &creds.username,
            &creds.password,
            "test_key"
        )
        .unwrap());
    }
}
