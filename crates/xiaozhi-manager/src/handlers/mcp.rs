use std::collections::HashSet;
use std::time::Duration;

use std::collections::HashMap;

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::app::{json_data, json_error, AppState};
use crate::openclaw_sse::{
    format_sse, sse_done, sse_error, sse_response, wants_openclaw_sse, ws_response_sse_chunk,
};
use crate::db::ConfigInput;
use crate::extractors::AuthUser;
use crate::mcp_imported_merge::manual_mcp_url_set;
use crate::mcp_market::{
    self, endpoint_import_warning, fetch_all_services, fetch_service_detail, filter_services,
    imported_auth_hint, market_request_auth, merge_imported_headers, normalize_auth_headers,
    parse_market_row, resolve_imported_request_auth, ServiceDetail,
};

pub async fn service_options(State(state): State<AppState>) -> Json<Value> {
    let names = crate::mcp_imported_merge::list_enabled_global_mcp_service_names(&state.db)
        .unwrap_or_default();
    json_data(json!({ "options": names }))
}

pub async fn agent_mcp_endpoint(
    State(state): State<AppState>,
    AuthUser(_): AuthUser,
    Path(id): Path<i64>,
) -> Json<Value> {
    json_data(state.ws_hub.endpoint_status(&id.to_string()).await)
}

pub async fn agent_openclaw_endpoint(
    State(state): State<AppState>,
    AuthUser(_): AuthUser,
    Path(id): Path<i64>,
) -> Json<Value> {
    let agent_id = id.to_string();
    let mut status = match state
        .ws_hub
        .broadcast_request(
            "GET",
            "/api/openclaw/status",
            json!({ "agent_id": agent_id }),
            Duration::from_secs(5),
        )
        .await
    {
        Ok(resp) if resp.status < 400 => resp.body,
        Ok(resp) => json!({
            "agent_id": agent_id,
            "connected": false,
            "status": "offline",
            "status_message": resp.error,
        }),
        Err(e) => json!({
            "agent_id": agent_id,
            "connected": false,
            "status": "offline",
            "status_message": e,
        }),
    };
    if let Some(obj) = status.as_object_mut() {
        obj.insert("endpoint".to_string(), json!(format!("/ws/openclaw/{id}")));
    }
    json_data(status)
}

pub async fn agent_mcp_tools(
    State(state): State<AppState>,
    AuthUser(_): AuthUser,
    Path(id): Path<i64>,
) -> Json<Value> {
    match state
        .ws_hub
        .broadcast_request(
            "GET",
            &format!("/api/agents/{id}/mcp-tools"),
            json!({}),
            Duration::from_secs(10),
        )
        .await
    {
        Ok(resp) if resp.status < 400 => {
            let mut data = resp.body;
            if data.get("tools").is_none() {
                data["tools"] = json!([]);
            }
            if data.get("tool_groups").is_none() {
                data["tool_groups"] = json!([]);
            }
            json_data(data)
        }
        Ok(resp) => json_data(json!({ "tools": [], "tool_groups": [], "error": resp.error })),
        Err(e) => json_data(json!({ "tools": [], "tool_groups": [], "warning": e })),
    }
}

#[derive(Deserialize)]
pub struct McpCallBody {
    pub tool_name: String,
    #[serde(default)]
    pub arguments: Value,
}

pub async fn agent_mcp_call(
    State(state): State<AppState>,
    AuthUser(_): AuthUser,
    Path(id): Path<i64>,
    Json(body): Json<McpCallBody>,
) -> Json<Value> {
    match state
        .ws_hub
        .broadcast_request(
            "POST",
            &format!("/api/agents/{id}/mcp-call"),
            json!({
                "tool_name": body.tool_name,
                "arguments": body.arguments,
            }),
            Duration::from_secs(30),
        )
        .await
    {
        Ok(resp) if resp.status < 400 => json_data(resp.body),
        Ok(resp) => json_data(json!({ "error": resp.error, "status": resp.status })),
        Err(e) => json_data(json!({ "error": e })),
    }
}

