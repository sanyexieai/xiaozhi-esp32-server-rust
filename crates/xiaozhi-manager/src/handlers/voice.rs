use axum::{
    extract::{Multipart, Path, Query, State},
    http::{header, StatusCode},
    response::Response,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::fs;

use crate::app::{json_data, json_error, AppState};
use crate::db::VoiceCloneInput;
use crate::extractors::{AdminUser, AuthUser};
use crate::voice_clone_api::{is_clone_active_status, voice_clones_to_api_list};
use crate::voice_clone_preview;
use crate::voice_clone_validate;
use crate::voice_clone_worker::{self, build_minimax_custom_voice_id, clone_provider_capability};
use crate::voice_options::{resolve_voice_options, VoiceOptionsQuery as VoiceOptionsParams};

#[derive(Deserialize)]
pub struct VoiceOptionsQuery {
    pub provider: String,
    pub config_id: Option<String>,
    pub api_url: Option<String>,
    pub api_key: Option<String>,
}

pub async fn voice_options(
    State(state): State<AppState>,
    Query(q): Query<VoiceOptionsQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    match resolve_voice_options(&state, None, to_voice_params(&q)).await {
        Ok(options) => Ok(json_data(options)),
        Err((status, message)) => Err(json_error(status, &message)),
    }
}

pub async fn admin_voice_options(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Path(user_id): Path<i64>,
    Query(q): Query<VoiceOptionsQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    match resolve_voice_options(&state, Some(user_id), to_voice_params(&q)).await {
        Ok(options) => Ok(json_data(options)),
        Err((status, message)) => Err(json_error(status, &message)),
    }
}

fn to_voice_params(q: &VoiceOptionsQuery) -> VoiceOptionsParams {
    VoiceOptionsParams {
        provider: q.provider.clone(),
        config_id: q.config_id.clone(),
        api_url: q.api_url.clone(),
        api_key: q.api_key.clone(),
    }
}

pub async fn capabilities(Query(q): Query<VoiceOptionsQuery>) -> Json<Value> {
    let cap = clone_provider_capability(&q.provider);
    let supported_langs: Vec<&str> = cap.supported_langs;
    json_data(json!({
        "provider": q.provider,
        "enabled": cap.enabled,
        "requires_transcript": cap.requires_transcript,
        "min_text_len": cap.min_text_len,
        "max_text_len": cap.max_text_len,
        "supported_langs": supported_langs,
    }))
}

fn tts_config_lookup(state: &AppState) -> Vec<(String, String, String)> {
    state
        .db
        .list_configs("tts")
        .unwrap_or_default()
        .into_iter()
        .map(|c| (c.config_id, c.name, c.provider))
        .collect()
}

pub async fn list(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
) -> Json<Value> {
    let clones = state.db.list_voice_clones(claims.sub).unwrap_or_default();
    let clone_ids: Vec<i64> = clones.iter().map(|c| c.id).collect();
    let tasks = state
        .db
        .latest_voice_clone_tasks_by_clone(claims.sub, &clone_ids)
        .unwrap_or_default();
    let lookup = tts_config_lookup(&state);
    json_data(voice_clones_to_api_list(&clones, &lookup, &tasks))
}

pub async fn admin_list(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Path(user_id): Path<i64>,
) -> Json<Value> {
    let clones = state.db.list_voice_clones(user_id).unwrap_or_default();
    let clone_ids: Vec<i64> = clones.iter().map(|c| c.id).collect();
    let tasks = state
        .db
        .latest_voice_clone_tasks_by_clone(user_id, &clone_ids)
        .unwrap_or_default();
    let lookup = tts_config_lookup(&state);
    json_data(voice_clones_to_api_list(&clones, &lookup, &tasks))
}

pub async fn create(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    mut multipart: Multipart,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let mut input = VoiceCloneInput {
        tts_config_id: String::new(),
        name: "新声音".to_string(),
        provider: String::new(),
        transcript: String::new(),
    };
    let mut audio_bytes: Option<Vec<u8>> = None;
    let mut audio_name = "sample.wav".to_string();
    let mut transcript_lang = "zh-CN".to_string();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, &e.to_string()))?
    {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "tts_config_id" => input.tts_config_id = field.text().await.unwrap_or_default(),
            "name" => input.name = field.text().await.unwrap_or_default(),
            "transcript" => input.transcript = field.text().await.unwrap_or_default(),
            "transcript_lang" => {
                let lang = field.text().await.unwrap_or_default();
                if !lang.trim().is_empty() {
                    transcript_lang = lang;
                }
            }
            "source_type" => {}
            "audio_file" | "audio_blob" => {
                audio_name = field.file_name().unwrap_or("sample.wav").to_string();
                audio_bytes = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| json_error(StatusCode::BAD_REQUEST, &e.to_string()))?
                        .to_vec(),
                );
            }
            _ => {}
        }
    }

    if input.tts_config_id.is_empty() {
        return Err(json_error(StatusCode::BAD_REQUEST, "tts_config_id 不能为空"));
    }
    let audio_bytes = audio_bytes
        .ok_or_else(|| json_error(StatusCode::BAD_REQUEST, "请上传音频文件(audio_file)"))?;

    let tts_cfg = state
        .db
        .find_config_by_type_and_id("tts", &input.tts_config_id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::BAD_REQUEST, "TTS 配置不存在"))?;
    let provider = if input.provider.trim().is_empty() {
        tts_cfg.provider.trim().to_lowercase()
    } else {
        input.provider.trim().to_lowercase()
    };
    let capability = clone_provider_capability(&provider);
    if !capability.enabled {
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            "当前仅支持 豆包/Minimax/CosyVoice/千问/IndexTTS 提供商的声音复刻",
        ));
    }
    if capability.requires_transcript && input.transcript.trim().is_empty() {
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            "该提供商复刻要求必须填写音频对应文字",
        ));
    }
    if provider == "minimax" && input.transcript.trim().len() < 10 {
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            "Minimax 复刻需要提供至少 10 个字符的 transcript",
        ));
    }

    let tmp_dir = state
        .data_dir
        .join("audio")
        .join("voice_clones")
        .join("_tmp");
    fs::create_dir_all(&tmp_dir)
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let tmp_name = format!("{}_{}", uuid::Uuid::new_v4(), audio_name);
    let tmp_path = tmp_dir.join(&tmp_name);
    fs::write(&tmp_path, &audio_bytes)
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if let Err(msg) = voice_clone_validate::validate_clone_audio_for_provider(
        &provider,
        &tmp_path.to_string_lossy(),
    ) {
        let _ = fs::remove_file(&tmp_path).await;
        return Err(json_error(StatusCode::BAD_REQUEST, &msg));
    }

    if let Err(msg) = state
        .db
        .consume_voice_clone_quota(claims.sub, &input.tts_config_id)
    {
        let _ = fs::remove_file(&tmp_path).await;
        return Err(json_error(StatusCode::BAD_REQUEST, &msg));
    }

    let voice_id = build_minimax_custom_voice_id(&input.tts_config_id);
    let id = state
        .db
        .create_voice_clone(
            claims.sub,
            &input,
            &provider,
            "processing",
            Some(&voice_id),
        )
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let dir = state
        .data_dir
        .join("audio")
        .join("voice_clones")
        .join(id.to_string());
    fs::create_dir_all(&dir)
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let path = dir.join(&audio_name);
    if let Err(e) = fs::rename(&tmp_path, &path).await {
        let _ = fs::remove_file(&tmp_path).await;
        return Err(json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()));
    }
    state
        .db
        .add_voice_clone_audio(id, &path.to_string_lossy(), &audio_name, &transcript_lang)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let task = state
        .db
        .create_voice_clone_task(claims.sub, id, &provider)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    voice_clone_worker::spawn_voice_clone_task(state.db.clone(), task.id);

    Ok(json_data(json!({
        "id": id,
        "name": input.name,
        "provider": provider,
        "provider_voice_id": voice_id,
        "voice_id": voice_id,
        "tts_config_id": input.tts_config_id,
        "status": "processing",
        "task_id": task.task_id,
        "task_status": task.status,
        "message": "复刻任务已提交"
    })))
}

