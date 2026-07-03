//! voice-server / asr_server 声纹 HTTP 客户端

use reqwest::multipart::{Form, Part};
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use xiaozhi_core::{Error, Result};

use crate::db::{ConfigRow, Database};

#[derive(Debug, Clone)]
pub struct SpeakerServiceConfig {
    pub base_url: String,
    pub threshold: f64,
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct VerifyResult {
    pub verified: bool,
    pub confidence: f32,
    pub threshold: f32,
    #[serde(default)]
    pub speaker_id: String,
    #[serde(default)]
    pub speaker_name: String,
}

pub fn load_speaker_service(db: &Database) -> Result<Option<SpeakerServiceConfig>> {
    let rows = db
        .list_configs("voice_identify")
        .map_err(|e| Error::Config(e.to_string()))?;
    let row = rows
        .iter()
        .find(|r| r.is_default)
        .or_else(|| rows.first());
    let Some(row) = row else {
        return Ok(None);
    };
    Ok(Some(parse_speaker_config_row(row)))
}

pub fn parse_speaker_config_row(row: &ConfigRow) -> SpeakerServiceConfig {
    let v: Value = serde_json::from_str(&row.json_data).unwrap_or_default();
    let enabled = v
        .get("enable")
        .and_then(|x| x.as_bool())
        .unwrap_or(row.enabled);
    let base_url = v
        .pointer("/service/base_url")
        .or_else(|| v.get("base_url"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim_end_matches('/')
        .to_string();
    let threshold = v
        .pointer("/service/threshold")
        .or_else(|| v.get("threshold"))
        .and_then(|x| x.as_f64())
        .unwrap_or(0.4);
    SpeakerServiceConfig {
        base_url,
        threshold,
        enabled,
    }
}

pub struct SpeakerClient {
    cfg: SpeakerServiceConfig,
    client: Client,
}

impl SpeakerClient {
    pub fn new(cfg: SpeakerServiceConfig) -> Self {
        Self {
            cfg,
            client: Client::new(),
        }
    }

    pub fn is_ready(&self) -> bool {
        self.cfg.enabled && !self.cfg.base_url.is_empty()
    }

    pub fn threshold(&self) -> f64 {
        self.cfg.threshold
    }

    pub async fn verify(
        &self,
        speaker_id: &str,
        agent_id: i64,
        user_id: i64,
        audio: Vec<u8>,
        filename: &str,
    ) -> Result<VerifyResult> {
        let url = format!(
            "{}/api/v1/speaker/verify/{}",
            self.cfg.base_url,
            urlencoding_path(speaker_id)
        );
        let part = Part::bytes(audio)
            .file_name(filename.to_string())
            .mime_str("audio/wav")
            .map_err(|e| Error::Http(format!("构建 multipart 失败: {e}")))?;
        let form = Form::new().part("audio", part);
        let resp = self
            .client
            .post(&url)
            .header("X-User-ID", user_id.to_string())
            .header("X-Agent-ID", agent_id.to_string())
            .multipart(form)
            .send()
            .await
            .map_err(|e| Error::Http(format!("声纹验证请求失败: {e}")))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| Error::Http(format!("读取声纹验证响应失败: {e}")))?;
        if !status.is_success() {
            return Err(Error::Http(format!(
                "声纹服务 HTTP {status}: {body}"
            )));
        }
        serde_json::from_str(&body)
            .map_err(|e| Error::Http(format!("解析声纹验证响应失败: {e}")))
    }

    pub async fn register(
        &self,
        speaker_id: &str,
        speaker_name: &str,
        sample_uuid: &str,
        agent_id: i64,
        user_id: i64,
        audio: Vec<u8>,
        filename: &str,
    ) -> Result<()> {
        let url = format!("{}/api/v1/speaker/register", self.cfg.base_url);
        let part = Part::bytes(audio)
            .file_name(filename.to_string())
            .mime_str("audio/wav")
            .map_err(|e| Error::Http(format!("构建 multipart 失败: {e}")))?;
        let form = Form::new()
            .text("speaker_id", speaker_id.to_string())
            .text("speaker_name", speaker_name.to_string())
            .text("uuid", sample_uuid.to_string())
            .text("agent_id", agent_id.to_string())
            .text("uid", user_id.to_string())
            .part("audio", part);
        let resp = self
            .client
            .post(&url)
            .header("X-User-ID", user_id.to_string())
            .header("X-Agent-ID", agent_id.to_string())
            .multipart(form)
            .send()
            .await
            .map_err(|e| Error::Http(format!("声纹注册请求失败: {e}")))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| Error::Http(format!("读取声纹注册响应失败: {e}")))?;
        if !status.is_success() {
            return Err(Error::Http(format!("声纹服务 HTTP {status}: {body}")));
        }
        Ok(())
    }

    pub async fn delete_sample(
        &self,
        speaker_id: &str,
        sample_uuid: &str,
        agent_id: i64,
        user_id: i64,
    ) -> Result<()> {
        let url = format!(
            "{}/api/v1/speaker/{}?uuid={}&agent_id={}",
            self.cfg.base_url,
            urlencoding_path(speaker_id),
            urlencoding_path(sample_uuid),
            agent_id
        );
        let resp = self
            .client
            .delete(&url)
            .header("X-User-ID", user_id.to_string())
            .header("X-Agent-ID", agent_id.to_string())
            .send()
            .await
            .map_err(|e| Error::Http(format!("声纹删除请求失败: {e}")))?;
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            tracing::warn!("声纹样本删除远程失败: {body}");
        }
        Ok(())
    }

    /// 删除声纹组下全部远程样本（路径参数为 speaker_id，不带 uuid）。
    pub async fn delete_group(
        &self,
        speaker_id: &str,
        agent_id: i64,
        user_id: i64,
    ) -> Result<()> {
        let url = format!(
            "{}/api/v1/speaker/{}?agent_id={}",
            self.cfg.base_url,
            urlencoding_path(speaker_id),
            agent_id
        );
        let resp = self
            .client
            .delete(&url)
            .header("X-User-ID", user_id.to_string())
            .header("X-Agent-ID", agent_id.to_string())
            .send()
            .await
            .map_err(|e| Error::Http(format!("声纹组删除请求失败: {e}")))?;
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Http(format!(
                "声纹服务删除声纹组失败: {body}"
            )));
        }
        Ok(())
    }
}

fn urlencoding_path(input: &str) -> String {
    input
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub fn verify_message(verified: bool, confidence: f32) -> String {
    if verified {
        format!("验证通过，相似度: {:.1}%", confidence * 100.0)
    } else {
        format!("验证未通过，相似度: {:.1}%", confidence * 100.0)
    }
}