pub async fn device_mcp_tools(
    State(state): State<AppState>,
    AuthUser(_): AuthUser,
    Path(id): Path<i64>,
) -> Json<Value> {
    match state
        .ws_hub
        .broadcast_request(
            "GET",
            &format!("/api/devices/{id}/mcp-tools"),
            json!({}),
            Duration::from_secs(10),
        )
        .await
    {
        Ok(resp) if resp.status < 400 => {
            let mut data = resp.body;
            if data.get("tools").is_none() {
                data["tools"] = json!([]);
            }
            if data.get("tool_groups").is_none() {
                data["tool_groups"] = json!([]);
            }
            json_data(data)
        }
        _ => json_data(json!({ "tools": [], "tool_groups": [] })),
    }
}

pub async fn device_mcp_call(
    State(state): State<AppState>,
    AuthUser(_): AuthUser,
    Path(id): Path<i64>,
    Json(body): Json<McpCallBody>,
) -> Json<Value> {
    match state
        .ws_hub
        .broadcast_request(
            "POST",
            &format!("/api/devices/{id}/mcp-call"),
            json!({
                "tool_name": body.tool_name,
                "arguments": body.arguments,
            }),
            Duration::from_secs(30),
        )
        .await
    {
        Ok(resp) => json_data(resp.body),
        Err(e) => json_data(json!({ "error": e })),
    }
}

pub async fn discover_tools(
    Json(body): Json<DiscoverToolsBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let headers = body.headers.unwrap_or_default();
    match xiaozhi_mcp::discover_mcp_tools(&body.transport, &body.url, &headers, &HashMap::new()).await {
        Ok(tools) => {
            let tools: Vec<Value> = tools
                .into_iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                    })
                })
                .collect();
            Ok(json_data(json!({ "tools": tools })))
        }
        Err(e) => Err(json_error(StatusCode::BAD_REQUEST, &e)),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct DiscoverToolsBody {
    transport: String,
    url: String,
    #[serde(default)]
    headers: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize, Default)]
pub struct OpenClawChatQuery {
    pub stream: Option<String>,
}

