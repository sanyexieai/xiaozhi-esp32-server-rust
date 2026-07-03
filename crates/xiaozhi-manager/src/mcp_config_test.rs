//! MCP 全局配置连通性测试（探测各启用服务器的工具列表）

use std::collections::HashMap;
use std::time::Instant;

use serde_json::{json, Value};

pub async fn test_mcp_config(json_data: &str) -> Value {
    let start = Instant::now();
    let cfg: Value = serde_json::from_str(json_data).unwrap_or(json!({}));
    let servers = extract_global_servers(&cfg);
    if servers.is_empty() {
        return json!({
            "ok": true,
            "message": "无全局 MCP 服务器，跳过探测",
            "first_packet_ms": start.elapsed().as_millis(),
        });
    }

    let mut tested = 0usize;
    let mut tool_total = 0usize;
    let mut errors = Vec::new();

    for (name, transport, url, headers) in servers {
        if url.trim().is_empty() {
            errors.push(format!("{name}: URL 为空"));
            continue;
        }
        tested += 1;
        match xiaozhi_mcp::discover_mcp_tools(&transport, &url, &headers, &HashMap::new()).await {
            Ok(tools) => {
                tool_total += tools.len();
            }
            Err(e) => errors.push(format!("{name}: {e}")),
        }
    }

    let ms = start.elapsed().as_millis() as u64;
    if !errors.is_empty() {
        return json!({
            "ok": false,
            "message": errors.join("; "),
            "first_packet_ms": ms,
            "servers_tested": tested,
            "tool_count": tool_total,
        });
    }

    json!({
        "ok": true,
        "message": if tested == 0 {
            "无有效 MCP 服务器 URL".into()
        } else {
            format!("探测成功：{tested} 台服务器，共 {tool_total} 个工具")
        },
        "first_packet_ms": ms,
        "servers_tested": tested,
        "tool_count": tool_total,
    })
}

fn extract_global_servers(
    cfg: &Value,
) -> Vec<(String, String, String, HashMap<String, String>)> {
    let servers_value = cfg
        .pointer("/mcp/global/servers")
        .or_else(|| cfg.pointer("/global/servers"))
        .and_then(|v| v.as_array());

    let Some(servers) = servers_value else {
        return Vec::new();
    };

    servers
        .iter()
        .filter(|s| s.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true))
        .filter_map(|s| {
            let name = s
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("未命名")
                .to_string();
            let transport = s
                .get("type")
                .or_else(|| s.get("transport"))
                .and_then(|v| v.as_str())
                .unwrap_or("sse")
                .to_string();
            let url = s.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let headers = s
                .get("headers")
                .and_then(|v| v.as_object())
                .map(|obj| {
                    obj.iter()
                        .filter_map(|(k, v)| {
                            v.as_str().map(|s| (k.clone(), s.to_string()))
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some((name, transport, url, headers))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_enabled_servers_from_mcp_global() {
        let cfg = json!({
            "mcp": {
                "global": {
                    "servers": [
                        { "name": "A", "type": "sse", "url": "http://a", "enabled": true },
                        { "name": "B", "type": "sse", "url": "http://b", "enabled": false }
                    ]
                }
            }
        });
        let servers = extract_global_servers(&cfg);
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].0, "A");
    }
}
