//! 声音复刻异步任务（对齐 Go `voice_clone_task_worker.go`）

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use reqwest::multipart::{Form, Part};
use serde_json::{json, Value};
use tokio::fs;

use crate::db::Database;

const DEFAULT_MINIMAX_CLONE_ENDPOINT: &str = "https://api.minimaxi.com/v1/voice_clone";
const DEFAULT_MINIMAX_UPLOAD_ENDPOINT: &str = "https://api.minimaxi.com/v1/files/upload";
const DEFAULT_MINIMAX_CLONE_MODEL: &str = "speech-2.5-hd-preview";
const DEFAULT_ALIYUN_QWEN_CLONE_ENDPOINT: &str =
    "https://dashscope.aliyuncs.com/api/v1/services/audio/tts/customization";
const DEFAULT_ALIYUN_QWEN_CLONE_ENDPOINT_INTL: &str =
    "https://dashscope-intl.aliyuncs.com/api/v1/services/audio/tts/customization";
const DEFAULT_ALIYUN_QWEN_CLONE_MODEL: &str = "qwen-voice-enrollment";
const DEFAULT_ALIYUN_QWEN_CLONE_TARGET_MODEL: &str = "qwen3-tts-vc-2026-01-22";
const MAX_ALIYUN_QWEN_CLONE_AUDIO_BYTES: usize = 10 * 1024 * 1024;
const COSYVOICE_CLONE_ENDPOINT: &str = "https://tts.linkerai.cn/clone";
const COSYVOICE_FIXED_KEY: &str = "https://linkerai.top/";
const INDEX_TTS_CLONE_ENDPOINT: &str = "/audio/clone";
const DEFAULT_INDEX_TTS_BASE_URL: &str = "http://127.0.0.1:7860";
const HTTP_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone)]
pub struct CloneProviderCapability {
    pub enabled: bool,
    pub requires_transcript: bool,
    pub min_text_len: u32,
    pub max_text_len: u32,
    pub supported_langs: Vec<&'static str>,
}

pub fn clone_provider_capability(provider: &str) -> CloneProviderCapability {
    match provider.trim().to_lowercase().as_str() {
        "doubao" | "minimax" | "aliyun_qwen" | "indextts_vllm" => CloneProviderCapability {
            enabled: true,
            requires_transcript: false,
            min_text_len: 0,
            max_text_len: 0,
            supported_langs: vec![],
        },
        "cosyvoice" => CloneProviderCapability {
            enabled: true,
            requires_transcript: true,
            min_text_len: 1,
            max_text_len: 0,
            supported_langs: vec![],
        },
        _ => CloneProviderCapability {
            enabled: false,
            requires_transcript: false,
            min_text_len: 0,
            max_text_len: 0,
            supported_langs: vec![],
        },
    }
}

pub fn spawn_voice_clone_task(db: Arc<Database>, task_pk: i64) {
    tokio::spawn(async move {
        if let Err(e) = run_voice_clone_task(&db, task_pk).await {
            tracing::warn!(task_pk, "声音复刻任务异常: {e}");
        }
    });
}

pub fn reload_pending_voice_clone_tasks(db: Arc<Database>) {
    let ids = match db.list_pending_voice_clone_task_ids() {
        Ok(ids) => ids,
        Err(e) => {
            tracing::warn!("加载待处理声音复刻任务失败: {e}");
            return;
        }
    };
    if ids.is_empty() {
        return;
    }
    tracing::info!(count = ids.len(), "重新加载待处理声音复刻任务");
    for task_pk in ids {
        spawn_voice_clone_task(Arc::clone(&db), task_pk);
    }
}