pub async fn openclaw_chat_test(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<i64>,
    Query(query): Query<OpenClawChatQuery>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let message = body
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if message.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "message 不能为空").into_response();
    }

    let agent_ok = if claims.role == "admin" {
        state
            .db
            .get_agent_by_id(id)
            .ok()
            .flatten()
            .is_some()
    } else {
        state
            .db
            .get_agent(id, claims.sub)
            .ok()
            .flatten()
            .is_some()
    };
    if !agent_ok {
        let msg = if claims.role == "admin" {
            "智能体不存在"
        } else {
            "智能体不存在或不属于当前用户"
        };
        return json_error(StatusCode::NOT_FOUND, msg).into_response();
    }

    let agent_id = id.to_string();
    let mut req_body = body;
    if let Some(obj) = req_body.as_object_mut() {
        obj.insert("message".to_string(), json!(message));
        obj.entry("agent_id")
            .or_insert_with(|| json!(agent_id.clone()));
    }

    let timeout_ms = xiaozhi_openclaw::parse_openclaw_timeout_ms(req_body.get("timeout_ms"));
    let wait_timeout = Duration::from_millis(timeout_ms.saturating_add(5000));

    let accept = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok());
    if wants_openclaw_sse(query.stream.as_deref(), accept) {
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let hub = state.ws_hub.clone();
        let agent_id_for_start = agent_id.clone();
        let body_for_req = req_body.clone();
        tokio::spawn(async move {
            let _ = event_tx.send(format_sse(
                "start",
                &json!({ "agent_id": agent_id_for_start }),
            ));

            let (mut rx, _request_id) = match hub
                .broadcast_stream_request(
                    "POST",
                    "/api/openclaw/chat",
                    body_for_req,
                    wait_timeout,
                )
                .await
            {
                Ok(v) => v,
                Err(e) => {
                    let _ = event_tx.send(sse_error(e));
                    let _ = event_tx.send(sse_done(false, None));
                    return;
                }
            };

            let mut final_body = None;
            let mut terminal_error = false;
            loop {
                match tokio::time::timeout(wait_timeout, rx.recv()).await {
                    Ok(Some(resp)) => {
                        let _ = event_tx.send(ws_response_sse_chunk(&resp));
                        if resp.status == 200 {
                            final_body = Some(resp.body);
                            break;
                        }
                        if resp.status >= 400 {
                            terminal_error = true;
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(_) => {
                        let _ = event_tx.send(sse_error("请求超时"));
                        terminal_error = true;
                        break;
                    }
                }
            }

            let _ = event_tx.send(sse_done(!terminal_error, final_body));
        });
        return sse_response(event_rx).into_response();
    }

    match state
        .ws_hub
        .broadcast_request(
            "POST",
            "/api/openclaw/chat",
            req_body,
            wait_timeout,
        )
        .await
    {
        Ok(resp) if resp.status < 400 => json_data(resp.body).into_response(),
        Ok(resp) => {
            let message = if resp.error.is_empty() {
                format!("OpenClaw 对话测试失败: status={}", resp.status)
            } else {
                resp.error.clone()
            };
            json_data(json!({ "error": message, "data": resp.body })).into_response()
        }
        Err(e) => json_data(json!({ "error": e })).into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct McpMarketSaveBody {
    pub name: String,
    pub provider_id: String,
    pub catalog_url: String,
    #[serde(default)]
    pub detail_url_template: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub auth: McpMarketAuthBody,
}

#[derive(Debug, Deserialize, Default)]
pub struct McpMarketAuthBody {
    #[serde(default)]
    pub r#type: String,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub header_name: String,
}

fn default_true() -> bool {
    true
}

fn build_market_json_data(body: &McpMarketSaveBody, preserve_token: Option<&str>) -> String {
    let auth_type = if body.auth.r#type.trim().is_empty() {
        "bearer"
    } else {
        body.auth.r#type.trim()
    };
    let header_name = if body.auth.header_name.trim().is_empty() {
        "Authorization"
    } else {
        body.auth.header_name.trim()
    };
    let token = body.auth.token.trim();
    let token = if token.is_empty() {
        preserve_token.unwrap_or("").to_string()
    } else {
        token.to_string()
    };
    serde_json::to_string(&json!({
        "name": body.name.trim(),
        "provider_id": body.provider_id.trim(),
        "catalog_url": body.catalog_url.trim(),
        "detail_url_template": body.detail_url_template.trim(),
        "enabled": body.enabled,
        "auth": {
            "type": auth_type,
            "token": token,
            "header_name": header_name,
        }
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

pub async fn create_mcp_market(
    State(state): State<AppState>,
    Json(body): Json<McpMarketSaveBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if body.name.trim().is_empty() || body.catalog_url.trim().is_empty() {
        return Err(json_error(StatusCode::BAD_REQUEST, "名称和目录 URL 不能为空"));
    }
    let provider = mcp_market::normalize_provider(&body.provider_id);
    let input = ConfigInput {
        r#type: "mcp_market".to_string(),
        name: body.name.trim().to_string(),
        config_id: format!("mcp_market_{provider}"),
        provider: provider.clone(),
        json_data: build_market_json_data(&body, None),
        enabled: body.enabled,
        is_default: false,
    };
    let id = state
        .db
        .create_config(&input)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    crate::system_configs::notify_system_config_changed(&state).await;
    Ok(json_data(json!({ "id": id, "message": "创建成功" })))
}

pub async fn update_mcp_market(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<McpMarketSaveBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if body.name.trim().is_empty() || body.catalog_url.trim().is_empty() {
        return Err(json_error(StatusCode::BAD_REQUEST, "名称和目录 URL 不能为空"));
    }
    let existing = state
        .db
        .get_config(id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "配置不存在"))?;
    if existing.r#type != "mcp_market" {
        return Err(json_error(StatusCode::NOT_FOUND, "配置不存在"));
    }
    let preserve_token = parse_market_row(&existing).map(|m| m.auth.token);
    let provider = mcp_market::normalize_provider(&body.provider_id);
    let input = ConfigInput {
        r#type: "mcp_market".to_string(),
        name: body.name.trim().to_string(),
        config_id: if existing.config_id.is_empty() {
            format!("mcp_market_{provider}")
        } else {
            existing.config_id
        },
        provider,
        json_data: build_market_json_data(&body, preserve_token.as_deref()),
        enabled: body.enabled,
        is_default: existing.is_default,
    };
    let ok = state
        .db
        .update_config(id, &input)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "配置不存在"));
    }
    crate::system_configs::notify_system_config_changed(&state).await;
    Ok(json_data(json!({ "message": "更新成功" })))
}

pub async fn market_providers() -> Json<Value> {
    json_data(vec![json!({
        "id": "modelscope",
        "name": "魔搭 ModelScope",
        "catalog_url": "https://www.modelscope.cn/openapi/v1/mcp/servers",
        "detail_url_template": "https://www.modelscope.cn/openapi/v1/mcp/servers/{raw_id}",
        "description": "需填写魔搭访问令牌，拉取你已激活的 MCP 服务（/operational）。",
    })])
}

pub async fn market_list(State(state): State<AppState>) -> Json<Value> {
    let rows = state.db.list_configs("mcp_market").unwrap_or_default();
    let items: Vec<Value> = rows
        .iter()
        .filter_map(|row| {
            parse_market_row(row).map(|m| {
                json!({
                    "id": row.id,
                    "name": m.name,
                    "provider_id": m.provider_id,
                    "catalog_url": m.catalog_url,
                    "detail_url_template": m.detail_url_template,
                    "enabled": m.enabled,
                    "auth_type": m.auth.auth_type,
                    "header_name": m.auth.header_name,
                    "has_token": !m.auth.token.is_empty(),
                })
            })
        })
        .collect();
    json_data(items)
}

pub async fn imported_services(State(state): State<AppState>) -> Json<Value> {
    let rows = state.db.list_configs("mcp_imported").unwrap_or_default();
    let items: Vec<Value> = rows
        .iter()
        .map(|row| flatten_imported_row(row))
        .collect();
    json_data(json!({ "items": items, "total": items.len(), "page": 1, "page_size": 50 }))
}

#[derive(Deserialize)]
pub struct MarketServiceQuery {
    pub q: Option<String>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

pub async fn market_services(
    State(state): State<AppState>,
    Query(q): Query<MarketServiceQuery>,
) -> Json<Value> {
    let page = q.page.unwrap_or(1).max(1);
    let page_size = q.page_size.unwrap_or(20).clamp(1, 100);

    let markets: Vec<_> = state
        .db
        .list_configs("mcp_market")
        .unwrap_or_default()
        .iter()
        .filter_map(parse_market_row)
        .filter(|m| m.enabled)
        .collect();

    if markets.is_empty() {
        return json_data(json!({
            "items": [],
            "total": 0,
            "page": page,
            "page_size": page_size,
            "warnings": [
                "尚未配置已启用的 MCP 市场连接：请先在左侧「新增连接」，填写魔搭 Token 并启用"
            ],
        }));
    }

    let (mut items, mut warnings) = fetch_all_services(&markets).await;
    if items.is_empty() && warnings.is_empty() {
        warnings.push(
            "市场连接正常，但未发现可导入服务。魔搭需先在 modelscope.cn 激活 MCP 服务，且 Token 需有权限"
                .to_string(),
        );
    }
    if let Some(query) = q.q.as_deref() {
        items = filter_services(&items, query);
    }
    items.sort_by(|a, b| {
        a.market_name
            .cmp(&b.market_name)
            .then(a.name.cmp(&b.name))
    });

    let total = items.len() as i64;
    let start = ((page - 1) * page_size) as usize;
    let end = (start + page_size as usize).min(items.len());
    let paged: Vec<Value> = items[start..end]
        .iter()
        .map(|s| {
            json!({
                "market_id": s.market_id,
                "market_name": s.market_name,
                "service_id": s.service_id,
                "name": s.name,
                "description": s.description,
            })
        })
        .collect();

    let mut data = json!({
        "items": paged,
        "total": total,
        "page": page,
        "page_size": page_size,
    });
    if !warnings.is_empty() {
        data["warnings"] = json!(warnings);
    }
    json_data(data)
}

pub async fn test_market(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let row = state
        .db
        .get_config(id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "市场配置不存在"))?;

    let market = parse_market_row(&row)
        .ok_or_else(|| json_error(StatusCode::BAD_REQUEST, "市场配置不完整"))?;

    let (items, warnings) = fetch_all_services(&[market.clone()]).await;
    if !warnings.is_empty() && items.is_empty() {
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            &format!("连接测试失败: {}", warnings.join("; ")),
        ));
    }
    let mut resp = json!({
        "service_count": items.len(),
        "market_id": market.id,
        "message": "连接测试成功",
    });
    if !warnings.is_empty() {
        resp["warnings"] = json!(warnings);
    }
    Ok(json_data(resp))
}

