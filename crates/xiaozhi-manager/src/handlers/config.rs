use std::time::Duration;

use axum::{
    extract::{Multipart, Path, State},
    http::StatusCode,
    Json,
};
use serde_json::{json, Value};

use crate::app::{json_data, json_error, AppState};
use crate::db::ConfigInput;
use crate::ota_test;
use crate::system_configs;
use xiaozhi_llm::normalize_llm_provider;

fn prepare_llm_config(body: &mut ConfigInput) {
    if body.json_data.is_empty() {
        body.json_data = "{}".to_string();
    }
    let mut cfg: Value = serde_json::from_str(&body.json_data).unwrap_or_else(|_| json!({}));
    let provider = normalize_llm_provider(&body.config_id, &body.provider, &cfg);
    body.provider = provider.clone();
    if let Value::Object(ref mut map) = cfg {
        map.insert("provider".into(), json!(provider));
    }
    body.json_data = serde_json::to_string(&cfg).unwrap_or_else(|_| body.json_data.clone());
}

macro_rules! config_handlers {
    ($type:expr, $get:ident, $create:ident, $update:ident, $delete:ident) => {
        pub async fn $get(State(state): State<AppState>) -> Json<Value> {
            let rows = state.db.list_configs($type).unwrap_or_default();
            json_data(rows)
        }

        pub async fn $create(
            State(state): State<AppState>,
            Json(mut body): Json<ConfigInput>,
        ) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
            body.r#type = $type.to_string();
            if body.json_data.is_empty() {
                body.json_data = "{}".to_string();
            }
            let id = state
                .db
                .create_config(&body)
                .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
            system_configs::notify_system_config_changed(&state).await;
            Ok(json_data(serde_json::json!({ "id": id, "message": "创建成功" })))
        }

        pub async fn $update(
            State(state): State<AppState>,
            Path(id): Path<i64>,
            Json(mut body): Json<ConfigInput>,
        ) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
            body.r#type = $type.to_string();
            let ok = state
                .db
                .update_config(id, &body)
                .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
            if !ok {
                return Err(json_error(StatusCode::NOT_FOUND, "配置不存在"));
            }
            system_configs::notify_system_config_changed(&state).await;
            Ok(json_data(serde_json::json!({ "message": "更新成功" })))
        }

        pub async fn $delete(
            State(state): State<AppState>,
            Path(id): Path<i64>,
        ) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
            let ok = state
                .db
                .delete_config(id)
                .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
            if !ok {
                return Err(json_error(StatusCode::NOT_FOUND, "配置不存在"));
            }
            system_configs::notify_system_config_changed(&state).await;
            Ok(json_data(serde_json::json!({ "message": "删除成功" })))
        }
    };
}

pub async fn get_llm(State(state): State<AppState>) -> Json<Value> {
    let rows = state.db.list_configs("llm").unwrap_or_default();
    json_data(rows)
}

pub async fn create_llm(
    State(state): State<AppState>,
    Json(mut body): Json<ConfigInput>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    body.r#type = "llm".to_string();
    prepare_llm_config(&mut body);
    let id = state
        .db
        .create_config(&body)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    system_configs::notify_system_config_changed(&state).await;
    Ok(json_data(serde_json::json!({ "id": id, "message": "创建成功" })))
}

pub async fn update_llm(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(mut body): Json<ConfigInput>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    body.r#type = "llm".to_string();
    prepare_llm_config(&mut body);
    let ok = state
        .db
        .update_config(id, &body)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "配置不存在"));
    }
    system_configs::notify_system_config_changed(&state).await;
    Ok(json_data(serde_json::json!({ "message": "更新成功" })))
}

pub async fn delete_llm(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let ok = state
        .db
        .delete_config(id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "配置不存在"));
    }
    system_configs::notify_system_config_changed(&state).await;
    Ok(json_data(serde_json::json!({ "message": "删除成功" })))
}