async fn run_voice_clone_task(db: &Database, task_pk: i64) -> Result<(), String> {
    let Some(task) = db
        .claim_voice_clone_task(task_pk)
        .map_err(|e| e.to_string())?
    else {
        return Ok(());
    };
    match process_voice_clone(db, task.voice_clone_id, task.user_id).await {
        Ok(voice_id) => {
            db.finish_voice_clone_task_success(task_pk, &voice_id)
                .map_err(|e| e.to_string())?;
        }
        Err(e) => {
            let msg = truncate_error(&e);
            let _ = db.finish_voice_clone_task_failed(task_pk, &msg);
            tracing::warn!(
                task_pk,
                clone_id = task.voice_clone_id,
                "声音复刻失败: {e}"
            );
        }
    }
    Ok(())
}

async fn process_voice_clone(db: &Database, clone_id: i64, user_id: i64) -> Result<String, String> {
    let clone = db
        .get_voice_clone(clone_id, user_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "复刻记录不存在".to_string())?;

    let audios = db
        .list_voice_clone_audios(clone_id)
        .map_err(|e| e.to_string())?;
    let audio = audios
        .last()
        .ok_or_else(|| "复刻音频不存在".to_string())?;

    if !fs::metadata(&audio.file_path).await.is_ok() {
        return Err(format!("音频文件不存在: {}", audio.file_path));
    }

    let tts_cfg = db
        .find_config_by_type_and_id("tts", &clone.tts_config_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("TTS 配置不存在: {}", clone.tts_config_id))?;

    let provider = if clone.provider.trim().is_empty() {
        tts_cfg.provider.trim().to_lowercase()
    } else {
        clone.provider.trim().to_lowercase()
    };

    let cfg: Value = serde_json::from_str(&tts_cfg.json_data).unwrap_or(json!({}));
    let voice_id = clone
        .voice_id
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| build_minimax_custom_voice_id(&clone.tts_config_id));

    let result = match provider.as_str() {
        "minimax" => {
            clone_with_minimax(
                &cfg,
                &audio.file_path,
                &audio.file_name,
                &clone.transcript,
                &voice_id,
            )
            .await?
        }
        "cosyvoice" => {
            clone_with_cosyvoice(&audio.file_path, &audio.file_name, &clone.transcript).await?
        }
        "aliyun_qwen" => {
            let transcript_lang = if audio.transcript_lang.trim().is_empty() {
                "zh-CN"
            } else {
                audio.transcript_lang.as_str()
            };
            clone_with_aliyun_qwen(
                &cfg,
                &clone.tts_config_id,
                &audio.file_path,
                &clone.transcript,
                transcript_lang,
            )
            .await?
        }
        "indextts_vllm" => {
            clone_with_indextts_vllm(&cfg, &audio.file_path, &audio.file_name, &voice_id).await?
        }
        "doubao" => {
            let voice_id = crate::voice_clone_doubao::clone_with_doubao(
                &cfg,
                &audio.file_path,
                &audio.file_name,
                &clone.transcript,
            )
            .await?;
            CloneResult { voice_id }
        }
        other => return Err(format!("声音复刻暂不支持 provider: {other}")),
    };
    Ok(result.voice_id)
}

struct CloneResult {
    voice_id: String,
}

