use std::sync::Arc;

use axum::{
    extract::{
        ws::{Message, WebSocket},
        Path, Query, State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use uuid::Uuid;
use xiaozhi_openclaw::{
    parse_openclaw_token, AgentSession, OpenClawManager, ResponsePayload, WsMessage,
};

use crate::shared_config::SharedAppConfig;

#[derive(Clone)]
pub struct OpenClawWsState {
    pub shared_config: SharedAppConfig,
    pub openclaw: Arc<OpenClawManager>,
}

#[derive(Deserialize)]
pub struct OpenClawWsQuery {
    pub token: String,
}

pub async fn openclaw_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<OpenClawWsState>,
    Path(agent_id): Path<String>,
    Query(query): Query<OpenClawWsQuery>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_openclaw_socket(socket, state, agent_id, query.token))
}

async fn handle_openclaw_socket(
    socket: WebSocket,
    state: OpenClawWsState,
    path_agent_id: String,
    token: String,
) {
    let secret = state
        .shared_config
        .read()
        .await
        .manager
        .endpoint_auth_token
        .clone();
    let claims = match parse_openclaw_token(&token, &secret) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("OpenClaw token 校验失败: {e}");
            return;
        }
    };
    let agent_id = claims.agent_id.trim().to_string();
    if !path_agent_id.trim().is_empty() && path_agent_id.trim() != agent_id {
        tracing::warn!(
            path_agent = %path_agent_id,
            claim_agent = %agent_id,
            "OpenClaw agent_id 不匹配"
        );
        return;
    }

    let (session, mut outbound_rx) = state.openclaw.register_agent_connection(&agent_id);
    let (mut ws_tx, mut ws_rx) = socket.split();

    let writer = tokio::spawn(async move {
        while let Some(text) = outbound_rx.recv().await {
            if ws_tx.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    if let Err(e) = send_handshake_ack(&session) {
        tracing::error!(agent_id = %agent_id, "OpenClaw handshake_ack 失败: {e}");
        state
            .openclaw
            .unregister_agent_connection(&agent_id, &session);
        writer.abort();
        return;
    }
    tracing::info!(
        agent_id = %agent_id,
        endpoint_id = %claims.endpoint_id,
        "OpenClaw 已连接"
    );

    while let Some(msg) = ws_rx.next().await {
        let Ok(msg) = msg else { break };
        match msg {
            Message::Text(text) => {
                if let Err(e) = handle_inbound_text(
                    &state.openclaw,
                    &agent_id,
                    session.as_ref(),
                    &text,
                ) {
                    tracing::warn!(agent_id = %agent_id, "OpenClaw 入站消息处理失败: {e}");
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    tracing::info!(agent_id = %agent_id, "OpenClaw 连接关闭");
    state
        .openclaw
        .unregister_agent_connection(&agent_id, &session);
    writer.abort();
}

fn send_handshake_ack(session: &AgentSession) -> xiaozhi_core::Result<()> {
    session.send(WsMessage {
        id: Uuid::new_v4().to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        msg_type: "handshake_ack".into(),
        correlation_id: String::new(),
        payload: serde_json::json!({
            "version": "1.0.0",
            "server": "xiaozhi-esp32-server-rust",
        }),
    })
}

fn handle_inbound_text(
    openclaw: &OpenClawManager,
    agent_id: &str,
    session: &AgentSession,
    text: &str,
) -> xiaozhi_core::Result<()> {
    let ws_msg: WsMessage = serde_json::from_str(text)
        .map_err(|e| xiaozhi_core::Error::Session(format!("OpenClaw JSON 解析失败: {e}")))?;
    match ws_msg.msg_type.as_str() {
        "handshake" => tracing::info!(agent_id, "OpenClaw handshake 收到"),
        "ping" => reply_pong(session, &ws_msg)?,
        "response" => {
            let payload: ResponsePayload = serde_json::from_value(ws_msg.payload.clone())
                .unwrap_or_default();
            openclaw.handle_response(agent_id, Some(session), &ws_msg.correlation_id, payload);
        }
        "error" => tracing::warn!(agent_id, payload = %ws_msg.payload, "OpenClaw error"),
        "close" => tracing::info!(agent_id, "OpenClaw 请求关闭"),
        other => tracing::warn!(agent_id, msg_type = other, "OpenClaw 未知消息类型"),
    }
    Ok(())
}

fn reply_pong(session: &AgentSession, ping: &WsMessage) -> xiaozhi_core::Result<()> {
    let seq = ping
        .payload
        .get("seq")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    session.send(WsMessage {
        id: Uuid::new_v4().to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        msg_type: "pong".into(),
        correlation_id: ping.id.clone(),
        payload: serde_json::json!({ "seq": seq }),
    })
}