config_handlers!("asr", get_asr, create_asr, update_asr, delete_asr);
config_handlers!("tts", get_tts, create_tts, update_tts, delete_tts);
config_handlers!("vad", get_vad, create_vad, update_vad, delete_vad);
config_handlers!("ota", get_ota, create_ota, update_ota, delete_ota);
config_handlers!("mqtt", get_mqtt, create_mqtt, update_mqtt, delete_mqtt);
config_handlers!("mqtt_server", get_mqtt_server, create_mqtt_server, update_mqtt_server, delete_mqtt_server);
config_handlers!("udp", get_udp, create_udp, update_udp, delete_udp);
config_handlers!("vision", get_vision, create_vision, update_vision, delete_vision);
config_handlers!("memory", get_memory, create_memory, update_memory, delete_memory);
config_handlers!("voice_identify", get_speaker, create_speaker, update_speaker, delete_speaker);
config_handlers!("knowledge_search", get_knowledge_search, create_knowledge_search, update_knowledge_search, delete_knowledge_search);
config_handlers!("mcp", get_mcp, create_mcp, update_mcp, delete_mcp);
config_handlers!("mcp_market", get_mcp_market, create_mcp_market, update_mcp_market, delete_mcp_market);
config_handlers!("mcp_imported", get_mcp_imported, create_mcp_imported, update_mcp_imported, delete_mcp_imported);

pub async fn set_memory_default(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if let Some(row) = state.db.get_config(id).map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))? {
        let input = crate::db::ConfigInput {
            r#type: row.r#type,
            name: row.name,
            config_id: row.config_id,
            provider: row.provider,
            json_data: row.json_data,
            enabled: row.enabled,
            is_default: true,
        };
        state.db.update_config(id, &input).map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
        system_configs::notify_system_config_changed(&state).await;
        return Ok(json_data(serde_json::json!({ "message": "已设为默认" })));
    }
    Err(json_error(StatusCode::NOT_FOUND, "配置不存在"))
}

pub async fn get_vision_base(State(state): State<AppState>) -> Json<Value> {
    let cfg = state.app_config.read();
    json_data(serde_json::json!({
        "enable_auth": cfg.vision.enable_auth,
        "vision_url": cfg.vision.vision_url,
    }))
}

pub async fn save_vision_base(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    {
        let mut cfg = state.app_config.write();
        if let Some(v) = body.get("enable_auth").and_then(|v| v.as_bool()) {
            cfg.vision.enable_auth = v;
        }
        if let Some(v) = body.get("vision_url").and_then(|v| v.as_str()) {
            cfg.vision.vision_url = v.to_string();
        }
    }

    let vision_base = {
        let cfg = state.app_config.read();
        json!({
            "enable_auth": cfg.vision.enable_auth,
            "vision_url": cfg.vision.vision_url,
        })
    };
    system_configs::upsert_default_system_config(
        &state.db,
        "vision_base",
        "默认 Vision 基础配置",
        &vision_base,
    )
    .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    system_configs::notify_system_config_changed(&state).await;
    Ok(json_data(serde_json::json!({ "message": "保存成功" })))
}

pub async fn toggle_config(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let ok = state
        .db
        .toggle_config(id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "配置不存在"));
    }
    system_configs::notify_system_config_changed(&state).await;
    Ok(json_data(serde_json::json!({ "message": "ok" })))
}

pub async fn test_config(State(state): State<AppState>, Json(body): Json<Value>) -> Json<Value> {
    let types = body
        .get("types")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if types.is_empty() {
        return Json(json!({
            "data": {},
            "success": false,
            "message": "types 不能为空",
        }));
    }

    let client_count = state.ws_hub.client_count().await;
    let mut data = serde_json::Map::new();

    for kind in types {
        match kind.as_str() {
            "ota" => {
                let env = body.get("env").and_then(|v| v.as_str());
                let custom_data = body.get("data").and_then(|d| d.get("ota"));
                let mut ota_result = serde_json::Map::new();

                if let Some(custom) = custom_data.and_then(|v| v.as_object()) {
                    for (config_id, cfg) in custom {
                        if config_id.starts_with('_') {
                            continue;
                        }
                        let item = ota_test::test_ota_config(cfg, env).await;
                        ota_result.insert(config_id.clone(), item);
                    }
                } else {
                    let rows = state.db.list_configs("ota").unwrap_or_default();
                    if rows.is_empty() {
                        ota_result.insert(
                            "_none".into(),
                            json!({ "message": "未配置或未启用 OTA" }),
                        );
                    } else {
                        for row in rows {
                            let cfg: Value =
                                serde_json::from_str(&row.json_data).unwrap_or(json!({}));
                            let item = ota_test::test_ota_config(&cfg, env).await;
                            ota_result.insert(row.config_id.clone(), item);
                        }
                    }
                }
                data.insert("ota".into(), Value::Object(ota_result));
            }
            "knowledge_search" => {
                data.insert(
                    "knowledge_search".into(),
                    test_knowledge_search_configs(&state, &body).await,
                );
            }
            "mcp" => {
                data.insert("mcp".into(), test_mcp_configs(&state, &body).await);
            }
            "vad" | "asr" | "llm" | "tts" | "memory" => {
                if client_count == 0 {
                    data.insert(
                        kind.clone(),
                        json!({
                            "_no_client": {
                                "message": "没有已连接的主服务客户端，请确认 xiaozhi-server 已启动"
                            }
                        }),
                    );
                    continue;
                }
                let type_result = test_type_via_ws(&state, &kind, &body).await;
                data.insert(kind, type_result);
            }
            other => {
                data.insert(
                    other.to_string(),
                    json!({
                        "_error": { "message": format!("不支持的测试类型: {other}") }
                    }),
                );
            }
        }
    }

    Json(json!({ "data": data, "success": true }))
}

