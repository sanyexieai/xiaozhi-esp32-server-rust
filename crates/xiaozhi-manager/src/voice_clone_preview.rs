//! 复刻音色试听（对齐 Go `PreviewClonedVoice`）

use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};

const VOICE_CLONE_PREVIEW_TEXT: &str = "我是一个有趣的人，一个脱离低级趣味的人";
const DEFAULT_DOUBAO_PREVIEW_ENDPOINT: &str =
    "https://openspeech.bytedance.com/api/v3/tts/unidirectional";
const DEFAULT_ALIYUN_QWEN_TTS_ENDPOINT: &str =
    "https://dashscope.aliyuncs.com/api/v1/services/aigc/multimodal-generation/generation";
const DEFAULT_ALIYUN_QWEN_TTS_ENDPOINT_INTL: &str =
    "https://dashscope-intl.aliyuncs.com/api/v1/services/aigc/multimodal-generation/generation";
const DEFAULT_ALIYUN_QWEN_CLONE_TARGET_MODEL: &str = "qwen3-tts-vc-2026-01-22";
const COSYVOICE_TTS_ENDPOINT: &str = "https://tts.linkerai.cn/tts";
const INDEX_TTS_SPEECH_ENDPOINT: &str = "/audio/speech";
const MINIMAX_TTS_WS_ENDPOINT: &str = "wss://api.minimaxi.com/ws/v1/t2a_v2";
const DEFAULT_INDEX_TTS_BASE_URL: &str = "http://127.0.0.1:7860";
const HTTP_TIMEOUT: Duration = Duration::from_secs(90);

pub fn preview_text() -> &'static str {
    VOICE_CLONE_PREVIEW_TEXT
}

pub async fn preview_cloned_voice(
    provider: &str,
    cfg: &Value,
    voice_id: &str,
) -> Result<(Vec<u8>, String), String> {
    let text = VOICE_CLONE_PREVIEW_TEXT;
    match provider.trim().to_lowercase().as_str() {
        "doubao" => preview_doubao(cfg, voice_id, text).await,
        "minimax" => preview_minimax(cfg, voice_id, text).await,
        "cosyvoice" => preview_cosyvoice(cfg, voice_id, text).await,
        "aliyun_qwen" => preview_aliyun_qwen(cfg, voice_id, text).await,
        "indextts_vllm" => preview_indextts(cfg, voice_id, text).await,
        other => Err(format!("当前提供商不支持复刻试听: {other}")),
    }
}

async fn preview_doubao(cfg: &Value, voice_id: &str, text: &str) -> Result<(Vec<u8>, String), String> {
    let app_id = str_field(cfg, "appid")?;
    let access_token = str_field(cfg, "access_token")?;
    let selection = resolve_doubao_preview_selection(optional_str(cfg, "model").as_deref(), voice_id);
    let endpoint = optional_str(cfg, "api_url")
        .unwrap_or_else(|| DEFAULT_DOUBAO_PREVIEW_ENDPOINT.to_string());

    let mut body = json!({
        "user": { "uid": random_digits(12) },
        "req_params": {
            "text": text,
            "speaker": voice_id.trim(),
            "audio_params": {
                "format": "mp3",
                "sample_rate": 24000
            }
        }
    });
    if !selection.request_model.is_empty() {
        body["req_params"]["model"] = json!(selection.request_model);
    }

    let client = http_client()?;
    let resp = client
        .post(&endpoint)
        .header("Authorization", format!("Bearer;{access_token}"))
        .header("X-Api-App-Id", &app_id)
        .header("X-Api-Access-Key", &access_token)
        .header("X-Api-Resource-Id", &selection.resource_id)
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("调用豆包试听失败: {e}"))?;
    let status = resp.status();
    let text_body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("豆包试听 HTTP {status}: {text_body}"));
    }

    let mut merged = Vec::new();
    for line in text_body.lines() {
        let mut line = line.trim();
        if line.is_empty() || line.starts_with("event:") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("data:") {
            line = rest.trim();
        }
        if line.is_empty() || line == "[DONE]" {
            continue;
        }
        let event: Value = serde_json::from_str(line)
            .map_err(|e| format!("解析豆包试听事件失败: {e}"))?;
        if let Some(code) = event.get("code").and_then(|c| c.as_i64()) {
            if code != 0 {
                let msg = event
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("");
                return Err(format!("豆包试听失败(code={code}, msg={msg})"));
            }
        }
        if let Some(data) = event.get("data").and_then(|d| d.as_str()) {
            let data = data.trim();
            if !data.is_empty() {
                let chunk = BASE64
                    .decode(data)
                    .map_err(|e| format!("解码豆包试听音频失败: {e}"))?;
                merged.extend_from_slice(&chunk);
            }
        }
    }
    if merged.is_empty() {
        return Err("豆包试听返回音频为空".to_string());
    }
    Ok((merged, "audio/mpeg".to_string()))
}

