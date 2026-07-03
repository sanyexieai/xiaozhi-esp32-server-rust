use std::sync::Arc;

use axum::{
    extract::{Path, State},
    Json,
};
use serde_json::json;
use xiaozhi_chat::ChatManagerRegistry;
use xiaozhi_mcp::McpManager;

pub struct McpApiState {
    pub chat_registry: Arc<ChatManagerRegistry>,
    pub mcp_manager: Arc<McpManager>,
}

pub async fn list_device_mcp_tools(
    State(state): State<Arc<McpApiState>>,
    Path(device_id): Path<String>,
) -> Json<serde_json::Value> {
    use xiaozhi_mcp::{McpManager, McpToolEntry, DEVICE_MCP_SERVER};

    let mut entries = state.mcp_manager.list_all_tool_entries().await;
    let local_names: std::collections::HashSet<_> = entries.iter().map(|t| t.name.clone()).collect();
    let mut device_count = 0usize;
    let mut device_ready = false;

    if let Some(mgr) = state.chat_registry.get(&device_id) {
        device_ready = mgr.is_device_mcp_ready().await;
        for tool in mgr.list_device_mcp_tools().await {
            if local_names.contains(&tool.name) {
                continue;
            }
            entries.push(McpToolEntry {
                name: tool.name,
                description: tool.description,
                input_schema: tool.input_schema,
                server_name: DEVICE_MCP_SERVER.to_string(),
            });
            device_count += 1;
        }
    }

    let tool_groups = McpManager::group_tool_entries(&entries);
    let tools: Vec<serde_json::Value> = entries
        .iter()
        .map(|entry| {
            json!({
                "name": entry.name,
                "description": entry.description,
                "input_schema": entry.input_schema,
                "server_name": entry.server_name,
            })
        })
        .collect();
    let global_count = state.mcp_manager.global_tool_count().await;
    let total = tools.len();

    Json(json!({
        "deviceId": device_id,
        "tools": tools,
        "tool_groups": tool_groups,
        "globalCount": global_count,
        "deviceCount": device_count,
        "totalCount": total,
        "device_mcp_ready": device_ready,
        "online": state.chat_registry.get(&device_id).is_some(),
        "timestamp": chrono::Utc::now().timestamp(),
    }))
}
