//! 管理台设备对话模拟器：WebSocket 代理与配置接口。
//!
//! 浏览器 WebSocket 无法设置 `Device-Id` 头，由 Manager 代为连接 xiaozhi-server。

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::IntoResponse,
    Json,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_tungstenite::{
    connect_async,
    tungstenite::client::IntoClientRequest,
};

use crate::app::{json_data, AppState};
use crate::auth::decode_token;
use crate::extractors::{AdminUser, AuthUser};
use crate::handlers::devices;
use crate::ota_test;
use xiaozhi_config::AppConfig;

#[derive(Debug, Deserialize)]
pub struct SimulatorWsQuery {
    pub device_id: String,
    #[serde(default = "default_protocol_version")]
    pub protocol_version: u8,
    /// 可选：覆盖 OTA 配置中的 WebSocket 地址（内网调试）
    pub ws_url: Option<String>,
    /// 浏览器无法带 Authorization 头时通过 query 传 JWT
    pub token: Option<String>,
}

fn default_protocol_version() -> u8 {
    1
}

#[derive(Debug, Deserialize)]
pub struct SimulatorSignalsQuery {
    pub device_id: String,
    #[serde(default)]
    pub after_id: u64,
    #[serde(default)]
    pub clear: bool,
}

/// 按 device_id 拉取服务端信令流水（模拟器连接后无 DB id 时也可调试 MCP）。
pub async fn simulator_signals(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Query(query): Query<SimulatorSignalsQuery>,
) -> Result<Json<Value>, StatusCode> {
    let device_id = query.device_id.trim().to_string();
    if device_id.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(devices::fetch_device_signals(&state, &device_id, query.after_id, query.clear).await)
}

/// 模拟器页面所需配置（WebSocket 目标地址、代理路径等）。
pub async fn get_config(
    State(state): State<AppState>,
    AdminUser(claims): AdminUser,
) -> Json<Value> {
    let cfg = load_ota_json(&state);
    let (ws_url, env_key) = resolve_default_ws_url(&cfg);
    let local_ws_url = local_server_ws_url(&state.app_config.read());
    let default_sim = state
        .db
        .ensure_web_simulator_device(claims.sub)
        .ok();
    json_data(json!({
        "ws_url": ws_url,
        "local_ws_url": local_ws_url,
        "env": env_key,
        "ws_proxy_path": "/api/admin/device-simulator/ws",
        "protocol_version": 1,
        "default_sim_device_id": default_sim.as_ref().map(|d| d.device_id.clone()),
        "default_sim_device_name": default_sim.as_ref().map(|d| d.name.clone()),
        "default_sim_db_id": default_sim.as_ref().map(|d| d.id),
        "audio_params": {
            "format": "opus",
            "sample_rate": 16000,
            "channels": 1,
            "frame_duration": 60
        },
        "features": {
            "text_chat": true,
            "voice_chat": false,
            "multimodal": false,
            "mcp_skill": false
        },
        "placeholders": {
            "voice": "语音上行/下行（Opus）尚未实现，可先使用文本模拟 listen.detect",
            "multimodal": "Vision 识图等多模态能力预留，将对接 /xiaozhi/api/vision",
            "mcp_skill": "MCP Skill 工具调用链预留，将对接设备 MCP 与会话内 mcp 消息"
        },
        "session_isolation": {
            "enabled": true,
            "mode": "endpoint_hub",
            "hint": "Web 与硬件可共享同一 device_id 会话；也可使用 default_sim_device_id 独立调试"
        }
    }))
}