async fn preview_minimax(cfg: &Value, voice_id: &str, text: &str) -> Result<(Vec<u8>, String), String> {
    let api_key = str_field(cfg, "api_key")?;
    let model = optional_str(cfg, "model").unwrap_or_else(|| "speech-2.8-hd".to_string());
    let speed = float_field(cfg, "speed").unwrap_or(1.0).max(0.1);
    let vol = float_field(cfg, "vol")
        .or_else(|| float_field(cfg, "volume"))
        .unwrap_or(1.0)
        .max(0.1);
    let pitch = int_field(cfg, "pitch").unwrap_or(0);
    let group_id = optional_str_any(cfg, &["group_id", "GroupId"]);

    let mut request = MINIMAX_TTS_WS_ENDPOINT
        .into_client_request()
        .map_err(|e| e.to_string())?;
    request
        .headers_mut()
        .insert("Authorization", format!("Bearer {api_key}").parse().unwrap());
    if let Some(ref gid) = group_id {
        request
            .headers_mut()
            .insert("Group-Id", gid.parse().unwrap());
        request
            .headers_mut()
            .insert("GroupId", gid.parse().unwrap());
    }

    let (mut ws, _) = connect_async(request)
        .await
        .map_err(|e| format!("连接 Minimax 语音接口失败: {e}"))?;

    let start = json!({
        "event": "task_start",
        "model": model,
        "voice_setting": {
            "voice_id": voice_id,
            "speed": speed,
            "vol": vol,
            "pitch": pitch,
            "english_normalization": false
        },
        "audio_setting": {
            "sample_rate": 32000,
            "bitrate": 128000,
            "format": "mp3",
            "channel": 1
        },
        "continuous_sound": false
    });
    ws.send(Message::Text(start.to_string().into()))
        .await
        .map_err(|e| format!("发送 Minimax task_start 失败: {e}"))?;
    ws.send(Message::Text(
        json!({"event": "task_continue", "text": text}).to_string().into(),
    ))
    .await
    .map_err(|e| format!("发送 Minimax task_continue 失败: {e}"))?;
    ws.send(Message::Text(json!({"event": "task_finish"}).to_string().into()))
        .await
        .map_err(|e| format!("发送 Minimax task_finish 失败: {e}"))?;

    let mut merged = Vec::new();
    while let Some(msg) = ws.next().await {
        let msg = msg.map_err(|e| format!("读取 Minimax 响应失败: {e}"))?;
        let text = match msg {
            Message::Text(t) => t.to_string(),
            Message::Binary(b) => String::from_utf8_lossy(&b).to_string(),
            Message::Close(_) => break,
            _ => continue,
        };
        let resp: Value = serde_json::from_str(&text).unwrap_or(json!({}));
        if let Some(code) = resp
            .get("base_resp")
            .and_then(|b| b.get("status_code"))
            .and_then(|c| c.as_i64())
        {
            if code != 0 {
                let msg = resp
                    .get("base_resp")
                    .and_then(|b| b.get("status_msg"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("");
                return Err(format!("Minimax 返回错误(code={code}, msg={msg})"));
            }
        }
        if let Some(audio_hex) = resp
            .get("data")
            .and_then(|d| d.get("audio"))
            .and_then(|a| a.as_str())
        {
            let audio_hex = audio_hex.trim();
            if !audio_hex.is_empty() {
                let chunk = hex::decode(audio_hex)
                    .map_err(|e| format!("解析 Minimax 音频数据失败: {e}"))?;
                merged.extend_from_slice(&chunk);
            }
        }
        let is_final = resp.get("is_final").and_then(|v| v.as_bool()).unwrap_or(false);
        let event = resp
            .get("event")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .eq_ignore_ascii_case("task_finish");
        if is_final || event {
            break;
        }
    }
    if merged.is_empty() {
        return Err("Minimax 返回音频为空".to_string());
    }
    Ok((merged, "audio/mpeg".to_string()))
}

async fn preview_cosyvoice(
    cfg: &Value,
    voice_id: &str,
    text: &str,
) -> Result<(Vec<u8>, String), String> {
    let endpoint = optional_str_any(cfg, &["api_url", "tts_endpoint"])
        .unwrap_or_else(|| COSYVOICE_TTS_ENDPOINT.to_string());
    let mut url = url::Url::parse(&endpoint).map_err(|e| e.to_string())?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("tts_text", text);
        query.append_pair("spk_id", voice_id.trim());
        query.append_pair("frame_durition", "60");
        query.append_pair("stream", "true");
        query.append_pair("target_sr", "24000");
        query.append_pair("audio_format", "mp3");
        if let Some(instruct) = optional_str(cfg, "instruct_text") {
            query.append_pair("instruct_text", &instruct);
        }
    }

    let resp = http_client()?
        .get(url)
        .header("Accept", "audio/mpeg,application/octet-stream,*/*")
        .send()
        .await
        .map_err(|e| format!("调用 CosyVoice 试听失败: {e}"))?;
    let status = resp.status();
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!(
            "CosyVoice 试听 HTTP {status}: {}",
            String::from_utf8_lossy(&bytes)
        ));
    }
    if bytes.is_empty() {
        return Err("CosyVoice 返回音频为空".to_string());
    }
    let content_type = "audio/mpeg".to_string();
    Ok((bytes.to_vec(), content_type))
}

