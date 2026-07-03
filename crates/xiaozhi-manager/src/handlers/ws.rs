use axum::{
    extract::{
        ws::{WebSocket, WebSocketUpgrade},
        State,
    },
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
};

use crate::app::AppState;
use crate::auth::decode_ws_token;

pub async fn ws_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, StatusCode> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.strip_prefix("Bearer ").unwrap_or(s).trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let claims = decode_ws_token(&token, &state.endpoint_auth_token)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    if claims.purpose != "manager-ws-client" {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let client_uuid = headers
        .get("UUID")
        .and_then(|v| v.to_str().ok())
        .unwrap_or(&claims.uuid)
        .to_string();

    Ok(ws.on_upgrade(move |socket| {
        let hub = state.ws_hub.clone();
        async move {
            hub.handle_socket(client_uuid, socket).await;
        }
    }))
}