/// 用户工作台：继续历史会话所需的 WebSocket 配置（仅能连接本人设备）。
pub async fn get_user_config(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
) -> Json<Value> {
    let cfg = load_ota_json(&state);
    let (ws_url, env_key) = resolve_default_ws_url(&cfg);
    let local_ws_url = local_server_ws_url(&state.app_config.read());
    let default_sim = state
        .db
        .ensure_web_simulator_device(claims.sub)
        .ok();
    json_data(json!({
        "ws_url": ws_url,
        "local_ws_url": local_ws_url,
        "env": env_key,
        "ws_proxy_path": "/api/user/device-chat/ws",
        "protocol_version": 1,
        "default_sim_device_id": default_sim.as_ref().map(|d| d.device_id.clone()),
        "default_sim_device_name": default_sim.as_ref().map(|d| d.name.clone()),
        "default_sim_db_id": default_sim.as_ref().map(|d| d.id),
        "audio_params": {
            "format": "opus",
            "sample_rate": 16000,
            "channels": 1,
            "frame_duration": 60
        },
        "features": {
            "text_chat": true,
            "voice_chat": false,
            "multimodal": false,
            "mcp_skill": false
        },
        "session_isolation": {
            "enabled": true,
            "mode": "endpoint_hub",
            "hint": "Web 与硬件可共享同一 device_id 会话；继续对话时带上 resume_session_id 恢复上下文"
        }
    }))
}

pub async fn user_ws_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<SimulatorWsQuery>,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, StatusCode> {
    let claims = authenticate_token(&headers, params.token.as_deref())
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    let device_id = params.device_id.trim().to_string();
    if device_id.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    if claims.role != "admin" && !user_can_access_device(&state, claims.sub, &device_id) {
        return Err(StatusCode::FORBIDDEN);
    }

    let upstream_device_id = device_id.clone();
    let upstream_url = if let Some(url) = params
        .ws_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        url.to_string()
    } else {
        local_server_ws_url(&state.app_config.read())
    };
    let protocol_version = params.protocol_version;

    Ok(ws.on_upgrade(move |client| async move {
        if let Err(e) = proxy_device_websocket(
            client,
            &upstream_url,
            &upstream_device_id,
            protocol_version,
        )
        .await
        {
            tracing::warn!(
                device_id = %device_id,
                upstream_device_id = %upstream_device_id,
                upstream = %upstream_url,
                "用户设备对话 WebSocket 代理结束: {e:#}"
            );
        }
    }))
}

fn user_can_access_device(state: &AppState, user_id: i64, device_id: &str) -> bool {
    let physical = xiaozhi_core::constants::simulator::resolve_physical_device_id(device_id);
    let candidates = [device_id, physical];
    for id in candidates {
        if id.is_empty() {
            continue;
        }
        if let Ok(Some(device)) = state.db.find_device_by_device_id(id) {
            return device.user_id == Some(user_id);
        }
    }
    false
}

pub async fn ws_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<SimulatorWsQuery>,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, StatusCode> {
    let claims = authenticate_token(&headers, params.token.as_deref())
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    if claims.role != "admin" {
        return Err(StatusCode::FORBIDDEN);
    }

    let device_id = params.device_id.trim().to_string();
    if device_id.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    // 多端在线：直接使用逻辑 device_id，由 EndpointHub 与硬件端共存
    let upstream_device_id = device_id.clone();

    let upstream_url = if let Some(url) = params
        .ws_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        url.to_string()
    } else {
        local_server_ws_url(&state.app_config.read())
    };
    let protocol_version = params.protocol_version;

    Ok(ws.on_upgrade(move |client| async move {
        if let Err(e) = proxy_device_websocket(
            client,
            &upstream_url,
            &upstream_device_id,
            protocol_version,
        )
        .await
        {
            tracing::warn!(
                device_id = %device_id,
                upstream_device_id = %upstream_device_id,
                upstream = %upstream_url,
                "设备模拟器 WebSocket 代理结束: {e:#}"
            );
        }
    }))
}

fn local_server_ws_url(app: &AppConfig) -> String {
    format!(
        "ws://127.0.0.1:{}/xiaozhi/v1/",
        app.websocket.port
    )
}