async fn preview_aliyun_qwen(
    cfg: &Value,
    voice_id: &str,
    text: &str,
) -> Result<(Vec<u8>, String), String> {
    let api_key = str_field(cfg, "api_key")?;
    let endpoint = resolve_aliyun_qwen_tts_endpoint(cfg);
    let language_type = optional_str(cfg, "language_type").unwrap_or_else(|| "Chinese".to_string());
    let body = json!({
        "model": DEFAULT_ALIYUN_QWEN_CLONE_TARGET_MODEL,
        "input": {
            "text": text,
            "voice": voice_id.trim(),
            "language_type": language_type
        }
    });

    let client = http_client()?;
    let resp = client
        .post(endpoint)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("调用千问试听失败: {e}"))?;
    let status = resp.status();
    let text_body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("千问试听 HTTP {status}: {text_body}"));
    }
    let parsed: Value = serde_json::from_str(&text_body).unwrap_or(json!({}));
    if let Some(code) = pick_string(&parsed, &["code"]) {
        let msg = pick_string(&parsed, &["message"]).unwrap_or_default();
        return Err(format!("千问试听失败(code={code}, msg={msg})"));
    }
    if let Some(status_code) = parsed.get("status_code").and_then(|c| c.as_i64()) {
        if status_code != 200 {
            return Err(format!("千问试听失败(status_code={status_code})"));
        }
    }
    let audio_url = parsed
        .get("output")
        .and_then(|o| o.get("audio"))
        .and_then(|a| a.get("url"))
        .and_then(|u| u.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "千问试听响应缺少 output.audio.url".to_string())?;

    let audio_resp = client
        .get(audio_url)
        .send()
        .await
        .map_err(|e| format!("下载千问试听音频失败: {e}"))?;
    let audio_status = audio_resp.status();
    let bytes = audio_resp.bytes().await.map_err(|e| e.to_string())?;
    if !audio_status.is_success() {
        return Err(format!(
            "下载千问试听音频 HTTP {audio_status}: {}",
            String::from_utf8_lossy(&bytes)
        ));
    }
    if bytes.is_empty() {
        return Err("千问试听返回音频为空".to_string());
    }
    Ok((bytes.to_vec(), "audio/wav".to_string()))
}

async fn preview_indextts(
    cfg: &Value,
    voice_id: &str,
    text: &str,
) -> Result<(Vec<u8>, String), String> {
    let base_url = normalize_indextts_base_url(optional_str(cfg, "api_url").as_deref());
    let mut body = json!({
        "input": text,
        "voice": voice_id.trim()
    });
    if let Some(model) = optional_str(cfg, "model") {
        body["model"] = json!(model);
    }
    let url = format!("{base_url}{INDEX_TTS_SPEECH_ENDPOINT}");
    let mut req = http_client()?
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "audio/wav,application/octet-stream,*/*")
        .json(&body);
    if let Some(api_key) = optional_str(cfg, "api_key") {
        req = req.header("Authorization", format!("Bearer {api_key}"));
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("调用 IndexTTS 试听失败: {e}"))?;
    let status = resp.status();
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!(
            "IndexTTS 试听 HTTP {status}: {}",
            String::from_utf8_lossy(&bytes)
        ));
    }
    if bytes.is_empty() {
        return Err("IndexTTS 试听返回音频为空".to_string());
    }
    Ok((bytes.to_vec(), "audio/wav".to_string()))
}

