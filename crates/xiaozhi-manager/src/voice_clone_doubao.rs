//! 豆包声音复刻（对齐 Go `doubao_voice_clone.go`）

use std::time::Duration;

use reqwest::multipart::{Form, Part};
use serde_json::{json, Value};
use tokio::fs;
use tokio::time::{sleep, Instant};

const DEFAULT_DOUBAO_CLONE_UPLOAD_ENDPOINT: &str =
    "https://openspeech.bytedance.com/api/v1/mega_tts/audio/upload";
const DEFAULT_DOUBAO_CLONE_STATUS_ENDPOINT: &str =
    "https://openspeech.bytedance.com/api/v1/mega_tts/status";
const MODEL_SEED_ICL_10: &str = "seed-icl-1.0";
const MODEL_SEED_ICL_20_STANDARD: &str = "seed-icl-2.0-standard";
const MODEL_SEED_ICL_20_EXPR: &str = "seed-icl-2.0-expressive";
const MODEL_SEED_TTS_20_STANDARD: &str = "seed-tts-2.0-standard";
const MODEL_SEED_TTS_20_EXPR: &str = "seed-tts-2.0-expressive";
const MODEL_SEED_TTS_11: &str = "seed-tts-1.1";
const RESOURCE_SEED_ICL_10: &str = "seed-icl-1.0";
const RESOURCE_SEED_ICL_20: &str = "seed-icl-2.0";
const RESOURCE_SEED_TTS_10: &str = "seed-tts-1.0";
const RESOURCE_SEED_TTS_20: &str = "seed-tts-2.0";
const POLL_INTERVAL: Duration = Duration::from_secs(4);
const POLL_TIMEOUT: Duration = Duration::from_secs(300);
const HTTP_TIMEOUT: Duration = Duration::from_secs(120);

struct DoubaoModelSelection {
    resource_id: String,
}

pub async fn clone_with_doubao(
    cfg: &Value,
    file_path: &str,
    file_name: &str,
    transcript: &str,
) -> Result<String, String> {
    let app_id = str_field(cfg, "appid")?;
    let access_token = str_field(cfg, "access_token")?;
    let model = optional_str(cfg, "model").unwrap_or_default();
    let (model_type, target_model) = resolve_doubao_clone_target_model(&model);
    let resource_id = resolve_doubao_model_selection(&target_model, "").resource_id;
    let upload_url = optional_str(cfg, "clone_upload_url")
        .unwrap_or_else(|| DEFAULT_DOUBAO_CLONE_UPLOAD_ENDPOINT.to_string());
    let status_url = optional_str(cfg, "clone_status_url")
        .unwrap_or_else(|| DEFAULT_DOUBAO_CLONE_STATUS_ENDPOINT.to_string());

    let file_bytes = fs::read(file_path)
        .await
        .map_err(|e| format!("读取豆包复刻音频失败: {e}"))?;
    let part = Part::bytes(file_bytes)
        .file_name(file_name.to_string())
        .mime_str("application/octet-stream")
        .map_err(|e| e.to_string())?;
    let mut form = Form::new()
        .text("appid", app_id.clone())
        .text("language", "zh")
        .text("model_type", model_type.to_string())
        .part("file", part);
    let transcript = transcript.trim();
    if !transcript.is_empty() {
        form = form.text("demo_text", transcript.to_string());
    }

    let client = http_client()?;
    let resp = client
        .post(&upload_url)
        .header("Authorization", format!("Bearer;{access_token}"))
        .header("X-Api-App-Id", &app_id)
        .header("X-Api-Access-Key", &access_token)
        .header("X-Api-Resource-Id", &resource_id)
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("调用豆包复刻上传接口失败: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("豆包复刻上传 HTTP {status}: {text}"));
    }
    let parsed: Value = serde_json::from_str(&text).unwrap_or(json!({}));
    if let Some(code) = parsed
        .get("base_resp")
        .and_then(|b| b.get("status_code"))
        .and_then(|c| c.as_i64())
    {
        if code != 0 {
            let msg = parsed
                .get("base_resp")
                .and_then(|b| b.get("status_msg"))
                .and_then(|m| m.as_str())
                .unwrap_or("");
            return Err(format!("豆包复刻上传失败(code={code}, msg={msg})"));
        }
    }
    let speaker_id = first_non_empty(&[
        pick_string(&parsed, &["icl_speaker_id"]),
        pick_string(&parsed, &["speaker_id"]),
    ])
    .ok_or_else(|| "豆包复刻上传成功但未返回 speaker_id".to_string())?;

    poll_doubao_clone_status(
        &client,
        &status_url,
        &app_id,
        &access_token,
        &resource_id,
        &speaker_id,
    )
    .await
}