async fn test_knowledge_search_configs(state: &AppState, body: &Value) -> Value {
    let custom_data = body.get("data").and_then(|d| d.get("knowledge_search"));
    let config_ids = body
        .get("config_ids")
        .and_then(|v| v.get("knowledge_search"))
        .and_then(|v| v.as_array());

    let rows = if let Some(ids) = config_ids {
        state
            .db
            .list_configs("knowledge_search")
            .unwrap_or_default()
            .into_iter()
            .filter(|r| ids.iter().any(|id| id.as_str() == Some(&r.config_id)))
            .collect()
    } else {
        state
            .db
            .list_configs("knowledge_search")
            .unwrap_or_default()
    };

    if rows.is_empty() && custom_data.is_none() {
        return json!({ "_none": { "message": "未配置知识检索连接" } });
    }

    let mut type_result = serde_json::Map::new();

    if let Some(custom) = custom_data.and_then(|v| v.as_object()) {
        for (config_id, cfg) in custom {
            if config_id.starts_with('_') {
                continue;
            }
            let provider = cfg
                .get("provider")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let config_json = cfg.get("config").cloned().unwrap_or_else(|| cfg.clone().into());
            let item =
                crate::knowledge_search_test::test_knowledge_search(provider, &config_json).await;
            type_result.insert(config_id.clone(), item);
        }
        return Value::Object(type_result);
    }

    for row in rows {
        let cfg: Value = serde_json::from_str(&row.json_data).unwrap_or(json!({}));
        let item =
            crate::knowledge_search_test::test_knowledge_search(&row.provider, &cfg).await;
        type_result.insert(row.config_id.clone(), item);
    }

    Value::Object(type_result)
}

async fn test_mcp_configs(state: &AppState, body: &Value) -> Value {
    let custom_data = body.get("data").and_then(|d| d.get("mcp"));
    let config_ids = body
        .get("config_ids")
        .and_then(|v| v.get("mcp"))
        .and_then(|v| v.as_array());

    let rows = if let Some(ids) = config_ids {
        state
            .db
            .list_configs("mcp")
            .unwrap_or_default()
            .into_iter()
            .filter(|r| ids.iter().any(|id| id.as_str() == Some(&r.config_id)))
            .collect()
    } else {
        state.db.list_configs("mcp").unwrap_or_default()
    };

    if rows.is_empty() && custom_data.is_none() {
        return json!({ "_none": { "message": "未配置 MCP" } });
    }

    let mut type_result = serde_json::Map::new();

    if let Some(custom) = custom_data.and_then(|v| v.as_object()) {
        for (config_id, cfg) in custom {
            if config_id.starts_with('_') {
                continue;
            }
            let json_str = if let Some(inner) = cfg.get("config") {
                serde_json::to_string(inner).unwrap_or_else(|_| cfg.to_string())
            } else {
                cfg.to_string()
            };
            let item = crate::mcp_config_test::test_mcp_config(&json_str).await;
            type_result.insert(config_id.clone(), item);
        }
        return Value::Object(type_result);
    }

    for row in rows {
        let item = crate::mcp_config_test::test_mcp_config(&row.json_data).await;
        type_result.insert(row.config_id.clone(), item);
    }

    Value::Object(type_result)
}

fn resolve_memory_pool_key(provider: &str, config: &Value) -> String {
    config
        .get("provider")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            let p = provider.trim();
            if p.is_empty() {
                "nomemo".to_string()
            } else {
                p.to_string()
            }
        })
}

fn resolve_asr_pool_key(provider: &str, config: &Value) -> String {
    config
        .get("provider")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            let p = provider.trim();
            if p.is_empty() {
                "funasr".to_string()
            } else {
                p.to_string()
            }
        })
}

fn resolve_tts_pool_key(provider: &str, config: &Value) -> String {
    config
        .get("provider")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            let p = provider.trim();
            if p.is_empty() {
                "doubao_ws".to_string()
            } else {
                p.to_string()
            }
        })
}