pub async fn market_service_detail(
    State(state): State<AppState>,
    Path((market_id, service_id)): Path<(i64, String)>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let service_id = service_id.trim().trim_start_matches('/').to_string();
    if service_id.is_empty() {
        return Err(json_error(StatusCode::BAD_REQUEST, "service_id 不能为空"));
    }

    let row = state
        .db
        .get_config(market_id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "市场配置不存在"))?;

    let market = parse_market_row(&row)
        .ok_or_else(|| json_error(StatusCode::BAD_REQUEST, "市场配置不完整"))?;

    let detail = fetch_service_detail(&market, &service_id)
        .await
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, &e))?;

    Ok(json_data(detail_to_json(&detail)))
}

#[derive(Deserialize)]
pub struct ImportServiceBody {
    pub market_id: i64,
    pub service_id: String,
    #[serde(default)]
    pub name_override: String,
    /// 指定要导入的 endpoint URL；多 endpoint 时必填。
    #[serde(default)]
    pub endpoint_url: String,
}

pub async fn import_service(
    State(state): State<AppState>,
    Json(body): Json<ImportServiceBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let service_id = body.service_id.trim().to_string();
    if service_id.is_empty() {
        return Err(json_error(StatusCode::BAD_REQUEST, "service_id 不能为空"));
    }

    let row = state
        .db
        .get_config(body.market_id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "市场配置不存在"))?;

    let market = parse_market_row(&row)
        .ok_or_else(|| json_error(StatusCode::BAD_REQUEST, "市场配置不完整"))?;

    let detail = fetch_service_detail(&market, &service_id)
        .await
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, &e))?;

    if detail.endpoints.is_empty() {
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            "该服务暂无可用的 SSE/StreamableHTTP 地址，请先在魔搭完成激活/部署后重试",
        ));
    }

    let endpoints_to_import = mcp_market::select_endpoints_for_import(
        &detail.endpoints,
        &body.endpoint_url,
    )
    .map_err(|e| json_error(StatusCode::BAD_REQUEST, &e))?;

    let existing = state.db.list_configs("mcp_imported").unwrap_or_default();
    let manual_urls = manual_mcp_url_set(&state.db).unwrap_or_default();
    let url_hashes: HashSet<String> = existing
        .iter()
        .filter_map(|r| {
            serde_json::from_str::<Value>(&r.json_data)
                .ok()
                .and_then(|v| v.get("url_hash").and_then(|h| h.as_str()).map(String::from))
        })
        .collect();
    let mut used_names: HashSet<String> = existing.iter().map(|r| r.name.clone()).collect();

    let base_name = if body.name_override.trim().is_empty() {
        detail.name.clone()
    } else {
        body.name_override.trim().to_string()
    };

    let mut imported_names = Vec::new();
    let mut skipped_urls = Vec::new();
    let mut import_warnings = Vec::new();

    for (idx, ep) in endpoints_to_import.iter().enumerate() {
        let resolved_url = ep.url.clone();
        let url_hash = url_hash(&resolved_url);
        let norm_url = mcp_market::normalize_url(&resolved_url);
        if manual_urls.contains(&norm_url) {
            skipped_urls.push(resolved_url.clone());
            continue;
        }
        if url_hashes.contains(&url_hash) {
            skipped_urls.push(resolved_url.clone());
            continue;
        }

        let name = if endpoints_to_import.len() > 1 {
            resolve_unique_name(&used_names, &format!("{}-{}", base_name, idx + 1))
        } else {
            resolve_unique_name(&used_names, &base_name)
        };

        let merged_headers = merge_imported_headers(ep.auth_required, Some(&market), &ep.headers);

        let json_data = json!({
            "transport": ep.transport,
            "url": resolved_url,
            "url_hash": url_hash,
            "headers": merged_headers,
            "auth_required": ep.auth_required,
            "allowed_tools": [],
            "market_id": detail.market_id,
            "provider_id": market.provider_id,
            "service_id": detail.service_id,
            "service_name": detail.name,
            "source_url": detail.source_url,
        });

        let input = ConfigInput {
            r#type: "mcp_imported".to_string(),
            name: name.clone(),
            config_id: format!("imported_{}_{}", detail.market_id, url_hash[..8].to_string()),
            provider: market.provider_id.clone(),
            json_data: json_data.to_string(),
            enabled: true,
            is_default: false,
        };

        state
            .db
            .create_config(&input)
            .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
        used_names.insert(name.clone());
        imported_names.push(name);
        if let Some(warn) = endpoint_import_warning(ep, &detail.source_url) {
            import_warnings.push(warn);
        }
    }

    if imported_names.is_empty() {
        let msg = if !skipped_urls.is_empty() {
            "所有可导入地址均已存在或与人工 MCP 配置冲突"
        } else {
            "所有可导入地址均已存在或冲突"
        };
        return Err(json_error(StatusCode::CONFLICT, msg));
    }

    crate::system_configs::notify_system_config_changed(&state).await;

    Ok(json_data(json!({
        "service_id": detail.service_id,
        "service_name": detail.name,
        "market_id": detail.market_id,
        "market_name": detail.market_name,
        "imported_names": imported_names,
        "imported_count": imported_names.len(),
        "skipped_urls": skipped_urls,
        "warnings": import_warnings,
    })))
}