#[derive(Deserialize)]
pub struct UpdateCloneBody {
    pub name: Option<String>,
    pub shared_to_all: Option<bool>,
}

pub async fn update(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<i64>,
    Json(body): Json<UpdateCloneBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if let Some(shared) = body.shared_to_all {
        if claims.role != "admin" {
            return Err(json_error(
                StatusCode::FORBIDDEN,
                "仅管理员可设置共享状态",
            ));
        }
        let clone = state
            .db
            .get_voice_clone(id, claims.sub)
            .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
            .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "声音复刻不存在"))?;
        if !is_clone_active_status(&clone.status) {
            return Err(json_error(
                StatusCode::BAD_REQUEST,
                "仅成功状态的复刻音色允许设置共享状态",
            ));
        }
        let _ = shared;
    }
    if let Some(name) = body.name.as_deref() {
        let name = name.trim();
        if name.is_empty() {
            return Err(json_error(StatusCode::BAD_REQUEST, "名称不能为空"));
        }
        if name.chars().count() > 100 {
            return Err(json_error(
                StatusCode::BAD_REQUEST,
                "名称长度不能超过100个字符",
            ));
        }
    }
    let ok = state
        .db
        .update_voice_clone(id, claims.sub, body.name.as_deref(), body.shared_to_all)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "声音复刻不存在"));
    }
    Ok(json_data(json!({ "message": "更新成功" })))
}

