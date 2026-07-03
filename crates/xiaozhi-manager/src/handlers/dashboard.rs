use axum::{extract::State, Json};
use serde::Deserialize;
use serde_json::Value;

use crate::app::{json_data, AppState};
use crate::extractors::AuthUser;

pub async fn stats(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
) -> Json<Value> {
    let user_filter = if claims.role == "admin" {
        None
    } else {
        Some(claims.sub)
    };

    let total_users = if claims.role == "admin" {
        state.db.count_users().unwrap_or(0)
    } else {
        0
    };
    let total_devices = state.db.count_devices(user_filter).unwrap_or(0);
    let online_devices = state.db.count_online_devices(user_filter).unwrap_or(0);
    let total_agents = state.db.count_agents(user_filter).unwrap_or(0);

    Json(serde_json::json!({
        "totalUsers": total_users,
        "totalDevices": total_devices,
        "onlineDevices": online_devices,
        "totalAgents": total_agents,
        "programStartedAt": chrono::Utc::now().to_rfc3339(),
    }))
}