pub async fn imported_service_tools(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let row = state
        .db
        .get_config(id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "导入服务不存在"))?;

    if row.r#type != "mcp_imported" {
        return Err(json_error(StatusCode::NOT_FOUND, "导入服务不存在"));
    }

    let data: Value = serde_json::from_str(&row.json_data)
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, &format!("配置解析失败: {e}")))?;

    let transport = data
        .get("transport")
        .and_then(|v| v.as_str())
        .unwrap_or("streamablehttp");
    let raw_url = data
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if raw_url.is_empty() {
        return Err(json_error(StatusCode::BAD_REQUEST, "url 不能为空"));
    }
    let url = raw_url;

    let market = data
        .get("market_id")
        .and_then(|v| v.as_i64())
        .and_then(|mid| {
            state
                .db
                .get_config(mid)
                .ok()
                .flatten()
                .and_then(|row| parse_market_row(&row))
        });
    let (headers, cookies) = resolve_imported_request_auth(&data, market.as_ref());
    if headers.is_empty() {
        if let Some(hint) = imported_auth_hint(&data) {
            return Err(json_error(StatusCode::BAD_REQUEST, &hint));
        }
        if data.get("provider_id").and_then(|v| v.as_str()) == Some("modelscope") {
            return Err(json_error(
                StatusCode::BAD_REQUEST,
                "魔搭 MCP 需要鉴权：请在 Headers 中配置 Authorization，或确认该服务是否需要服务商 Token",
            ));
        }
    }

    let tools = xiaozhi_mcp::discover_mcp_tools(transport, &url, &headers, &cookies)
        .await
        .map_err(|e| {
            if e.contains("401") {
                let mut msg = e.clone();
                if let Some(hint) = imported_auth_hint(&data) {
                    msg = format!("{msg}。{hint}");
                } else {
                    msg = format!("{msg}（请确认 Token 有效；部分服务需服务商 Token 而非魔搭 ms- Token）");
                }
                json_error(StatusCode::BAD_REQUEST, &msg)
            } else {
                json_error(StatusCode::BAD_REQUEST, &e)
            }
        })?;

    let tools_json: Vec<Value> = tools
        .into_iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
            })
        })
        .collect();

    Ok(json_data(json!({
        "service": flatten_imported_row(&row),
        "tools": tools_json,
    })))
}

