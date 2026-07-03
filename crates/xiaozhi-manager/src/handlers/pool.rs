use axum::{
    extract::{Query, State},
    Json,
};
use serde::Deserialize;
use serde_json::Value;

use crate::app::{json_data, AppState};
use crate::extractors::AdminUser;

#[derive(Deserialize)]
pub struct StatsQuery {
    #[serde(default = "default_latest")]
    pub r#type: String,
    pub start: Option<String>,
    pub end: Option<String>,
}

fn default_latest() -> String {
    "latest".to_string()
}

pub async fn summary(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
) -> Json<Value> {
    json_data(state.pool_stats.summary())
}

pub async fn query(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Query(q): Query<StatsQuery>,
) -> Json<Value> {
    json_data(state.pool_stats.query(&q.r#type, q.start.as_deref(), q.end.as_deref()))
}