async fn clone_with_minimax(
    cfg: &Value,
    file_path: &str,
    file_name: &str,
    transcript: &str,
    voice_id: &str,
) -> Result<CloneResult, String> {
    let api_key = str_field(cfg, "api_key")?;
    let clone_endpoint = optional_str(cfg, "voice_clone_endpoint")
        .or_else(|| optional_str(cfg, "clone_endpoint"))
        .unwrap_or_else(|| DEFAULT_MINIMAX_CLONE_ENDPOINT.to_string());
    let upload_endpoint = optional_str(cfg, "voice_clone_upload_endpoint")
        .or_else(|| optional_str(cfg, "files_upload_endpoint"))
        .or_else(|| optional_str(cfg, "file_upload_endpoint"))
        .unwrap_or_else(|| DEFAULT_MINIMAX_UPLOAD_ENDPOINT.to_string());
    let model = optional_str(cfg, "voice_clone_model")
        .or_else(|| optional_str(cfg, "voice_clone_model_id"))
        .or_else(|| optional_str(cfg, "model"))
        .unwrap_or_else(|| DEFAULT_MINIMAX_CLONE_MODEL.to_string());
    let group_id = optional_str(cfg, "group_id").or_else(|| optional_str(cfg, "GroupId"));

    let file_bytes = fs::read(file_path)
        .await
        .map_err(|e| format!("读取音频文件失败: {e}"))?;
    let file_id = upload_minimax_file(
        &api_key,
        &upload_endpoint,
        group_id.as_deref(),
        file_name,
        &file_bytes,
    )
    .await?;

    let mut body = json!({
        "file_id": minimax_file_id_payload(&file_id),
        "voice_id": voice_id,
    });
    let transcript = transcript.trim();
    if !transcript.is_empty() {
        body["text"] = json!(transcript);
        body["model"] = json!(model);
    }

    let client = reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    let mut req = client
        .post(&clone_endpoint)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&body);
    if let Some(gid) = group_id.as_deref().filter(|s| !s.is_empty()) {
        req = req.header("Group-Id", gid).header("GroupId", gid);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("Minimax 复刻 HTTP {status}: {text}"));
    }
    let parsed: Value = serde_json::from_str(&text).unwrap_or(json!({ "raw": text }));
    if let Some((code, msg)) = parse_minimax_status(&parsed) {
        if code != 0 {
            return Err(format!("Minimax 复刻被拒绝(code={code}, msg={msg}): {text}"));
        }
    }
    let resolved = pick_voice_id(&parsed).unwrap_or_else(|| voice_id.to_string());
    Ok(CloneResult {
        voice_id: resolved,
    })
}

async fn clone_with_cosyvoice(
    file_path: &str,
    file_name: &str,
    transcript: &str,
) -> Result<CloneResult, String> {
    let transcript = transcript.trim();
    if transcript.is_empty() {
        return Err("CosyVoice 复刻要求必须填写音频对应文字(train_text)".to_string());
    }

    let mut clone_url = url::Url::parse(COSYVOICE_CLONE_ENDPOINT).map_err(|e| e.to_string())?;
    clone_url
        .query_pairs_mut()
        .append_pair("key", COSYVOICE_FIXED_KEY);

    let file_bytes = fs::read(file_path)
        .await
        .map_err(|e| format!("读取音频文件失败: {e}"))?;
    let part = Part::bytes(file_bytes)
        .file_name(file_name.to_string())
        .mime_str("application/octet-stream")
        .map_err(|e| e.to_string())?;
    let form = Form::new()
        .text("train_text", transcript.to_string())
        .part("train_wav_file", part);

    let resp = http_client()?
        .post(clone_url)
        .header("Accept", "application/json")
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("调用 CosyVoice 克隆接口失败: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("CosyVoice 克隆 HTTP {status}: {text}"));
    }
    let parsed: Value = serde_json::from_str(&text).unwrap_or(json!({}));
    let status_text = pick_string_field(&parsed, &["status"]).unwrap_or_default();
    if status_text != "成功" {
        return Err(format!("CosyVoice 克隆失败(status={status_text}): {text}"));
    }
    let sid = pick_string_field(&parsed, &["sid"])
        .ok_or_else(|| format!("CosyVoice 响应缺少 sid: {text}"))?;
    Ok(CloneResult { voice_id: sid })
}