/// 测试请求体可能带 name/config_id 等元数据；剥离后保证 provider/voice 等字段可被 create_tts 读取
fn prepare_tts_config_for_test(stored_provider: &str, config_json: &mut Value) {
    const META: &[&str] = &[
        "name", "config_id", "enabled", "is_default", "double_stream", "type",
    ];
    let provider = resolve_tts_pool_key(stored_provider, config_json);
    if let Value::Object(ref mut map) = config_json {
        for key in META {
            map.remove(*key);
        }
        map.insert("provider".into(), json!(provider));
    }
}

async fn test_type_via_ws(state: &AppState, kind: &str, body: &Value) -> Value {
    let custom_data = body.get("data").and_then(|d| d.get(kind));
    let config_ids = body
        .get("config_ids")
        .and_then(|v| v.get(kind))
        .and_then(|v| v.as_array());

    let rows = if let Some(ids) = config_ids {
        state
            .db
            .list_configs(kind)
            .unwrap_or_default()
            .into_iter()
            .filter(|r| ids.iter().any(|id| id.as_str() == Some(&r.config_id)))
            .collect()
    } else {
        state.db.list_configs(kind).unwrap_or_default()
    };

    if rows.is_empty() && custom_data.is_none() {
        return json!({ "_none": { "message": "未配置或未启用" } });
    }

    let mut type_result = serde_json::Map::new();

    if let Some(custom) = custom_data.and_then(|v| v.as_object()) {
        for (config_id, cfg) in custom {
            let mut config_json = cfg.get("config").cloned().unwrap_or_else(|| cfg.clone().into());
            let stored = cfg
                .get("provider")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let pool_key = if kind == "llm" {
                let normalized = normalize_llm_provider(config_id, stored, &config_json);
                if let Value::Object(ref mut map) = config_json {
                    map.insert("provider".into(), json!(normalized));
                    map.insert("config_id".into(), json!(config_id));
                }
                config_id.to_string()
            } else if kind == "asr" {
                resolve_asr_pool_key(stored, &config_json)
            } else if kind == "tts" {
                prepare_tts_config_for_test(stored, &mut config_json);
                resolve_tts_pool_key(stored, &config_json)
            } else if kind == "memory" {
                resolve_memory_pool_key(stored, &config_json)
            } else {
                stored.to_string()
            };
            let item = run_ws_config_test(state, kind, &pool_key, config_json).await;
            type_result.insert(config_id.clone(), item);
        }
        return Value::Object(type_result);
    }

    for row in rows {
        let mut cfg: Value = serde_json::from_str(&row.json_data).unwrap_or(json!({}));
        let pool_key = if kind == "llm" {
            let normalized = normalize_llm_provider(&row.config_id, &row.provider, &cfg);
            if let Value::Object(ref mut map) = cfg {
                map.insert("provider".into(), json!(normalized));
                map.insert("config_id".into(), json!(row.config_id));
            }
            row.config_id.clone()
        } else if kind == "asr" {
            resolve_asr_pool_key(&row.provider, &cfg)
        } else if kind == "tts" {
            prepare_tts_config_for_test(&row.provider, &mut cfg);
            resolve_tts_pool_key(&row.provider, &cfg)
        } else if kind == "memory" {
            resolve_memory_pool_key(&row.provider, &cfg)
        } else {
            row.provider.clone()
        };
        let item = run_ws_config_test(state, kind, &pool_key, cfg).await;
        type_result.insert(row.config_id.clone(), item);
    }

    Value::Object(type_result)
}

async fn run_ws_config_test(
    state: &AppState,
    kind: &str,
    provider: &str,
    config: Value,
) -> Value {
    match state
        .ws_hub
        .broadcast_request(
            "POST",
            &format!("/ws/test/{kind}"),
            json!({ "provider": provider, "config": config }),
            Duration::from_secs(30),
        )
        .await
    {
        Ok(resp) if resp.status < 400 => {
            let ok = resp.body.get("ok").and_then(|v| v.as_bool()).unwrap_or(true);
            json!({
                "ok": ok,
                "message": resp.body.get("message").and_then(|v| v.as_str()).unwrap_or("测试完成"),
                "first_packet_ms": resp.body.get("first_packet_ms"),
            })
        }
        Ok(resp) => json!({
            "ok": false,
            "message": if resp.error.is_empty() { "测试失败".into() } else { resp.error },
        }),
        Err(e) => json!({ "ok": false, "message": e }),
    }
}