#[derive(Deserialize)]
pub struct ImportedServiceSaveBody {
    pub name: String,
    pub enabled: Option<bool>,
    pub transport: String,
    pub url: String,
    #[serde(default)]
    pub headers: Option<Value>,
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(default)]
    pub market_id: Option<i64>,
    #[serde(default)]
    pub provider_id: String,
    #[serde(default)]
    pub service_id: String,
    #[serde(default)]
    pub service_name: String,
    #[serde(default)]
    pub auth_required: Option<bool>,
    #[serde(default)]
    pub source_url: String,
}

pub async fn create_imported_service(
    State(state): State<AppState>,
    Json(body): Json<ImportedServiceSaveBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let (input, view) = build_imported_service_input(&state, &body, None)?;
    if url_hash_exists(&state, &input, None)? {
        return Err(json_error(StatusCode::CONFLICT, "已存在相同 URL 的导入服务"));
    }
    let id = state
        .db
        .create_config(&input)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    crate::system_configs::notify_system_config_changed(&state).await;
    Ok(json_data(json!({ "id": id, "data": view })))
}

pub async fn update_imported_service(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<ImportedServiceSaveBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let existing = state
        .db
        .get_config(id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "导入服务不存在"))?;
    if existing.r#type != "mcp_imported" {
        return Err(json_error(StatusCode::NOT_FOUND, "导入服务不存在"));
    }
    let (input, view) = build_imported_service_input(&state, &body, Some(&existing))?;
    if url_hash_exists(&state, &input, Some(id))? {
        return Err(json_error(StatusCode::CONFLICT, "已存在相同 URL 的导入服务"));
    }
    let ok = state
        .db
        .update_config(id, &input)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "导入服务不存在"));
    }
    crate::system_configs::notify_system_config_changed(&state).await;
    Ok(json_data(json!({ "data": view })))
}