struct DoubaoPreviewSelection {
    resource_id: String,
    request_model: String,
}

fn resolve_doubao_preview_selection(model: Option<&str>, voice: &str) -> DoubaoPreviewSelection {
    let model = model.unwrap_or("");
    let mut normalized = normalize_doubao_model(model);
    if infer_doubao_voice_family(voice) == "tts2" && normalized == "seed-tts-1.1" {
        normalized = "seed-tts-2.0-standard".to_string();
    }
    if normalized.is_empty() {
        return match infer_doubao_voice_family(voice) {
            "icl2" => DoubaoPreviewSelection {
                resource_id: "seed-icl-2.0".to_string(),
                request_model: "seed-tts-2.0-expressive".to_string(),
            },
            "icl1" => DoubaoPreviewSelection {
                resource_id: "seed-icl-1.0".to_string(),
                request_model: String::new(),
            },
            "tts2" => DoubaoPreviewSelection {
                resource_id: "seed-tts-2.0".to_string(),
                request_model: String::new(),
            },
            _ => DoubaoPreviewSelection {
                resource_id: "seed-tts-1.0".to_string(),
                request_model: "seed-tts-1.1".to_string(),
            },
        };
    }
    let (resource_id, request_model) = match normalized.as_str() {
        "seed-tts-1.1" => {
            if infer_doubao_voice_family(voice) == "tts2" {
                ("seed-tts-2.0", "")
            } else {
                ("seed-tts-1.0", "seed-tts-1.1")
            }
        }
        "seed-tts-2.0-standard" => {
            if infer_doubao_voice_family(voice) == "tts2" {
                ("seed-tts-2.0", "")
            } else {
                ("seed-tts-2.0", "seed-tts-2.0-standard")
            }
        }
        "seed-tts-2.0-expressive" => {
            if infer_doubao_voice_family(voice) == "tts2" {
                ("seed-tts-2.0", "")
            } else {
                ("seed-tts-2.0", "seed-tts-2.0-expressive")
            }
        }
        "seed-icl-1.0" => ("seed-icl-1.0", ""),
        "seed-icl-2.0-standard" => ("seed-icl-2.0", "seed-tts-2.0-standard"),
        "seed-icl-2.0-expressive" => ("seed-icl-2.0", "seed-tts-2.0-expressive"),
        _ => ("seed-tts-1.0", normalized.as_str()),
    };
    DoubaoPreviewSelection {
        resource_id: resource_id.to_string(),
        request_model: request_model.to_string(),
    }
}

fn normalize_doubao_model(model: &str) -> String {
    match model.trim().to_ascii_lowercase().as_str() {
        "" | "default" => String::new(),
        "seed-tts-1.1" => "seed-tts-1.1".to_string(),
        "seed-tts-2.0-standard" | "seed-tts-2.0" => "seed-tts-2.0-standard".to_string(),
        "seed-tts-2.0-expressive" => "seed-tts-2.0-expressive".to_string(),
        "seed-icl-1.0" => "seed-icl-1.0".to_string(),
        "seed-icl-2.0-standard" => "seed-icl-2.0-standard".to_string(),
        "seed-icl-2.0-expressive" => "seed-icl-2.0-expressive".to_string(),
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

fn resolve_aliyun_qwen_tts_endpoint(cfg: &Value) -> String {
    if let Some(endpoint) = optional_str_any(cfg, &["api_url", "tts_endpoint"]) {
        return endpoint;
    }
    if optional_str(cfg, "region")
        .is_some_and(|r| r.eq_ignore_ascii_case("singapore"))
    {
        return DEFAULT_ALIYUN_QWEN_TTS_ENDPOINT_INTL.to_string();
    }
    DEFAULT_ALIYUN_QWEN_TTS_ENDPOINT.to_string()
}

fn normalize_indextts_base_url(raw: Option<&str>) -> String {
    let raw = raw.unwrap_or("").trim();
    if raw.is_empty() {
        DEFAULT_INDEX_TTS_BASE_URL.to_string()
    } else {
        raw.trim_end_matches('/').to_string()
    }
}

fn random_digits(len: usize) -> String {
    (0..len)
        .map(|_| (b'0' + rand::random::<u8>() % 10) as char)
        .collect()
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

fn optional_str_any(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| optional_str(v, key))
}

fn float_field(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(|x| x.as_f64())
}

fn int_field(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(|x| x.as_i64())
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())
}