async fn clone_with_aliyun_qwen(
    cfg: &Value,
    tts_config_id: &str,
    file_path: &str,
    transcript: &str,
    transcript_lang: &str,
) -> Result<CloneResult, String> {
    let api_key = str_field(cfg, "api_key")?;
    let endpoint = resolve_aliyun_qwen_clone_endpoint(cfg);
    let preferred_name = build_aliyun_qwen_preferred_name(tts_config_id);
    let (audio_data, _mime, _size) = build_aliyun_qwen_audio_data_uri(file_path).await?;

    let mut input = json!({
        "action": "create",
        "target_model": DEFAULT_ALIYUN_QWEN_CLONE_TARGET_MODEL,
        "preferred_name": preferred_name,
        "audio": { "data": audio_data },
    });
    let transcript = transcript.trim();
    if !transcript.is_empty() {
        input["text"] = json!(transcript);
        if let Some(lang) = map_aliyun_qwen_clone_language(transcript_lang) {
            input["language"] = json!(lang);
        }
    }
    let body = json!({
        "model": DEFAULT_ALIYUN_QWEN_CLONE_MODEL,
        "input": input,
    });

    let resp = http_client()?
        .post(endpoint)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("调用千问复刻接口失败: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("千问复刻 HTTP {status}: {text}"));
    }
    let parsed: Value = serde_json::from_str(&text).unwrap_or(json!({}));
    if let Some(code) = pick_string_field(&parsed, &["code"]) {
        let msg = pick_string_field(&parsed, &["message"]).unwrap_or_default();
        return Err(format!("千问复刻失败(code={code}, msg={msg}): {text}"));
    }
    let voice_id = parsed
        .get("output")
        .and_then(|o| pick_string_field(o, &["voice"]))
        .ok_or_else(|| format!("千问复刻响应缺少 output.voice: {text}"))?;
    Ok(CloneResult { voice_id })
}

async fn clone_with_indextts_vllm(
    cfg: &Value,
    file_path: &str,
    file_name: &str,
    voice_name: &str,
) -> Result<CloneResult, String> {
    let base_url = normalize_indextts_base_url(optional_str(cfg, "api_url").as_deref());
    let voice_name = voice_name.trim();
    let file_bytes = fs::read(file_path)
        .await
        .map_err(|e| format!("读取音频文件失败: {e}"))?;
    let part = Part::bytes(file_bytes)
        .file_name(file_name.to_string())
        .mime_str("application/octet-stream")
        .map_err(|e| e.to_string())?;
    let form = Form::new()
        .text("voice", voice_name.to_string())
        .part("audio", part);

    let url = format!("{base_url}{INDEX_TTS_CLONE_ENDPOINT}");
    let mut req = http_client()?
        .post(url)
        .header("Accept", "application/json")
        .multipart(form);
    if let Some(api_key) = optional_str(cfg, "api_key") {
        req = req.header("Authorization", format!("Bearer {api_key}"));
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("调用 IndexTTS 克隆接口失败: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("IndexTTS 克隆 HTTP {status}: {text}"));
    }
    let parsed: Value = serde_json::from_str(&text).unwrap_or(json!({}));
    let voice_id = pick_string_field(&parsed, &["voice"])
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| voice_name.to_string());
    if voice_id.is_empty() {
        return Err(format!("IndexTTS 克隆响应缺少 voice: {text}"));
    }
    Ok(CloneResult { voice_id })
}

pub async fn append_indextts_reference_audio(
    cfg: &Value,
    file_path: &str,
    file_name: &str,
    voice_name: &str,
) -> Result<(), String> {
    clone_with_indextts_vllm(cfg, file_path, file_name, voice_name).await?;
    Ok(())
}

async fn upload_minimax_file(
    api_key: &str,
    upload_endpoint: &str,
    group_id: Option<&str>,
    file_name: &str,
    file_bytes: &[u8],
) -> Result<String, String> {
    let part = Part::bytes(file_bytes.to_vec())
        .file_name(file_name.to_string())
        .mime_str("application/octet-stream")
        .map_err(|e| e.to_string())?;
    let form = Form::new()
        .text("purpose", "voice_clone")
        .part("file", part);
    let client = reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    let mut req = client
        .post(upload_endpoint)
        .header("Authorization", format!("Bearer {api_key}"))
        .multipart(form);
    if let Some(gid) = group_id.filter(|s| !s.is_empty()) {
        req = req.header("Group-Id", gid).header("GroupId", gid);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("上传复刻音频 HTTP {status}: {text}"));
    }
    let parsed: Value = serde_json::from_str(&text).unwrap_or(json!({}));
    if let Some((code, msg)) = parse_minimax_status(&parsed) {
        if code != 0 {
            return Err(format!("上传复刻音频被拒绝(code={code}, msg={msg}): {text}"));
        }
    }
    parsed
        .get("file")
        .and_then(|f| pick_string_field(f, &["file_id", "fileId", "id"]))
        .ok_or_else(|| format!("上传响应中未返回 file_id: {text}"))
}