pub async fn export_config(State(state): State<AppState>) -> Result<String, (StatusCode, String)> {
    system_configs::sync_app_config_from_db(&state)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    serde_yaml::to_string(&*state.app_config.read())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub async fn import_config(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let mut yaml_content = String::new();
    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name().is_some_and(|n| n == "file") {
            yaml_content = field
                .text()
                .await
                .map_err(|e| json_error(StatusCode::BAD_REQUEST, &format!("读取文件失败: {e}")))?;
            break;
        }
    }
    if yaml_content.trim().is_empty() {
        return Err(json_error(StatusCode::BAD_REQUEST, "未找到配置文件"));
    }

    let config: xiaozhi_config::AppConfig = serde_yaml::from_str(&yaml_content)
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, &format!("解析配置失败: {e}")))?;

    system_configs::import_system_blocks_from_app_config(&state.db, &config)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    if let Err(e) = system_configs::sync_app_config_from_db(&state) {
        tracing::warn!("导入后合并 DB 配置失败: {e:#}");
    }

    system_configs::notify_system_config_changed(&state).await;
    Ok(json_data(serde_json::json!({ "message": "导入成功" })))
}

pub async fn get_chat_settings(State(state): State<AppState>) -> Json<Value> {
    let c = state.app_config.read();
    let global_system_prompt = if c.chat.global_system_prompt.is_empty() {
        c.system_prompt.clone()
    } else {
        c.chat.global_system_prompt.clone()
    };
    json_data(serde_json::json!({
        "auth": {
            "enable": c.auth.enable,
            "login_captcha_enabled": c.auth.login_captcha_enabled,
        },
        "chat": {
            "max_idle_duration": c.chat.max_idle_duration,
            "chat_max_silence_duration": c.chat.chat_max_silence_duration,
            "realtime_mode": c.chat.realtime_mode,
            "global_system_prompt": global_system_prompt,
        },
    }))
}

pub async fn save_chat_settings(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let auth_json = body.get("auth").cloned().unwrap_or(json!({}));
    let chat_json = body.get("chat").cloned().unwrap_or(json!({}));

    {
        let mut cfg = state.app_config.write();
        if let Some(v) = auth_json.get("enable").and_then(|v| v.as_bool()) {
            cfg.auth.enable = v;
        }
        if let Some(v) = auth_json
            .get("login_captcha_enabled")
            .and_then(|v| v.as_bool())
        {
            cfg.auth.login_captcha_enabled = v;
        }
        if let Some(v) = chat_json.get("max_idle_duration").and_then(|v| v.as_u64()) {
            cfg.chat.max_idle_duration = v;
        }
        if let Some(v) = chat_json
            .get("chat_max_silence_duration")
            .and_then(|v| v.as_u64())
        {
            cfg.chat.chat_max_silence_duration = v;
        }
        if let Some(v) = chat_json.get("realtime_mode").and_then(|v| v.as_u64()) {
            cfg.chat.realtime_mode = v as u8;
        }
        if let Some(v) = chat_json
            .get("global_system_prompt")
            .and_then(|v| v.as_str())
        {
            cfg.system_prompt = v.to_string();
            cfg.chat.global_system_prompt = v.to_string();
        }
    }

    system_configs::upsert_default_system_config(
        &state.db,
        "auth",
        "默认认证配置",
        &auth_json,
    )
    .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let chat_to_store = {
        let cfg = state.app_config.read();
        json!({
            "max_idle_duration": cfg.chat.max_idle_duration,
            "chat_max_silence_duration": cfg.chat.chat_max_silence_duration,
            "speak_request_reuse_window_ms": cfg.chat.speak_request_reuse_window_ms,
            "realtime_mode": cfg.chat.realtime_mode,
            "global_system_prompt": cfg.chat.global_system_prompt,
        })
    };
    system_configs::upsert_default_system_config(
        &state.db,
        "chat",
        "默认聊天配置",
        &chat_to_store,
    )
    .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    system_configs::notify_system_config_changed(&state).await;
    Ok(json_data(serde_json::json!({ "message": "保存成功" })))
}

pub async fn weknora_models(
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let base_url = body
        .get("base_url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let api_key = body
        .get("api_key")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    match crate::weknora_models::fetch_weknora_models(base_url, api_key).await {
        Ok(lists) => Ok(json_data(serde_json::json!({
            "embedding_models": lists.embedding_models,
            "llm_models": lists.llm_models,
            "rerank_models": lists.rerank_models,
            "all_models": lists.all_models,
        }))),
        Err(e) => Err(json_error(StatusCode::BAD_GATEWAY, &e)),
    }
}