fn authenticate_token(
    headers: &HeaderMap,
    query_token: Option<&str>,
) -> anyhow::Result<crate::auth::Claims> {
    let header_token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.strip_prefix("Bearer ").unwrap_or(s).trim())
        .filter(|s| !s.is_empty());

    let token = header_token
        .or(query_token.map(str::trim))
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing token"))?;

    decode_token(token).map_err(|e| e.into())
}

fn load_ota_json(state: &AppState) -> Value {
    state
        .db
        .list_configs("ota")
        .unwrap_or_default()
        .into_iter()
        .find_map(|row| serde_json::from_str::<Value>(&row.json_data).ok())
        .unwrap_or_else(|| json!({}))
}

fn resolve_default_ws_url(cfg: &Value) -> (Option<String>, &'static str) {
    if let Some((url, _)) = ota_test::resolve_ws_and_env(cfg, None) {
        let env = if cfg.get("external").is_some_and(|v| {
            v.get("websocket")
                .and_then(|w| w.get("url"))
                .and_then(|u| u.as_str())
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false)
        }) {
            "external"
        } else {
            "test"
        };
        return (Some(url), env);
    }
    (None, "unknown")
}

async fn proxy_device_websocket(
    client: WebSocket,
    upstream_url: &str,
    device_id: &str,
    protocol_version: u8,
) -> anyhow::Result<()> {
    let mut request = upstream_url
        .into_client_request()
        .map_err(|e| anyhow::anyhow!("构造上游 WebSocket 请求失败: {e}"))?;

    let headers = request.headers_mut();
    headers.insert(
        HeaderName::from_static("device-id"),
        HeaderValue::from_str(device_id)?,
    );
    headers.insert(
        HeaderName::from_static("protocol-version"),
        HeaderValue::from_str(&protocol_version.to_string())?,
    );

    let (upstream, _) = connect_async(request)
        .await
        .map_err(|e| anyhow::anyhow!("连接 xiaozhi-server 失败: {e}"))?;

    let (mut client_sink, mut client_stream) = client.split();
    let (mut upstream_sink, mut upstream_stream) = upstream.split();

    let device_id_log = device_id.to_string();
    let upstream_log = upstream_url.to_string();

    let client_to_upstream = tokio::spawn(async move {
        while let Some(msg) = client_stream.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if upstream_sink
                        .send(tokio_tungstenite::tungstenite::Message::Text(
                            text.to_string().into(),
                        ))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(Message::Binary(data)) => {
                    if upstream_sink
                        .send(tokio_tungstenite::tungstenite::Message::Binary(
                            data.into(),
                        ))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(Message::Ping(data)) => {
                    let _ = upstream_sink
                        .send(tokio_tungstenite::tungstenite::Message::Ping(data.into()))
                        .await;
                }
                Ok(Message::Pong(data)) => {
                    let _ = upstream_sink
                        .send(tokio_tungstenite::tungstenite::Message::Pong(data.into()))
                        .await;
                }
                Ok(Message::Close(_)) | Err(_) => break,
            }
        }
    });

    let upstream_to_client = tokio::spawn(async move {
        while let Some(msg) = upstream_stream.next().await {
            match msg {
                Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                    if client_sink
                        .send(Message::Text(text.to_string().into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(tokio_tungstenite::tungstenite::Message::Binary(data)) => {
                    if client_sink
                        .send(Message::Binary(data.into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(tokio_tungstenite::tungstenite::Message::Ping(data)) => {
                    let _ = client_sink.send(Message::Ping(data.into())).await;
                }
                Ok(tokio_tungstenite::tungstenite::Message::Pong(data)) => {
                    let _ = client_sink.send(Message::Pong(data.into())).await;
                }
                Ok(tokio_tungstenite::tungstenite::Message::Close(_)) | Err(_) => break,
                Ok(_) => {}
            }
        }
    });

    tokio::select! {
        _ = client_to_upstream => {},
        _ = upstream_to_client => {},
    }

    tracing::debug!(
        device_id = %device_id_log,
        upstream = %upstream_log,
        "设备模拟器 WebSocket 代理已关闭"
    );
    Ok(())
}