fn minimax_file_id_payload(file_id: &str) -> Value {
    let file_id = file_id.trim();
    if file_id.parse::<i64>().is_ok() {
        Value::String(file_id.to_string())
    } else {
        Value::String(file_id.to_string())
    }
}

fn parse_minimax_status(payload: &Value) -> Option<(i64, String)> {
    let base = payload.get("base_resp")?;
    let code = base.get("status_code")?.as_i64()?;
    let msg = base
        .get("status_msg")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Some((code, msg))
}

fn pick_voice_id(payload: &Value) -> Option<String> {
    for key in ["voice_id", "voiceId", "voice", "speaker_id", "speakerId"] {
        if let Some(v) = pick_string_field(payload, &[key]) {
            return Some(v);
        }
    }
    if let Some(data) = payload.get("data") {
        for key in ["voice_id", "voiceId", "voice", "speaker_id", "speakerId"] {
            if let Some(v) = pick_string_field(data, &[key]) {
                return Some(v);
            }
        }
    }
    None
}

fn pick_string_field(v: &Value, keys: &[&str]) -> Option<String> {
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

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())
}

fn resolve_aliyun_qwen_clone_endpoint(cfg: &Value) -> String {
    if let Some(endpoint) = optional_str_any(
        cfg,
        &[
            "voice_clone_endpoint",
            "clone_endpoint",
            "customization_endpoint",
        ],
    ) {
        return endpoint;
    }
    let api_url = optional_str(cfg, "api_url").unwrap_or_default().to_lowercase();
    if api_url.contains("dashscope-intl.aliyuncs.com") {
        DEFAULT_ALIYUN_QWEN_CLONE_ENDPOINT_INTL.to_string()
    } else {
        DEFAULT_ALIYUN_QWEN_CLONE_ENDPOINT.to_string()
    }
}

fn build_aliyun_qwen_preferred_name(tts_config_id: &str) -> String {
    let mut name = sanitize_voice_id_prefix(tts_config_id);
    if name.is_empty() {
        name = "voiceclone".to_string();
    }
    if name.len() > 16 {
        name.truncate(16);
    }
    if name
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_digit())
    {
        name = format!("vc_{name}");
        if name.len() > 16 {
            name.truncate(16);
        }
    }
    name
}

fn map_aliyun_qwen_clone_language(transcript_lang: &str) -> Option<String> {
    let lang = transcript_lang.trim().to_lowercase();
    let mapped = match lang.as_str() {
        "zh" | "zh-cn" | "zh-hans" | "zh-hant" | "zh-tw" | "zh-hk" => "zh",
        "en" | "en-us" | "en-gb" => "en",
        "de" | "de-de" => "de",
        "it" | "it-it" => "it",
        "pt" | "pt-pt" | "pt-br" => "pt",
        "es" | "es-es" | "es-mx" => "es",
        "ja" | "ja-jp" => "ja",
        "ko" | "ko-kr" => "ko",
        "fr" | "fr-fr" => "fr",
        "ru" | "ru-ru" => "ru",
        _ => {
            if lang.len() >= 2 {
                match &lang[..2] {
                    "zh" | "en" | "de" | "it" | "pt" | "es" | "ja" | "ko" | "fr" | "ru" => {
                        &lang[..2]
                    }
                    _ => return None,
                }
            } else {
                return None;
            }
        }
    };
    Some(mapped.to_string())
}

