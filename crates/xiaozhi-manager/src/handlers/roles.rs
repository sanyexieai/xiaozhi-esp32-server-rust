use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::{json, Value};

use crate::app::{json_data, json_error, AppState};
use crate::db::{RoleInput, RoleRow};
use crate::extractors::{AdminUser, AuthUser};

pub fn role_json(role: &RoleRow) -> Value {
    json!({
        "id": role.id,
        "user_id": role.user_id,
        "name": role.name,
        "description": role.description,
        "prompt": role.prompt,
        "llm_config_id": role.llm_config_id,
        "tts_config_id": role.tts_config_id,
        "voice": role.voice,
        "role_type": role.role_type,
        "status": role.status,
        "sort_order": role.sort_order,
        "is_default": role.is_default,
        "created_at": role.created_at,
        "updated_at": role.updated_at,
    })
}

fn normalize_status(status: &str) -> String {
    match status {
        "inactive" => "inactive".to_string(),
        _ => "active".to_string(),
    }
}

fn can_access_role(role: &RoleRow, user_id: i64, is_admin: bool) -> bool {
    if is_admin {
        return true;
    }
    role.role_type == "global" || role.user_id == Some(user_id)
}

fn can_modify_role(role: &RoleRow, user_id: i64, is_admin: bool) -> bool {
    if role.role_type == "global" {
        return is_admin;
    }
    is_admin || role.user_id == Some(user_id)
}

pub async fn list(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
) -> Json<Value> {
    let is_admin = claims.role == "admin";
    let (global_roles, user_roles) = state
        .db
        .list_roles_for_user(claims.sub, is_admin)
        .unwrap_or_default();
    json_data(json!({
        "global_roles": global_roles.iter().map(role_json).collect::<Vec<_>>(),
        "user_roles": user_roles.iter().map(role_json).collect::<Vec<_>>(),
    }))
}

pub async fn list_global(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
) -> Json<Value> {
    let roles = state.db.list_global_roles().unwrap_or_default();
    let data: Vec<Value> = roles.iter().map(role_json).collect();
    json_data(data)
}

pub async fn create(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Json(mut body): Json<RoleInput>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if body.name.is_empty() {
        return Err(json_error(StatusCode::BAD_REQUEST, "角色名称不能为空"));
    }
    if body.prompt.is_empty() {
        return Err(json_error(StatusCode::BAD_REQUEST, "系统提示词不能为空"));
    }
    body.status = normalize_status(&body.status);
    body.role_type = "user".to_string();
    body.user_id = Some(claims.sub);
    body.is_default = false;

    let id = state
        .db
        .create_role(&body)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    Ok(json_data(json!({ "id": id, "message": "创建成功" })))
}

pub async fn create_global(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Json(mut body): Json<RoleInput>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if body.name.is_empty() {
        return Err(json_error(StatusCode::BAD_REQUEST, "角色名称不能为空"));
    }
    if body.prompt.is_empty() {
        return Err(json_error(StatusCode::BAD_REQUEST, "系统提示词不能为空"));
    }
    body.status = normalize_status(&body.status);
    body.role_type = "global".to_string();
    body.user_id = None;

    let id = state
        .db
        .create_role(&body)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    Ok(json_data(json!({ "id": id, "message": "创建成功" })))
}

pub async fn update(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<i64>,
    Json(mut body): Json<RoleInput>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let role = state
        .db
        .get_role(id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "角色不存在"))?;
    let is_admin = claims.role == "admin";
    if !can_modify_role(&role, claims.sub, is_admin) {
        return Err(json_error(StatusCode::FORBIDDEN, "无权修改此角色"));
    }
    body.status = normalize_status(&body.status);
    body.role_type = role.role_type.clone();
    body.user_id = role.user_id;
    body.is_default = if role.role_type == "global" {
        body.is_default
    } else {
        false
    };

    let ok = state
        .db
        .update_role(id, &body)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "角色不存在"));
    }
    Ok(json_data(json!({ "message": "更新成功" })))
}

pub async fn update_global(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Path(id): Path<i64>,
    Json(mut body): Json<RoleInput>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let role = state
        .db
        .get_role(id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "角色不存在"))?;
    if role.role_type != "global" {
        return Err(json_error(StatusCode::BAD_REQUEST, "该接口仅允许操作全局角色"));
    }
    body.status = normalize_status(&body.status);
    body.role_type = "global".to_string();
    body.user_id = None;

    let ok = state
        .db
        .update_role(id, &body)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "角色不存在"));
    }
    Ok(json_data(json!({ "message": "更新成功" })))
}

pub async fn delete(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let role = state
        .db
        .get_role(id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "角色不存在"))?;
    let is_admin = claims.role == "admin";
    if !can_modify_role(&role, claims.sub, is_admin) {
        return Err(json_error(StatusCode::FORBIDDEN, "无权删除此角色"));
    }
    let ok = state
        .db
        .delete_role(id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "角色不存在"));
    }
    Ok(json_data(json!({ "message": "删除成功" })))
}

pub async fn delete_global(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let role = state
        .db
        .get_role(id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "角色不存在"))?;
    if role.role_type != "global" {
        return Err(json_error(StatusCode::BAD_REQUEST, "该接口仅允许操作全局角色"));
    }
    let ok = state
        .db
        .delete_role(id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "角色不存在"));
    }
    Ok(json_data(json!({ "message": "删除成功" })))
}

pub async fn toggle(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let role = state
        .db
        .get_role(id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "角色不存在"))?;
    let is_admin = claims.role == "admin";
    if !can_modify_role(&role, claims.sub, is_admin) {
        return Err(json_error(StatusCode::FORBIDDEN, "无权修改此角色"));
    }
    state
        .db
        .toggle_role_status(id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    Ok(json_data(json!({ "message": "状态切换成功" })))
}

pub async fn toggle_global(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    state
        .db
        .toggle_role_status(id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    Ok(json_data(json!({ "message": "状态切换成功" })))
}

pub async fn set_default_global(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let ok = state
        .db
        .set_default_global_role(id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "角色不存在"));
    }
    Ok(json_data(json!({ "message": "已设为默认角色" })))
}

#[derive(serde::Deserialize)]
pub struct ApplyRoleRequest {
    pub role_id: Option<i64>,
}

pub async fn apply_to_device(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(device_id): Path<i64>,
    Json(body): Json<ApplyRoleRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let device = state
        .db
        .get_device_by_id(device_id, claims.sub)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "设备不存在"))?;

    let role_id = body.role_id.ok_or_else(|| {
        json_error(StatusCode::BAD_REQUEST, "role_id 不能为空")
    })?;
    let role = state
        .db
        .get_role(role_id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "角色不存在"))?;
    let is_admin = claims.role == "admin";
    if !can_access_role(&role, claims.sub, is_admin) {
        return Err(json_error(StatusCode::FORBIDDEN, "无权使用此角色"));
    }

    state
        .db
        .set_device_role(device.id, Some(role.id), &role.name)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    Ok(json_data(json!({
        "device_id": device.id,
        "role_id": role.id,
        "role_name": role.name,
    })))
}
