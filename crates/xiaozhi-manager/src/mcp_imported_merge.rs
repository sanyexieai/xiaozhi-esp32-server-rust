use std::collections::HashSet;

use serde_json::{json, Value};

use crate::db::{ConfigRow, Database};
use crate::mcp_market;

/// 将已启用的 `mcp_imported` 合并进人工 MCP 配置的 `global.servers`（对齐 Go `mergeManualAndMarketServers`）。
pub fn merge_mcp_with_enabled_imported_services(
    db: &Database,
    manual_mcp: Value,
) -> Result<(Value, Vec<String>), String> {
    let mut merged = manual_mcp;
    let Some(global_obj) = merged
        .as_object_mut()
        .and_then(|m| m.get_mut("global"))
        .and_then(|g| g.as_object_mut())
    else {
        return Ok((merged, Vec::new()));
    };

    let servers = global_obj
        .get("servers")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut existing_urls = HashSet::new();
    for server in &servers {
        if let Some(url) = server.get("url").and_then(|v| v.as_str()) {
            let norm = mcp_market::normalize_url(url);
            if !norm.is_empty() {
                existing_urls.insert(norm);
            }
        }
        if let Some(url) = server.get("sse_url").and_then(|v| v.as_str()) {
            let norm = mcp_market::normalize_url(url);
            if !norm.is_empty() {
                existing_urls.insert(norm);
            }
        }
    }

    let imported_rows = db.list_configs("mcp_imported").unwrap_or_default();
    let mut warnings = Vec::new();
    let mut out_servers = servers;

    for row in imported_rows {
        if !row.enabled {
            continue;
        }
        let data: Value = serde_json::from_str(&row.json_data).unwrap_or(json!({}));
        if data.get("enabled").and_then(|v| v.as_bool()) == Some(false) {
            continue;
        }

        let url = data
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let norm = mcp_market::normalize_url(&url);
        if norm.is_empty() {
            continue;
        }

        if existing_urls.contains(&norm) {
            warnings.push(format!(
                "市场服务 {} 因 URL 与人工配置冲突被跳过",
                row.name
            ));
            continue;
        }

        let transport = normalize_imported_transport(
            data.get("transport")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
            &url,
        );
        if transport != "sse" && transport != "streamablehttp" {
            continue;
        }

        let service_id = data
            .get("service_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let mut server = json!({
            "name": row.name,
            "type": transport,
            "url": url,
            "enabled": true,
            "provider": "mcp-market",
            "service_id": service_id,
        });
        if transport == "sse" {
            server["sse_url"] = json!(url);
        }
        if let Some(headers) = data.get("headers") {
            if headers
                .as_object()
                .is_some_and(|obj| !obj.is_empty())
            {
                server["headers"] = headers.clone();
            }
        }
        if let Some(tools) = data.get("allowed_tools").and_then(|v| v.as_array()) {
            if !tools.is_empty() {
                server["allowed_tools"] = json!(tools);
            }
        }

        out_servers.push(server);
        existing_urls.insert(norm);
    }

    global_obj.insert("servers".to_string(), Value::Array(out_servers));
    Ok((merged, warnings))
}

/// 智能体「MCP 服务」下拉选项：合并人工 + 市场导入后，返回已启用的 global.servers 名称（对齐 Go）。
pub fn list_enabled_global_mcp_service_names(db: &Database) -> Result<Vec<String>, String> {
    let rows = db.list_configs("mcp").unwrap_or_default();
    let selected = rows
        .iter()
        .find(|r| r.is_default)
        .or_else(|| rows.first());
    let manual_mcp = if let Some(row) = selected {
        let parsed: Value = serde_json::from_str(&row.json_data).unwrap_or(json!({}));
        parsed.get("mcp").cloned().unwrap_or(parsed)
    } else {
        json!({
            "global": {
                "enabled": true,
                "servers": []
            }
        })
    };

    let (merged, _) = merge_mcp_with_enabled_imported_services(db, manual_mcp)?;
    collect_enabled_server_names(&merged)
}

fn collect_enabled_server_names(mcp_map: &Value) -> Result<Vec<String>, String> {
    let Some(global) = mcp_map.get("global").and_then(|v| v.as_object()) else {
        return Ok(Vec::new());
    };
    if global.get("enabled").and_then(|v| v.as_bool()) == Some(false) {
        return Ok(Vec::new());
    }
    let servers = global
        .get("servers")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut names = Vec::new();
    let mut seen = HashSet::new();
    for server in servers {
        let name = server
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if name.is_empty() {
            continue;
        }
        if server.get("enabled").and_then(|v| v.as_bool()) == Some(false) {
            continue;
        }
        if seen.insert(name.to_string()) {
            names.push(name.to_string());
        }
    }
    names.sort();
    Ok(names)
}

fn normalize_imported_transport(transport: &str, url: &str) -> String {
    let t = transport.trim().to_lowercase();
    match t.as_str() {
        "sse" => "sse".to_string(),
        "streamablehttp" | "streamable_http" | "streamable-http" | "http" => {
            "streamablehttp".to_string()
        }
        _ => {
            let lower = url.to_lowercase();
            if lower.contains("/sse") {
                "sse".to_string()
            } else if lower.contains("streamable") {
                "streamablehttp".to_string()
            } else {
                t
            }
        }
    }
}

/// 导入时检测 URL 是否与人工 MCP 配置冲突（对齐 Go `collectManualURLSet`）。
pub fn manual_mcp_url_set(db: &Database) -> Result<HashSet<String>, String> {
    let rows = db.list_configs("mcp").unwrap_or_default();
    let selected = rows
        .iter()
        .find(|r| r.is_default)
        .or_else(|| rows.first());
    let Some(row) = selected else {
        return Ok(HashSet::new());
    };
    let parsed: Value = serde_json::from_str(&row.json_data).unwrap_or(json!({}));
    let mcp_block = parsed.get("mcp").cloned().unwrap_or(parsed);
    let servers = mcp_block
        .get("global")
        .and_then(|g| g.get("servers"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut set = HashSet::new();
    for server in servers {
        for key in ["url", "sse_url"] {
            if let Some(url) = server.get(key).and_then(|v| v.as_str()) {
                let norm = mcp_market::normalize_url(url);
                if !norm.is_empty() {
                    set.insert(norm);
                }
            }
        }
    }
    Ok(set)
}

pub fn imported_row_url_hash(row: &ConfigRow) -> Option<String> {
    serde_json::from_str::<Value>(&row.json_data)
        .ok()
        .and_then(|v| v.get("url_hash").and_then(|h| h.as_str()).map(String::from))
}