pub async fn delete_imported_service(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let row = state
        .db
        .get_config(id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "导入服务不存在"))?;
    if row.r#type != "mcp_imported" {
        return Err(json_error(StatusCode::NOT_FOUND, "导入服务不存在"));
    }
    state
        .db
        .delete_config(id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    crate::system_configs::notify_system_config_changed(&state).await;
    Ok(json_data(json!({ "message": "删除成功" })))
}

fn build_imported_service_input(
    state: &AppState,
    body: &ImportedServiceSaveBody,
    existing: Option<&crate::db::ConfigRow>,
) -> Result<(ConfigInput, Value), (StatusCode, Json<Value>)> {
    let transport = normalize_imported_transport(&body.transport, &body.url)
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, &e))?;

    let existing_data: Value = existing
        .map(|r| serde_json::from_str(&r.json_data).unwrap_or(json!({})))
        .unwrap_or(json!({}));

    let url = body.url.trim();
    if url.is_empty() {
        return Err(json_error(StatusCode::BAD_REQUEST, "url 不能为空"));
    }
    let hash = url_hash(url);
    if hash.is_empty() {
        return Err(json_error(StatusCode::BAD_REQUEST, "url 不能为空"));
    }

    let auth_required = body
        .auth_required
        .or_else(|| existing_data.get("auth_required").and_then(|v| v.as_bool()))
        .unwrap_or(true);

    let mut headers = headers_from_value(body.headers.as_ref());
    enrich_imported_headers(state, body.market_id, auth_required, &mut headers);

    let allowed_tools = body
        .allowed_tools
        .clone()
        .or_else(|| {
            existing_data
                .get("allowed_tools")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
        })
        .unwrap_or_default();

    let enabled = body.enabled.unwrap_or(existing.map(|r| r.enabled).unwrap_or(true));
    let name = normalize_imported_name(
        &body.name,
        &body.service_name,
        &body.service_id,
        &url,
        existing.map(|r| r.name.as_str()),
    );

    let market_id = body.market_id.or_else(|| {
        existing_data
            .get("market_id")
            .and_then(|v| v.as_i64())
    });
    let provider_id = if body.provider_id.trim().is_empty() {
        existing_data
            .get("provider_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    } else {
        body.provider_id.trim().to_string()
    };
    let service_id = if body.service_id.trim().is_empty() {
        existing_data
            .get("service_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    } else {
        body.service_id.trim().to_string()
    };
    let service_name = if body.service_name.trim().is_empty() {
        existing_data
            .get("service_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    } else {
        body.service_name.trim().to_string()
    };
    let source_url = if body.source_url.trim().is_empty() {
        existing_data
            .get("source_url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    } else {
        body.source_url.trim().to_string()
    };

    let json_data = json!({
        "transport": transport,
        "url": url,
        "url_hash": hash,
        "headers": headers,
        "auth_required": auth_required,
        "allowed_tools": allowed_tools,
        "market_id": market_id,
        "provider_id": provider_id,
        "service_id": service_id,
        "service_name": service_name,
        "source_url": source_url,
    });

    let config_id = existing
        .and_then(|r| {
            if r.config_id.is_empty() {
                None
            } else {
                Some(r.config_id.clone())
            }
        })
        .unwrap_or_else(|| {
            format!(
                "imported_{}_{}",
                market_id.unwrap_or(0),
                &hash[..hash.len().min(8)]
            )
        });

    let input = ConfigInput {
        r#type: "mcp_imported".to_string(),
        name,
        config_id,
        provider: provider_id.clone(),
        json_data: json_data.to_string(),
        enabled,
        is_default: false,
    };

    let mut view = json_data.clone();
    if let Some(row) = existing {
        view["id"] = json!(row.id);
    }
    Ok((input, view))
}

fn url_hash_exists(
    state: &AppState,
    input: &ConfigInput,
    exclude_id: Option<i64>,
) -> Result<bool, (StatusCode, Json<Value>)> {
    let parsed: Value = serde_json::from_str(&input.json_data)
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, &e.to_string()))?;
    let hash = parsed
        .get("url_hash")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if hash.is_empty() {
        return Ok(false);
    }
    let rows = state
        .db
        .list_configs("mcp_imported")
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    Ok(rows.iter().any(|row| {
        if Some(row.id) == exclude_id {
            return false;
        }
        serde_json::from_str::<Value>(&row.json_data)
            .ok()
            .and_then(|v| v.get("url_hash").and_then(|h| h.as_str()).map(String::from))
            == Some(hash.to_string())
    }))
}

fn normalize_imported_transport(transport: &str, url: &str) -> Result<String, String> {
    let t = transport.trim().to_lowercase();
    let normalized = match t.as_str() {
        "sse" => "sse".to_string(),
        "streamablehttp" | "streamable_http" | "streamable-http" | "http" => {
            "streamablehttp".to_string()
        }
        _ => {
            let lower = url.to_lowercase();
            if lower.contains("/sse") {
                "sse".to_string()
            } else if lower.contains("streamable") || lower.ends_with("/mcp") {
                "streamablehttp".to_string()
            } else {
                return Err("transport 仅支持 sse/streamablehttp".into());
            }
        }
    };
    Ok(normalized)
}

fn normalize_imported_name(
    name: &str,
    service_name: &str,
    service_id: &str,
    url: &str,
    existing_name: Option<&str>,
) -> String {
    if !name.trim().is_empty() {
        return name.trim().to_string();
    }
    if !service_name.trim().is_empty() {
        return service_name.trim().to_string();
    }
    if !service_id.trim().is_empty() {
        return service_id.trim().to_string();
    }
    if let Some(existing) = existing_name {
        if !existing.trim().is_empty() {
            return existing.trim().to_string();
        }
    }
    url.to_string()
}

fn headers_from_value(value: Option<&Value>) -> HashMap<String, String> {
    let mut headers = HashMap::new();
    let Some(Value::Object(obj)) = value else {
        return headers;
    };
    for (k, v) in obj {
        if let Some(s) = v.as_str() {
            if !s.trim().is_empty() {
                headers.insert(k.clone(), s.to_string());
            }
        }
    }
    normalize_auth_headers(&mut headers);
    headers
}

fn enrich_imported_headers(
    state: &AppState,
    market_id: Option<i64>,
    auth_required: bool,
    headers: &mut HashMap<String, String>,
) {
    if !auth_required {
        return;
    }
    if let Some(mid) = market_id {
        if let Ok(Some(row)) = state.db.get_config(mid) {
            if let Some(market) = parse_market_row(&row) {
                let (mh, _) = market_request_auth(&market);
                for (k, v) in mh {
                    headers.entry(k).or_insert(v);
                }
            }
        }
    }
    normalize_auth_headers(headers);
}

fn detail_to_json(detail: &ServiceDetail) -> Value {
    json!({
        "market_id": detail.market_id,
        "market_name": detail.market_name,
        "service_id": detail.service_id,
        "name": detail.name,
        "description": detail.description,
        "source_url": detail.source_url,
        "endpoints": detail.endpoints,
    })
}

fn flatten_imported_row(row: &crate::db::ConfigRow) -> Value {
    let mut data: Value = serde_json::from_str(&row.json_data).unwrap_or(json!({}));
    if let Some(obj) = data.as_object_mut() {
        obj.entry("id".to_string()).or_insert(json!(row.id));
        obj.entry("name".to_string())
            .or_insert(json!(row.name.clone()));
        obj.entry("enabled".to_string())
            .or_insert(json!(row.enabled));
        return Value::Object(obj.clone());
    }
    json!({
        "id": row.id,
        "name": row.name,
        "enabled": row.enabled,
        "data": data,
    })
}

fn url_hash(url: &str) -> String {
    let normalized = mcp_market::normalize_url(url);
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn resolve_unique_name(used: &HashSet<String>, base: &str) -> String {
    let mut name = base.to_string();
    let mut idx = 2;
    while used.contains(&name) {
        name = format!("{base}-{idx}");
        idx += 1;
    }
    name
}