async fn poll_doubao_clone_status(
    client: &reqwest::Client,
    status_url: &str,
    app_id: &str,
    access_token: &str,
    resource_id: &str,
    speaker_id: &str,
) -> Result<String, String> {
    let started = Instant::now();
    loop {
        let (status_resp, raw) = fetch_doubao_clone_status(
            client,
            status_url,
            app_id,
            access_token,
            resource_id,
            speaker_id,
        )
        .await?;
        if is_doubao_clone_success(&status_resp) {
            return Ok(
                first_non_empty(&[
                    pick_string(&status_resp, &["icl_speaker_id"]),
                    pick_string(&raw, &["icl_speaker_id"]),
                    pick_string(&raw, &["speaker"]),
                    pick_string(&status_resp, &["speaker_id"]),
                    Some(speaker_id.to_string()),
                ])
                .unwrap_or_else(|| speaker_id.to_string()),
            );
        }
        if is_doubao_clone_failed(&status_resp) {
            let msg = pick_string(&status_resp, &["status_msg"])
                .or_else(|| pick_string(&raw, &["message"]))
                .or_else(|| pick_string(&raw, &["error"]))
                .unwrap_or_else(|| "豆包复刻训练失败".to_string());
            return Err(msg);
        }
        if started.elapsed() >= POLL_TIMEOUT {
            return Err("等待豆包复刻结果超时".to_string());
        }
        sleep(POLL_INTERVAL).await;
    }
}

async fn fetch_doubao_clone_status(
    client: &reqwest::Client,
    status_url: &str,
    app_id: &str,
    access_token: &str,
    resource_id: &str,
    speaker_id: &str,
) -> Result<(Value, Value), String> {
    let body = json!({
        "appid": app_id,
        "speaker_id": speaker_id,
    });
    let resp = client
        .post(status_url)
        .header("Authorization", format!("Bearer;{access_token}"))
        .header("X-Api-App-Id", app_id)
        .header("X-Api-Access-Key", access_token)
        .header("X-Api-Resource-Id", resource_id)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("调用豆包复刻状态接口失败: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("豆包复刻状态 HTTP {status}: {text}"));
    }
    let parsed: Value = serde_json::from_str(&text).unwrap_or(json!({}));
    if let Some(code) = parsed
        .get("base_resp")
        .and_then(|b| b.get("status_code"))
        .and_then(|c| c.as_i64())
    {
        if code != 0 {
            let msg = parsed
                .get("base_resp")
                .and_then(|b| b.get("status_msg"))
                .and_then(|m| m.as_str())
                .unwrap_or("");
            return Err(format!("豆包复刻状态查询失败(code={code}, msg={msg})"));
        }
    }
    Ok((parsed.clone(), parsed))
}

fn is_doubao_clone_success(resp: &Value) -> bool {
    for key in ["train_status", "status"] {
        if let Some(status) = resp.get(key).and_then(|v| v.as_str()) {
            match status.trim().to_lowercase().as_str() {
                "9" | "success" | "succeeded" | "done" | "completed" | "finish" | "finished" => {
                    return true;
                }
                _ => {}
            }
        }
    }
    false
}

fn is_doubao_clone_failed(resp: &Value) -> bool {
    for key in ["train_status", "status"] {
        if let Some(status) = resp.get(key).and_then(|v| v.as_str()) {
            match status.trim().to_lowercase().as_str() {
                "-1" | "0" | "failed" | "error" | "rejected" => return true,
                _ => {}
            }
        }
    }
    false
}

