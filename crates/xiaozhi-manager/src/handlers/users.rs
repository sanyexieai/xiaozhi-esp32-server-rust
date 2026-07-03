use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app::{json_data, json_error, AppState};
use crate::extractors::AdminUser;

#[derive(Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    pub email: String,
    #[serde(default = "default_user_role")]
    pub role: String,
}

#[derive(Deserialize)]
pub struct UpdateUserRequest {
    pub email: String,
    pub role: String,
}

#[derive(Deserialize)]
pub struct ResetPasswordRequest {
    pub new_password: String,
}

fn default_user_role() -> String {
    "user".to_string()
}

pub async fn list(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
) -> Json<Value> {
    let users = state.db.list_users().unwrap_or_default();
    json_data(users)
}

pub async fn create(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Json(req): Json<CreateUserRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if req.username.len() < 3 {
        return Err(json_error(StatusCode::BAD_REQUEST, "用户名至少 3 个字符"));
    }
    if req.password.len() < 6 {
        return Err(json_error(StatusCode::BAD_REQUEST, "密码至少 6 个字符"));
    }
    if state
        .db
        .find_user_by_username(&req.username)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .is_some()
    {
        return Err(json_error(StatusCode::BAD_REQUEST, "用户名已存在"));
    }
    let hash = crate::auth::hash_password(&req.password)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let id = state
        .db
        .create_user(&req.username, &hash, &req.email, &req.role)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    Ok(json_data(json!({ "id": id, "message": "创建成功" })))
}

pub async fn update(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Path(id): Path<i64>,
    Json(req): Json<UpdateUserRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let ok = state
        .db
        .update_user(id, &req.email, &req.role)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "用户不存在"));
    }
    Ok(json_data(json!({ "message": "更新成功" })))
}

pub async fn delete(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if let Ok(Some(user)) = state.db.find_user_by_id(id) {
        if user.role == "admin" {
            return Err(json_error(StatusCode::BAD_REQUEST, "不能删除管理员账户"));
        }
    }
    let ok = state
        .db
        .delete_user(id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "用户不存在"));
    }
    Ok(json_data(json!({ "message": "删除成功" })))
}

pub async fn reset_password(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Path(id): Path<i64>,
    Json(req): Json<ResetPasswordRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if req.new_password.len() < 6 {
        return Err(json_error(StatusCode::BAD_REQUEST, "密码至少 6 个字符"));
    }
    let hash = crate::auth::hash_password(&req.new_password)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let ok = state
        .db
        .update_user_password(id, &hash)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "用户不存在"));
    }
    Ok(json_data(json!({ "message": "密码重置成功" })))
}

pub async fn voice_clone_quotas(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Path(user_id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let user = state
        .db
        .find_user_by_id(user_id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "用户不存在"))?;
    if user.role != "user" {
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            "仅支持为普通用户分配复刻额度",
        ));
    }

    let tts_configs = state
        .db
        .list_configs("tts")
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let quotas = state
        .db
        .list_voice_clone_quotas(user_id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let usage = state
        .db
        .count_voice_clones_by_tts_config(user_id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let usage_map: std::collections::HashMap<String, i64> = usage.into_iter().collect();
    let quota_map: std::collections::HashMap<String, _> = quotas
        .into_iter()
        .map(|q| (q.tts_config_id.clone(), q))
        .collect();

    let mut result = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for cfg in &tts_configs {
        seen.insert(cfg.config_id.clone());
        let quota = quota_map.get(&cfg.config_id);
        let mut max_count = 0_i64;
        let mut used_count = *usage_map.get(&cfg.config_id).unwrap_or(&0);
        if let Some(q) = quota {
            max_count = q.max_count;
            if q.used_count > used_count {
                used_count = q.used_count;
            }
        }
        let remaining_count = if max_count >= 0 {
            (max_count - used_count).max(0)
        } else {
            -1
        };
        result.push(json!({
            "tts_config_id": cfg.config_id,
            "tts_config_name": cfg.name,
            "provider": cfg.provider,
            "enabled": cfg.enabled,
            "max_count": max_count,
            "used_count": used_count,
            "remaining_count": remaining_count,
        }));
    }
    for (config_id, quota) in &quota_map {
        if seen.contains(config_id) {
            continue;
        }
        let mut used_count = *usage_map.get(config_id).unwrap_or(&0);
        if quota.used_count > used_count {
            used_count = quota.used_count;
        }
        let remaining_count = if quota.max_count >= 0 {
            (quota.max_count - used_count).max(0)
        } else {
            -1
        };
        result.push(json!({
            "tts_config_id": config_id,
            "tts_config_name": "(已删除配置)",
            "provider": "",
            "enabled": false,
            "max_count": quota.max_count,
            "used_count": used_count,
            "remaining_count": remaining_count,
        }));
    }

    Ok(json_data(json!({
        "user_id": user_id,
        "username": user.username,
        "quotas": result,
        "updated_at": chrono::Utc::now().to_rfc3339(),
    })))
}

#[derive(Deserialize)]
pub struct UpdateVoiceCloneQuotasRequest {
    pub items: Vec<VoiceCloneQuotaItem>,
}

#[derive(Deserialize)]
pub struct VoiceCloneQuotaItem {
    pub tts_config_id: String,
    pub max_count: i64,
}

pub async fn update_voice_clone_quotas(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Path(user_id): Path<i64>,
    Json(body): Json<UpdateVoiceCloneQuotasRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let user = state
        .db
        .find_user_by_id(user_id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "用户不存在"))?;
    if user.role != "user" {
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            "仅支持为普通用户分配复刻额度",
        ));
    }
    if body.items.is_empty() {
        return Err(json_error(StatusCode::BAD_REQUEST, "items不能为空"));
    }

    let mut item_map = std::collections::HashMap::new();
    for item in &body.items {
        let config_id = item.tts_config_id.trim();
        if config_id.is_empty() {
            return Err(json_error(StatusCode::BAD_REQUEST, "tts_config_id不能为空"));
        }
        if item.max_count < -1 {
            return Err(json_error(
                StatusCode::BAD_REQUEST,
                "max_count 不能小于 -1",
            ));
        }
        item_map.insert(config_id.to_string(), item.max_count);
    }

    let tts_configs = state
        .db
        .list_configs("tts")
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let valid: std::collections::HashSet<String> =
        tts_configs.iter().map(|c| c.config_id.clone()).collect();
    for (config_id, max_count) in &item_map {
        if valid.contains(config_id) || *max_count == -1 {
            continue;
        }
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            &format!("TTS配置不存在: {config_id}"),
        ));
    }

    let usage = state
        .db
        .count_voice_clones_by_tts_config(user_id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let usage_map: std::collections::HashMap<String, i64> = usage.into_iter().collect();

    for (config_id, max_count) in &item_map {
        if *max_count == -1 {
            state
                .db
                .delete_voice_clone_quota(user_id, config_id)
                .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
            continue;
        }
        let used_count = *usage_map.get(config_id).unwrap_or(&0);
        state
            .db
            .upsert_voice_clone_quota(user_id, config_id, *max_count, used_count)
            .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    }

    Ok(json_data(json!({ "message": "保存成功" })))
}