fn aliyun_qwen_clone_audio_mime_type(ext: &str) -> Option<&'static str> {
    match ext.to_lowercase().as_str() {
        ".wav" => Some("audio/wav"),
        ".mp3" => Some("audio/mpeg"),
        ".m4a" => Some("audio/mp4"),
        _ => None,
    }
}

async fn build_aliyun_qwen_audio_data_uri(file_path: &str) -> Result<(String, String, usize), String> {
    let ext = Path::new(file_path)
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| format!(".{}", s))
        .unwrap_or_default();
    let mime_type = aliyun_qwen_clone_audio_mime_type(&ext)
        .ok_or_else(|| format!("千问声音复刻仅支持 WAV/MP3/M4A，检测到扩展名: {ext}"))?;
    let data = fs::read(file_path)
        .await
        .map_err(|e| format!("读取音频文件失败: {e}"))?;
    if data.is_empty() {
        return Err("音频文件不能为空".to_string());
    }
    if data.len() > MAX_ALIYUN_QWEN_CLONE_AUDIO_BYTES {
        return Err(format!(
            "千问声音复刻音频大小不能超过{}MB，当前{:.2}MB",
            MAX_ALIYUN_QWEN_CLONE_AUDIO_BYTES / 1024 / 1024,
            data.len() as f64 / 1024.0 / 1024.0
        ));
    }
    let encoded = BASE64.encode(&data);
    Ok((
        format!("data:{mime_type};base64,{encoded}"),
        mime_type.to_string(),
        data.len(),
    ))
}

fn normalize_indextts_base_url(raw: Option<&str>) -> String {
    let raw = raw.unwrap_or("").trim();
    if raw.is_empty() {
        DEFAULT_INDEX_TTS_BASE_URL.to_string()
    } else {
        raw.trim_end_matches('/').to_string()
    }
}

pub fn build_minimax_custom_voice_id(tts_config_id: &str) -> String {
    let prefix = sanitize_voice_id_prefix(tts_config_id);
    let suffix: String = (0..8)
        .map(|_| (b'0' + rand::random::<u8>() % 10) as char)
        .collect();
    format!("{prefix}_{suffix}")
}

fn sanitize_voice_id_prefix(tts_config_id: &str) -> String {
    let filtered: String = tts_config_id
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let prefix = filtered.trim_matches('_');
    if prefix.is_empty() {
        "voice".to_string()
    } else {
        prefix.chars().take(40).collect()
    }
}

fn truncate_error(msg: &str) -> String {
    let msg = msg.trim();
    if msg.len() <= 800 {
        return msg.to_string();
    }
    format!("{}...", &msg[..800])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_minimax_voice_id() {
        let id = build_minimax_custom_voice_id("tts-minimax-01");
        assert!(id.starts_with("tts_minimax_01_"));
        assert_eq!(id.len(), "tts_minimax_01_".len() + 8);
    }

    #[test]
    fn aliyun_qwen_preferred_name_truncates() {
        let name = build_aliyun_qwen_preferred_name("123456789012345678");
        assert!(name.starts_with("vc_"));
        assert!(name.len() <= 16);
    }

    #[test]
    fn maps_aliyun_qwen_language() {
        assert_eq!(map_aliyun_qwen_clone_language("zh-CN"), Some("zh".into()));
        assert_eq!(map_aliyun_qwen_clone_language("en-US"), Some("en".into()));
        assert_eq!(map_aliyun_qwen_clone_language("xx"), None);
    }

    #[test]
    fn cosyvoice_requires_transcript() {
        let cap = clone_provider_capability("cosyvoice");
        assert!(cap.requires_transcript);
        assert!(clone_provider_capability("minimax").enabled);
        assert!(!clone_provider_capability("unknown").enabled);
    }
}