fn resolve_doubao_clone_target_model(model: &str) -> (i32, String) {
    match normalize_doubao_model(model).as_str() {
        MODEL_SEED_TTS_20_STANDARD | MODEL_SEED_ICL_20_STANDARD => {
            (4, MODEL_SEED_ICL_20_STANDARD.to_string())
        }
        MODEL_SEED_TTS_20_EXPR | MODEL_SEED_ICL_20_EXPR => (4, MODEL_SEED_ICL_20_EXPR.to_string()),
        _ => (1, MODEL_SEED_ICL_10.to_string()),
    }
}

fn resolve_doubao_model_selection(model: &str, voice: &str) -> DoubaoModelSelection {
    let mut normalized = normalize_doubao_model(model);
    if infer_doubao_voice_family(voice) == "tts2" && normalized == MODEL_SEED_TTS_11 {
        normalized = MODEL_SEED_TTS_20_STANDARD.to_string();
    }
    if normalized.is_empty() {
        let resource_id = match infer_doubao_voice_family(voice) {
            "icl2" => RESOURCE_SEED_ICL_20,
            "icl1" => RESOURCE_SEED_ICL_10,
            "tts2" => RESOURCE_SEED_TTS_20,
            _ => RESOURCE_SEED_TTS_10,
        };
        return DoubaoModelSelection {
            resource_id: resource_id.to_string(),
        };
    }
    let resource_id = match normalized.as_str() {
        MODEL_SEED_TTS_11 => {
            if infer_doubao_voice_family(voice) == "tts2" {
                RESOURCE_SEED_TTS_20
            } else {
                RESOURCE_SEED_TTS_10
            }
        }
        MODEL_SEED_TTS_20_STANDARD | MODEL_SEED_TTS_20_EXPR => RESOURCE_SEED_TTS_20,
        MODEL_SEED_ICL_10 => RESOURCE_SEED_ICL_10,
        MODEL_SEED_ICL_20_STANDARD | MODEL_SEED_ICL_20_EXPR => RESOURCE_SEED_ICL_20,
        _ => RESOURCE_SEED_TTS_10,
    };
    DoubaoModelSelection {
        resource_id: resource_id.to_string(),
    }
}

fn normalize_doubao_model(model: &str) -> String {
    match model.trim().to_ascii_lowercase().as_str() {
        "" | "default" => String::new(),
        m if m == MODEL_SEED_TTS_11 => MODEL_SEED_TTS_11.to_string(),
        m if m == MODEL_SEED_TTS_20_STANDARD || m == "seed-tts-2.0" => {
            MODEL_SEED_TTS_20_STANDARD.to_string()
        }
        m if m == MODEL_SEED_TTS_20_EXPR => MODEL_SEED_TTS_20_EXPR.to_string(),
        m if m == MODEL_SEED_ICL_10 => MODEL_SEED_ICL_10.to_string(),
        m if m == MODEL_SEED_ICL_20_STANDARD => MODEL_SEED_ICL_20_STANDARD.to_string(),
        m if m == MODEL_SEED_ICL_20_EXPR => MODEL_SEED_ICL_20_EXPR.to_string(),
        other => other.to_string(),
    }
}

fn infer_doubao_voice_family(voice: &str) -> &'static str {
    let voice = voice.trim().to_ascii_lowercase();
    if voice.is_empty() {
        return "unknown";
    }
    if voice.starts_with("saturn_") || voice.contains("_bigtts") {
        return "tts2";
    }
    if voice.starts_with("s_") || voice.starts_with("icl_") {
        return "icl1";
    }
    "tts1"
}

fn first_non_empty(values: &[Option<String>]) -> Option<String> {
    values.iter().find_map(|v| v.clone().filter(|s| !s.trim().is_empty()))
}

fn pick_string(v: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(s) = v.get(*key).and_then(|x| x.as_str()) {
            let s = s.trim();
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn str_field(v: &Value, key: &str) -> Result<String, String> {
    let s = v
        .get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if s.is_empty() {
        return Err(format!("{key} 不能为空"));
    }
    Ok(s)
}

fn optional_str(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_clone_target_model() {
        let (ty, model) = resolve_doubao_clone_target_model("seed-icl-1.0");
        assert_eq!(ty, 1);
        assert_eq!(model, MODEL_SEED_ICL_10);
        let (ty, model) = resolve_doubao_clone_target_model("seed-tts-2.0-standard");
        assert_eq!(ty, 4);
        assert_eq!(model, MODEL_SEED_ICL_20_STANDARD);
    }
}