pub async fn delete(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let ok = state
        .db
        .delete_voice_clone(id, claims.sub)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "声音复刻不存在"));
    }
    Ok(json_data(json!({ "message": "删除成功" })))
}

pub async fn retry(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let clone = state
        .db
        .get_voice_clone(id, claims.sub)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "声音复刻不存在"))?;
    if clone.status != "failed" {
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            "仅失败状态的复刻任务允许重新提交",
        ));
    }
    let task_pk = state
        .db
        .requeue_failed_voice_clone_task(id, claims.sub)
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "复刻任务不存在"))?;
    voice_clone_worker::spawn_voice_clone_task(state.db.clone(), task_pk);
    let task = state
        .db
        .get_voice_clone_task(task_pk)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "复刻任务不存在"))?;
    Ok(json_data(json!({
        "id": clone.id,
        "status": "processing",
        "task_id": task.task_id,
        "task_status": task.status,
        "message": "复刻任务已重新提交"
    })))
}

pub async fn list_audios(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let _ = state
        .db
        .get_voice_clone(id, claims.sub)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "声音复刻不存在"))?;
    let audios = state
        .db
        .list_voice_clone_audios(id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    Ok(json_data(audios))
}

pub async fn audio_file(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(audio_id): Path<i64>,
) -> Result<Response, (StatusCode, Json<Value>)> {
    let audio = state
        .db
        .get_voice_clone_audio(audio_id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "音频不存在"))?;
    let _ = state
        .db
        .get_voice_clone(audio.clone_id, claims.sub)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "无权访问"))?;
    let bytes = fs::read(&audio.file_path)
        .await
        .map_err(|_| json_error(StatusCode::NOT_FOUND, "文件不存在"))?;
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "audio/wav")
        .body(bytes.into())
        .unwrap())
}

pub async fn preview(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<i64>,
) -> Result<Response, (StatusCode, Json<Value>)> {
    let clone = state
        .db
        .get_voice_clone(id, claims.sub)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "声音复刻不存在"))?;
    if !is_clone_active_status(&clone.status) {
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            "仅已成功的复刻音色允许试听",
        ));
    }
    let voice_id = clone
        .voice_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| json_error(StatusCode::BAD_REQUEST, "复刻音色ID为空，无法试听"))?;

    let tts_cfg = state
        .db
        .find_config_by_type_and_id("tts", &clone.tts_config_id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::BAD_REQUEST, "关联TTS配置不存在"))?;
    let provider = if clone.provider.trim().is_empty() {
        tts_cfg.provider.trim().to_lowercase()
    } else {
        clone.provider.trim().to_lowercase()
    };
    let cfg: Value = serde_json::from_str(&tts_cfg.json_data).unwrap_or(json!({}));

    let (bytes, content_type) = voice_clone_preview::preview_cloned_voice(&provider, &cfg, voice_id)
        .await
        .map_err(|e| json_error(StatusCode::BAD_GATEWAY, &format!("生成试听音频失败: {e}")))?;

    Ok(Response::builder()
        .header(header::CONTENT_TYPE, content_type)
        .header(
            header::CONTENT_DISPOSITION,
            format!("inline; filename=\"voice_clone_preview_{id}\""),
        )
        .body(bytes.into())
        .unwrap())
}

pub async fn append_audio(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<i64>,
    mut multipart: Multipart,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let clone = state
        .db
        .get_voice_clone(id, claims.sub)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "声音复刻不存在"))?;
    if !is_clone_active_status(&clone.status) {
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            "仅已成功的复刻音色允许追加参考音频",
        ));
    }
    let voice_id = clone
        .voice_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| json_error(StatusCode::BAD_REQUEST, "复刻音色ID为空"))?;

    let mut audio_bytes = Vec::new();
    let mut audio_name = "append.wav".to_string();
    let mut transcript_lang = "zh-CN".to_string();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, &e.to_string()))?
    {
        match field.name() {
            Some("audio_file") | Some("audio_blob") => {
                audio_name = field.file_name().unwrap_or("append.wav").to_string();
                audio_bytes = field
                    .bytes()
                    .await
                    .map_err(|e| json_error(StatusCode::BAD_REQUEST, &e.to_string()))?
                    .to_vec();
            }
            Some("transcript_lang") => {
                let lang = field.text().await.unwrap_or_default();
                if !lang.trim().is_empty() {
                    transcript_lang = lang;
                }
            }
            _ => {}
        }
    }
    if audio_bytes.is_empty() {
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            "请上传音频文件(audio_file)",
        ));
    }

    let tts_cfg = state
        .db
        .find_config_by_type_and_id("tts", &clone.tts_config_id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::BAD_REQUEST, "关联TTS配置不存在"))?;
    let provider = if clone.provider.trim().is_empty() {
        tts_cfg.provider.trim().to_lowercase()
    } else {
        clone.provider.trim().to_lowercase()
    };

    let dir = state
        .data_dir
        .join("audio")
        .join("voice_clones")
        .join(id.to_string());
    fs::create_dir_all(&dir)
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let path = dir.join(&audio_name);
    fs::write(&path, &audio_bytes)
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if let Err(msg) =
        voice_clone_validate::validate_clone_audio_for_provider("indextts_vllm", &path.to_string_lossy())
    {
        let _ = fs::remove_file(&path).await;
        return Err(json_error(StatusCode::BAD_REQUEST, &msg));
    }

    if provider == "indextts_vllm" {
        let cfg: Value = serde_json::from_str(&tts_cfg.json_data).unwrap_or(json!({}));
        voice_clone_worker::append_indextts_reference_audio(
            &cfg,
            &path.to_string_lossy(),
            &audio_name,
            voice_id,
        )
        .await
        .map_err(|e| json_error(StatusCode::BAD_GATEWAY, &format!("追加参考音频失败: {e}")))?;
    }

    let audio_id = state
        .db
        .add_voice_clone_audio(id, &path.to_string_lossy(), &audio_name, &transcript_lang)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    Ok(json_data(json!({
        "id": clone.id,
        "provider_voice_id": voice_id,
        "audio_id": audio_id,
        "message": "追加参考音频成功"
    })))
}
