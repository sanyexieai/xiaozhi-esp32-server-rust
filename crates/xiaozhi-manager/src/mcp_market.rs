use std::collections::{HashMap, HashSet};
use std::time::Duration;

use reqwest::Client;
use serde_json::{json, Value};

use crate::db::ConfigRow;

const PROVIDER_MODELSCOPE: &str = "modelscope";

#[derive(Debug, Clone)]
pub struct MarketAuth {
    pub auth_type: String,
    pub header_name: String,
    pub token: String,
    pub extra_headers: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct MarketConnection {
    pub id: i64,
    pub name: String,
    pub provider_id: String,
    pub catalog_url: String,
    pub detail_url_template: String,
    pub enabled: bool,
    pub auth: MarketAuth,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ServiceSummary {
    pub market_id: i64,
    pub market_name: String,
    pub service_id: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ParsedEndpoint {
    pub name: String,
    pub transport: String,
    pub url: String,
    pub headers: HashMap<String, String>,
    /// 魔搭 operational_urls.auth_required；为 false 时不应自动注入 ms- Token。
    pub auth_required: bool,
    /// 端点来源：mcpServers / operational_urls / other（用于排序，不做 URL 硬编码替换）。
    pub source: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ServiceDetail {
    pub market_id: i64,
    pub market_name: String,
    pub service_id: String,
    pub name: String,
    pub description: String,
    pub source_url: String,
    pub endpoints: Vec<ParsedEndpoint>,
}

pub fn parse_market_row(row: &ConfigRow) -> Option<MarketConnection> {
    let mut data: Value = serde_json::from_str(&row.json_data).unwrap_or(json!({}));
    if data.is_null() {
        data = json!({});
    }

    let name = data
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(&row.name)
        .to_string();
    let provider_id = normalize_provider(
        data.get("provider_id")
            .or_else(|| data.get("provider"))
            .and_then(|v| v.as_str())
            .unwrap_or(&row.provider),
    );
    let catalog_url = data
        .get("catalog_url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if catalog_url.is_empty() {
        return None;
    }

    let detail_url_template = data
        .get("detail_url_template")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let enabled = data
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(row.enabled);

    let auth_obj = data.get("auth");
    let auth_type = auth_obj
        .and_then(|a| a.get("type"))
        .or_else(|| data.get("auth_type"))
        .and_then(|v| v.as_str())
        .unwrap_or("none")
        .to_string();
    let header_name = auth_obj
        .and_then(|a| a.get("header_name"))
        .or_else(|| data.get("header_name"))
        .and_then(|v| v.as_str())
        .unwrap_or("Authorization")
        .to_string();
    let token = auth_obj
        .and_then(|a| a.get("token"))
        .or_else(|| data.get("token_ciphertext"))
        .or_else(|| data.get("token"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mut extra_headers = HashMap::new();
    if let Some(obj) = data.get("extra_headers").and_then(|v| v.as_object()) {
        for (k, v) in obj {
            if let Some(s) = v.as_str() {
                extra_headers.insert(k.clone(), s.to_string());
            }
        }
    }

    Some(MarketConnection {
        id: row.id,
        name,
        provider_id,
        catalog_url,
        detail_url_template,
        enabled: row.enabled && enabled,
        auth: MarketAuth {
            auth_type,
            header_name,
            token,
            extra_headers,
        },
    })
}

pub fn normalize_provider(id: &str) -> String {
    let id = id.trim().to_lowercase();
    if id.is_empty() {
        "generic".to_string()
    } else {
        id
    }
}

pub fn market_request_auth(
    market: &MarketConnection,
) -> (HashMap<String, String>, HashMap<String, String>) {
    (
        build_headers(&market.auth, &market.provider_id),
        build_cookies(&market.auth, &market.provider_id),
    )
}

/// 为已导入服务补全鉴权：优先使用已存 headers，否则在 auth_required 时回退市场 Token。
pub fn resolve_imported_request_auth(
    data: &Value,
    market: Option<&MarketConnection>,
) -> (HashMap<String, String>, HashMap<String, String>) {
    let mut headers = HashMap::new();
    let mut cookies = HashMap::new();
    let auth_required = imported_auth_required(data);

    if let Some(market) = market {
        if auth_required {
            let (mh, mc) = market_request_auth(market);
            headers = mh;
            cookies = mc;
        }
    }

    if let Some(obj) = data.get("headers").and_then(|v| v.as_object()) {
        for (k, v) in obj {
            if let Some(s) = v.as_str() {
                if !s.trim().is_empty() {
                    headers.insert(k.clone(), s.to_string());
                }
            }
        }
    }
    normalize_auth_headers(&mut headers);

    (headers, cookies)
}

pub fn imported_auth_required(data: &Value) -> bool {
    data.get("auth_required")
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

pub fn merge_imported_headers(
    auth_required: bool,
    market: Option<&MarketConnection>,
    endpoint_headers: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut headers = endpoint_headers.clone();
    if auth_required {
        if let Some(market) = market {
            let (market_headers, _) = market_request_auth(market);
            for (k, v) in market_headers {
                headers.entry(k).or_insert(v);
            }
        }
    }
    normalize_auth_headers(&mut headers);
    headers
}

pub fn imported_auth_hint(data: &Value) -> Option<String> {
    let auth_required = imported_auth_required(data);
    let source_url = data
        .get("source_url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let url = data
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();

    if !auth_required && is_modelscope_inference_url(url) {
        if source_url.is_empty() {
            return Some(
                "该服务 auth_required=false，当前为魔搭托管地址，可能需改用服务商文档中的官方 MCP 接入地址，并填写服务商 Token"
                    .into(),
            );
        }
        return Some(format!(
            "该服务 auth_required=false，魔搭托管地址可能不接受服务商 Token；请查阅服务商文档 {source_url} 确认官方 MCP 接入地址"
        ));
    }
    if !auth_required && headers_missing_provider_token(data) {
        return Some(
            "该服务需服务商 Token：请在 Headers 中配置 Authorization（不是魔搭 ms- Token）".into(),
        );
    }
    if auth_required && headers_missing_provider_token(data) && is_modelscope_inference_url(url) {
        return Some(
            "请确认魔搭 Token 有效，或在 Headers 中配置正确的 Authorization".into(),
        );
    }
    None
}

fn headers_missing_provider_token(data: &Value) -> bool {
    data.get("headers")
        .and_then(|v| v.as_object())
        .and_then(|obj| obj.get("Authorization").or_else(|| obj.get("authorization")))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().is_empty())
        .unwrap_or(true)
}

pub fn is_modelscope_inference_url(url: &str) -> bool {
    url.to_lowercase()
        .contains("mcp.api-inference.modelscope.net")
}

/// 导入时按 URL 选择单条 endpoint；未指定且仅一条时自动选中。
pub fn select_endpoints_for_import<'a>(
    endpoints: &'a [ParsedEndpoint],
    endpoint_url: &str,
) -> Result<Vec<&'a ParsedEndpoint>, String> {
    if endpoints.is_empty() {
        return Err("该服务暂无可用的 SSE/StreamableHTTP 地址".into());
    }
    let want = endpoint_url.trim();
    if !want.is_empty() {
        let norm_want = normalize_url(want);
        let matched: Vec<_> = endpoints
            .iter()
            .filter(|ep| normalize_url(&ep.url) == norm_want)
            .collect();
        if matched.is_empty() {
            return Err(format!("所选接入地址不在该服务详情中: {want}"));
        }
        return Ok(matched);
    }
    if endpoints.len() == 1 {
        return Ok(vec![&endpoints[0]]);
    }
    Err("该服务有多个接入地址，请选择要导入的 endpoint".into())
}

fn is_root_http_url(url: &str) -> bool {
    url::Url::parse(url.trim())
        .ok()
        .is_some_and(|u| {
            matches!(u.scheme(), "http" | "https") && (u.path().is_empty() || u.path() == "/")
        })
}

/// 导入/展示时排序：优先 mcpServers 配置，其次非魔搭 inference 托管地址。
pub fn prioritize_endpoints(endpoints: &mut [ParsedEndpoint]) {
    endpoints.sort_by(|a, b| {
        endpoint_rank(b)
            .cmp(&endpoint_rank(a))
            .then(a.transport.cmp(&b.transport))
            .then(a.url.cmp(&b.url))
    });
}

fn endpoint_rank(ep: &ParsedEndpoint) -> i32 {
    let mut score = 0;
    if ep.source == "mcpServers" {
        score += 100;
    } else if ep.source == "operational_urls" {
        score += 10;
    }
    if !is_modelscope_inference_url(&ep.url) {
        score += 50;
    }
    if !ep.auth_required {
        score += 5;
    }
    score
}

pub fn endpoint_import_warning(ep: &ParsedEndpoint, source_url: &str) -> Option<String> {
    if ep.auth_required || !is_modelscope_inference_url(&ep.url) {
        return None;
    }
    if source_url.trim().is_empty() {
        Some(format!(
            "已导入魔搭托管地址 {}；auth_required=false，若探测失败请查阅服务商文档，改用官方 MCP 地址并填写服务商 Token",
            ep.url
        ))
    } else {
        Some(format!(
            "已导入魔搭托管地址 {}；auth_required=false，若探测失败请查阅 {} 确认官方 MCP 接入地址",
            ep.url, source_url.trim()
        ))
    }
}

pub async fn fetch_catalog(
    market: &MarketConnection,
) -> Result<Value, String> {
    let endpoint = resolve_catalog_endpoint(market)?;
    let headers = build_headers(&market.auth, &market.provider_id);
    let cookies = build_cookies(&market.auth, &market.provider_id);
    fetch_json(&endpoint, "GET", None, &headers, &cookies).await
}

pub async fn fetch_service_detail(
    market: &MarketConnection,
    service_id: &str,
) -> Result<ServiceDetail, String> {
    let mut detail = fetch_service_detail_raw(market, service_id).await?;
    if detail.endpoints.is_empty() {
        detail.endpoints = endpoints_from_operational_catalog(market, service_id).await;
    }
    if detail.endpoints.is_empty() && market.provider_id == PROVIDER_MODELSCOPE {
        return Err(
            "该服务暂无可用的 SSE/StreamableHTTP 地址，请先在魔搭完成激活/部署后重试".into(),
        );
    }
    Ok(detail)
}

async fn fetch_service_detail_raw(
    market: &MarketConnection,
    service_id: &str,
) -> Result<ServiceDetail, String> {
    let mut detail_url =
        build_detail_url(&market.catalog_url, &market.detail_url_template, service_id)?;
    if market.provider_id == PROVIDER_MODELSCOPE {
        if market.auth.token.trim().is_empty() {
            return Err("魔搭服务详情需要 Token，请在该市场连接中填写 Token".into());
        }
        detail_url = append_query_param(&detail_url, "get_operational_url", "true");
    }
    let headers = build_headers(&market.auth, &market.provider_id);
    let cookies = build_cookies(&market.auth, &market.provider_id);
    let raw = fetch_json(&detail_url, "GET", None, &headers, &cookies).await?;
    parse_service_detail(&raw, market, service_id)
}

/// 详情接口未返回地址时，从 `/operational` 列表项中补全（魔搭常见：列表含 operational_urls，详情为空）。
async fn endpoints_from_operational_catalog(
    market: &MarketConnection,
    service_id: &str,
) -> Vec<ParsedEndpoint> {
    let raw = match fetch_catalog(market).await {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let headers = build_headers(&market.auth, &market.provider_id);
    find_catalog_item(&raw, service_id)
        .map(|item| extract_mcp_endpoints(&item, &headers))
        .unwrap_or_default()
}

fn find_catalog_item(payload: &Value, service_id: &str) -> Option<Value> {
    let want = service_id.trim();
    for item in extract_service_list(payload) {
        let (id, _, _) = parse_service_summary(&item);
        if ids_equivalent(&id, want) {
            return Some(item);
        }
    }
    None
}

fn ids_equivalent(a: &str, b: &str) -> bool {
    let a = a.trim();
    let b = b.trim();
    if a == b {
        return true;
    }
    normalize_service_id(a) == normalize_service_id(b)
}

fn normalize_service_id(id: &str) -> String {
    let id = id.trim();
    if let Some(rest) = id.strip_prefix('@') {
        rest.to_string()
    } else {
        id.to_string()
    }
}

/// 对齐 Go `fetchMarketCatalog`：魔搭走 `/operational` 且必须带 Token。
fn resolve_catalog_endpoint(market: &MarketConnection) -> Result<String, String> {
    let base = market.catalog_url.trim();
    if base.is_empty() {
        return Err("catalog_url 不能为空".into());
    }
    if market.provider_id == PROVIDER_MODELSCOPE {
        if market.auth.token.trim().is_empty() {
            return Err("魔搭市场仅拉取已激活服务，请先在该市场连接中填写 Token".into());
        }
        return Ok(format!("{}/operational", base.trim_end_matches('/')));
    }
    Ok(base.to_string())
}

fn append_query_param(url: &str, key: &str, value: &str) -> String {
    let sep = if url.contains('?') { '&' } else { '?' };
    format!("{url}{sep}{key}={}", urlencoding::encode(value))
}

fn market_http_client() -> Result<Client, String> {
    Client::builder()
        .timeout(Duration::from_secs(15))
        // Windows 上系统代理常指向 127.0.0.1，会导致魔搭域名连接失败
        .no_proxy()
        .user_agent(concat!("xiaozhi-manager/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())
}

pub async fn fetch_all_services(
    markets: &[MarketConnection],
) -> (Vec<ServiceSummary>, Vec<String>) {
    let mut items = Vec::new();
    let mut warnings = Vec::new();

    for market in markets {
        if !market.enabled {
            continue;
        }
        match fetch_catalog(market).await {
            Ok(raw) => {
                for item in extract_service_list(&raw) {
                    let (service_id, name, description) = parse_service_summary(&item);
                    if service_id.is_empty() || name.is_empty() {
                        continue;
                    }
                    items.push(ServiceSummary {
                        market_id: market.id,
                        market_name: market.name.clone(),
                        service_id,
                        name,
                        description,
                    });
                }
            }
            Err(e) => warnings.push(format!("{}: {e}", market.name)),
        }
    }

    (dedupe_services(items), warnings)
}

fn build_headers(auth: &MarketAuth, provider_id: &str) -> HashMap<String, String> {
    let mut headers = auth.extra_headers.clone();
    let token = auth.token.trim();
    if token.is_empty() {
        return headers;
    }

    let auth_type = auth.auth_type.to_lowercase();
    if auth_type == "header" {
        let key = if auth.header_name.is_empty() {
            "Authorization"
        } else {
            &auth.header_name
        };
        headers.insert(key.to_string(), token.to_string());
    } else if provider_id == PROVIDER_MODELSCOPE || auth_type == "bearer" || auth_type.is_empty() {
        headers.insert("Authorization".to_string(), format_bearer_token(token));
    }
    headers
}

/// 避免 `Bearer Bearer xxx` 导致魔搭推理端 401。
pub fn format_bearer_token(token: &str) -> String {
    let token = token.trim();
    if token.is_empty() {
        return String::new();
    }
    if token.to_lowercase().starts_with("bearer ") {
        token.to_string()
    } else {
        format!("Bearer {token}")
    }
}

pub fn normalize_auth_headers(headers: &mut HashMap<String, String>) {
    let auth_key = headers
        .keys()
        .find(|k| k.eq_ignore_ascii_case("authorization"))
        .cloned();
    if let Some(key) = auth_key {
        if let Some(val) = headers.remove(&key) {
            headers.insert("Authorization".to_string(), format_bearer_token(&val));
        }
    }
}

fn build_cookies(auth: &MarketAuth, provider_id: &str) -> HashMap<String, String> {
    let token = auth.token.trim();
    if token.is_empty() {
        return HashMap::new();
    }
    if provider_id == PROVIDER_MODELSCOPE {
        let mut m = HashMap::new();
        m.insert("m_session_id".to_string(), token.to_string());
        return m;
    }
    HashMap::new()
}

async fn fetch_json(
    endpoint: &str,
    method: &str,
    body: Option<Value>,
    headers: &HashMap<String, String>,
    cookies: &HashMap<String, String>,
) -> Result<Value, String> {
    let client = market_http_client()?;

    let mut req = match method.to_uppercase().as_str() {
        "POST" => client.post(endpoint),
        _ => client.get(endpoint),
    };
    req = req.header("Accept", "application/json");
    if let Some(b) = body {
        req = req.json(&b);
    }
    for (k, v) in headers {
        req = req.header(k, v);
    }
    for (k, v) in cookies {
        req = req.header("Cookie", format!("{k}={v}"));
    }

    let resp = req.send().await.map_err(|e| format!("请求失败: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| format!("读取响应失败: {e}"))?;
    if !status.is_success() {
        let msg = if text.trim().is_empty() {
            status.to_string()
        } else {
            text
        };
        return Err(format!("请求失败({status}): {msg}"));
    }
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(&text).map_err(|e| format!("解析JSON失败: {e}"))
}

fn build_detail_url(catalog_url: &str, template: &str, service_id: &str) -> Result<String, String> {
    let service_id = service_id.trim();
    if service_id.is_empty() {
        return Err("service_id 不能为空".into());
    }
    let template = template.trim();
    if !template.is_empty() {
        let url = template
            .replace("{raw_id}", service_id)
            .replace("{id}", &urlencoding::encode(service_id));
        return Ok(url);
    }
    let base = catalog_url.trim_end_matches('/');
    Ok(format!("{base}/{}", urlencoding::encode(service_id)))
}

fn extract_service_list(payload: &Value) -> Vec<Value> {
    find_first_object_array(payload).unwrap_or_default()
}

fn find_first_object_array(v: &Value) -> Option<Vec<Value>> {
    match v {
        Value::Array(arr) => {
            if arr.iter().all(|x| x.is_object()) {
                Some(arr.clone())
            } else {
                None
            }
        }
        Value::Object(map) => {
            for key in [
                "data",
                "items",
                "services",
                "list",
                "results",
                "records",
                "mcp_server_list",
            ] {
                if let Some(next) = map.get(key) {
                    if let Some(found) = find_first_object_array(next) {
                        return Some(found);
                    }
                }
            }
            for val in map.values() {
                if let Some(found) = find_first_object_array(val) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

fn parse_service_summary(item: &Value) -> (String, String, String) {
    let obj = item.as_object();
    let id = first_string(obj, &[
        "service_id", "id", "slug", "name", "serviceName", "serviceId", "tool_id", "toolId",
    ]);
    let name = first_string(obj, &[
        "name", "title", "service_name", "serviceName", "tool_name", "toolName", "id",
    ]);
    let description = first_string(obj, &["description", "desc", "summary", "intro", "detail"]);
    let id = if id.is_empty() { name.clone() } else { id };
    let name = if name.is_empty() { id.clone() } else { name };
    (id, name, description)
}

fn parse_service_detail(
    payload: &Value,
    market: &MarketConnection,
    service_id: &str,
) -> Result<ServiceDetail, String> {
    let headers = build_headers(&market.auth, &market.provider_id);
    let mut endpoints = extract_mcp_endpoints(payload, &headers);
    if endpoints.is_empty() {
        if let Some(inner) = unwrap_openapi_data(payload) {
            endpoints = extract_mcp_endpoints(inner, &headers);
        }
    }

    let focus = unwrap_openapi_data(payload).unwrap_or(payload);
    let obj = focus.as_object();
    let name = first_string(obj, &["name", "title", "service_name", "serviceName"]);
    let description = first_string(obj, &["description", "desc", "summary", "intro"]);
    let source_url = first_string(obj, &["source_url", "sourceUrl", "homepage", "home_page"]);
    let name = if name.is_empty() {
        service_id.to_string()
    } else {
        name
    };
    prioritize_endpoints(&mut endpoints);
    Ok(ServiceDetail {
        market_id: market.id,
        market_name: market.name.clone(),
        service_id: service_id.to_string(),
        name,
        description,
        source_url,
        endpoints,
    })
}

fn unwrap_openapi_data<'a>(payload: &'a Value) -> Option<&'a Value> {
    let obj = payload.as_object()?;
    obj.get("data")
        .or_else(|| obj.get("Data"))
        .filter(|v| v.is_object() || v.is_array())
}

fn extract_mcp_endpoints(payload: &Value, headers: &HashMap<String, String>) -> Vec<ParsedEndpoint> {
    let mut ret = Vec::new();
    let mut seen = HashSet::new();
    walk_any(payload, &mut |node| {
        let Some(map) = node.as_object() else {
            return;
        };

        if let Some(servers) = map.get("mcpServers").and_then(|v| v.as_object()) {
            for (name, cfg) in servers {
                if let Some(cfg) = cfg.as_object() {
                    let transport = first_string(
                        Some(cfg),
                        &["type", "transport", "protocol", "transport_type"],
                    );
                    let url = first_string(
                        Some(cfg),
                        &["url", "endpoint", "sse_url", "sseUrl"],
                    );
                    let auth_required = endpoint_auth_required(cfg, "mcpServers", &url);
                    let base = if auth_required {
                        headers
                    } else {
                        &HashMap::new()
                    };
                    let ep_headers = merge_endpoint_headers(base, cfg);
                    push_endpoint(
                        &mut ret,
                        &mut seen,
                        name,
                        &transport,
                        &url,
                        &ep_headers,
                        auth_required,
                        "mcpServers",
                    );
                }
            }
        }

        for key in [
            "operational_urls",
            "mcp_servers",
            "servers",
            "endpoints",
        ] {
            if let Some(node) = map.get(key) {
                append_endpoints_from_node(&mut ret, &mut seen, node, headers, key);
            }
        }

        if let Some(url) = map
            .get("operational_url")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
        {
            let name = first_string(Some(map), &["name", "title"]);
            let auth_required = endpoint_auth_required(map, "operational_url", url);
            push_endpoint(
                &mut ret,
                &mut seen,
                &name,
                "",
                url,
                headers,
                auth_required,
                "operational_url",
            );
        }

        let transport = first_string(
            Some(map),
            &["type", "transport", "protocol", "transport_type"],
        );
        let url = first_string(Some(map), &["url", "endpoint", "mcp_url", "mcpUrl"]);
        if !url.is_empty() {
            let name = first_string(Some(map), &["name", "title"]);
            let auth_required = endpoint_auth_required(map, "other", &url);
            let base = if auth_required {
                headers
            } else {
                &HashMap::new()
            };
            let ep_headers = merge_endpoint_headers(base, map);
            push_endpoint(
                &mut ret,
                &mut seen,
                &name,
                &transport,
                &url,
                &ep_headers,
                auth_required,
                "other",
            );
        }
        let sse = first_string(Some(map), &["sse_url", "sseUrl", "sse"]);
        if !sse.is_empty() {
            let name = first_string(Some(map), &["name", "title"]);
            let auth_required = endpoint_auth_required(map, "other", &sse);
            push_endpoint(
                &mut ret,
                &mut seen,
                &name,
                "sse",
                &sse,
                headers,
                auth_required,
                "other",
            );
        }
        let sh = first_string(
            Some(map),
            &["streamablehttp", "streamable_http", "streamable-http"],
        );
        if !sh.is_empty() {
            let name = first_string(Some(map), &["name", "title"]);
            let auth_required = endpoint_auth_required(map, "other", &sh);
            push_endpoint(
                &mut ret,
                &mut seen,
                &name,
                "streamablehttp",
                &sh,
                headers,
                auth_required,
                "other",
            );
        }
    });
    prioritize_endpoints(&mut ret);
    ret
}

fn append_endpoints_from_node(
    ret: &mut Vec<ParsedEndpoint>,
    seen: &mut HashSet<String>,
    node: &Value,
    headers: &HashMap<String, String>,
    source: &str,
) {
    match node {
        Value::Array(arr) => {
            for item in arr {
                append_endpoint_value(ret, seen, item, headers, source);
            }
        }
        Value::Object(map) => {
            for (key, val) in map {
                if let Some(url) = val.as_str() {
                    if url.trim().is_empty() {
                        continue;
                    }
                    let transport = if key.contains("sse") {
                        "sse"
                    } else if key.contains("streamable") || key == "http" {
                        "streamablehttp"
                    } else {
                        ""
                    };
                    push_endpoint(ret, seen, key, transport, url, headers, true, source);
                }
            }
        }
        Value::String(_) => {
            append_endpoint_value(ret, seen, node, headers, source);
        }
        _ => {}
    }
}

fn append_endpoint_value(
    ret: &mut Vec<ParsedEndpoint>,
    seen: &mut HashSet<String>,
    item: &Value,
    headers: &HashMap<String, String>,
    source: &str,
) {
    if let Some(url) = item.as_str() {
        if !url.trim().is_empty() {
            push_endpoint(ret, seen, "", "", url, headers, true, source);
        }
        return;
    }
    let Some(map) = item.as_object() else {
        return;
    };
    let url = first_string(Some(map), &["url", "endpoint", "sse_url", "sseUrl", "mcp_url"]);
    let transport = first_string(
        Some(map),
        &["type", "transport", "protocol", "transport_type"],
    );
    let name = first_string(Some(map), &["name", "title"]);
    let auth_required = endpoint_auth_required(map, source, &url);
    let base = if auth_required {
        headers
    } else {
        &HashMap::new()
    };
    let ep_headers = merge_endpoint_headers(base, map);
    push_endpoint(
        ret,
        seen,
        &name,
        &transport,
        &url,
        &ep_headers,
        auth_required,
        source,
    );
}

fn endpoint_auth_required(
    map: &serde_json::Map<String, Value>,
    source: &str,
    url: &str,
) -> bool {
    if let Some(v) = map.get("auth_required").and_then(|v| v.as_bool()) {
        return v;
    }
    if source == "mcpServers" && !is_modelscope_inference_url(url) {
        return false;
    }
    true
}

fn merge_endpoint_headers(
    base: &HashMap<String, String>,
    node: &serde_json::Map<String, Value>,
) -> HashMap<String, String> {
    let mut out = base.clone();
    if let Some(obj) = node.get("headers").and_then(|v| v.as_object()) {
        for (k, v) in obj {
            if let Some(s) = v.as_str() {
                if !s.trim().is_empty() {
                    out.insert(k.clone(), s.to_string());
                }
            }
        }
    }
    normalize_auth_headers(&mut out);
    out
}

fn push_endpoint(
    ret: &mut Vec<ParsedEndpoint>,
    seen: &mut HashSet<String>,
    name: &str,
    transport: &str,
    url: &str,
    headers: &HashMap<String, String>,
    auth_required: bool,
    source: &str,
) {
    let url = url.trim();
    if url.is_empty() {
        return;
    }
    let mut transport = normalize_transport(transport, url);
    if transport != "sse" && transport != "streamablehttp" {
        transport = infer_transport_from_url(url, source);
    }
    if transport != "sse" && transport != "streamablehttp" {
        return;
    }
    let key = format!("{transport}|{}", normalize_url_key(url));
    if !seen.insert(key) {
        return;
    }
    ret.push(ParsedEndpoint {
        name: name.to_string(),
        transport,
        url: url.to_string(),
        headers: headers.clone(),
        auth_required,
        source: source.to_string(),
    });
}

#[cfg(test)]
mod endpoint_header_tests {
    use super::*;

    #[test]
    fn merge_endpoint_headers_from_mcp_servers() {
        let base = HashMap::from([(
            "Authorization".to_string(),
            "Bearer catalog-token".to_string(),
        )]);
        let cfg = json!({
            "url": "https://mcp.api-inference.modelscope.net/uuid/mcp",
            "headers": { "Authorization": "Bearer ms-abc-123" }
        })
        .as_object()
        .unwrap()
        .clone();
        let merged = merge_endpoint_headers(&base, &cfg);
        assert_eq!(
            merged.get("Authorization").map(String::as_str),
            Some("Bearer ms-abc-123")
        );
    }
}

fn infer_transport_from_url(url: &str, source: &str) -> String {
    let lower = url.trim().to_lowercase();
    if lower.ends_with("/sse") || lower.contains("/sse?") {
        return "sse".to_string();
    }
    if lower.ends_with("/streamable_http")
        || lower.contains("/streamable_http?")
        || lower.ends_with("/streamablehttp")
        || lower.contains("/streamablehttp?")
    {
        return "streamablehttp".to_string();
    }
    if lower.ends_with("/mcp") || lower.contains("/mcp?") {
        return "streamablehttp".to_string();
    }
    if source == "mcpServers" && is_root_http_url(url) {
        return "streamablehttp".to_string();
    }
    String::new()
}

fn normalize_url_key(url: &str) -> String {
    normalize_url(url)
}

fn normalize_transport(transport: &str, url: &str) -> String {
    let t = transport.trim().to_lowercase();
    match t.as_str() {
        "sse" => "sse".to_string(),
        "streamablehttp" | "streamable_http" | "http" | "streamable-http" => {
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

fn walk_any(v: &Value, visit: &mut dyn FnMut(&Value)) {
    visit(v);
    match v {
        Value::Object(map) => {
            for child in map.values() {
                walk_any(child, visit);
            }
        }
        Value::Array(arr) => {
            for child in arr {
                walk_any(child, visit);
            }
        }
        _ => {}
    }
}

fn first_string(obj: Option<&serde_json::Map<String, Value>>, keys: &[&str]) -> String {
    let Some(map) = obj else {
        return String::new();
    };
    for key in keys {
        if let Some(val) = map.get(*key) {
            if let Some(s) = val.as_str() {
                if !s.trim().is_empty() {
                    return s.trim().to_string();
                }
            }
        }
    }
    String::new()
}

fn dedupe_services(items: Vec<ServiceSummary>) -> Vec<ServiceSummary> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in items {
        let key = format!("{}|{}", item.market_id, item.service_id);
        if seen.insert(key) {
            out.push(item);
        }
    }
    out
}

pub fn normalize_url(raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return String::new();
    }
    let mut url = raw.to_string();
    if let Ok(parsed) = url::Url::parse(raw) {
        let mut normalized = parsed.clone();
        if let Some(host) = parsed.host_str() {
            normalized.set_host(Some(host)).ok();
        }
        let path = parsed.path().trim_end_matches('/');
        normalized.set_path(if path.is_empty() { "/" } else { path });
        url = normalized.to_string();
    }
    url.trim_end_matches('/').to_string()
}

pub fn filter_services(items: &[ServiceSummary], query: &str) -> Vec<ServiceSummary> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return items.to_vec();
    }
    items
        .iter()
        .filter(|s| {
            s.service_id.to_lowercase().contains(&q)
                || s.name.to_lowercase().contains(&q)
                || s.description.to_lowercase().contains(&q)
                || s.market_name.to_lowercase().contains(&q)
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_modelscope_operational_urls() {
        let payload = json!({
            "data": {
                "id": "@demo/ServerA",
                "name": "ServerA",
                "operational_urls": [
                    {
                        "url": "https://mcp.api-inference.modelscope.net/uuid/sse",
                        "auth_required": true
                    },
                    {
                        "url": "https://mcp.api-inference.modelscope.net/uuid/streamable_http",
                        "auth_required": false
                    }
                ]
            }
        });
        let eps = extract_mcp_endpoints(&payload, &HashMap::from([(
            "Authorization".to_string(),
            "Bearer ms-test".to_string(),
        )]));
        assert_eq!(eps.len(), 2);
        assert!(eps.iter().any(|e| e.transport == "sse" && e.auth_required));
        let no_auth = eps
            .iter()
            .find(|e| e.transport == "streamablehttp")
            .expect("streamable");
        assert!(!no_auth.auth_required);
        assert!(no_auth.headers.is_empty());
    }

    #[test]
    fn extract_operational_urls_object_map() {
        let payload = json!({
            "operational_urls": {
                "sse": "https://example.com/mcp/sse",
                "streamable_http": "https://example.com/mcp/streamable_http"
            }
        });
        let eps = extract_mcp_endpoints(&payload, &HashMap::new());
        assert_eq!(eps.len(), 2);
    }

    #[test]
    fn find_catalog_item_by_id() {
        let payload = json!({
            "data": {
                "mcp_server_list": [
                    {
                        "id": "@modelscope/demo",
                        "name": "demo",
                        "operational_urls": [{"url": "https://example.com/sse"}]
                    }
                ]
            }
        });
        let item = find_catalog_item(&payload, "@modelscope/demo").expect("item");
        let eps = extract_mcp_endpoints(&item, &HashMap::new());
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].transport, "sse");
    }

    #[test]
    fn prioritize_mcp_servers_over_inference_proxy() {
        let mut eps = vec![
            ParsedEndpoint {
                name: "proxy".into(),
                transport: "streamablehttp".into(),
                url: "https://mcp.api-inference.modelscope.net/uuid/mcp".into(),
                headers: HashMap::new(),
                auth_required: false,
                source: "operational_urls".into(),
            },
            ParsedEndpoint {
                name: "direct".into(),
                transport: "streamablehttp".into(),
                url: "https://provider.example.com/mcp".into(),
                headers: HashMap::new(),
                auth_required: false,
                source: "mcpServers".into(),
            },
        ];
        prioritize_endpoints(&mut eps);
        assert_eq!(eps[0].url, "https://provider.example.com/mcp");
    }

    #[test]
    fn extract_mcp_servers_root_https_url() {
        let payload = json!({
            "mcpServers": {
                "mcd": {
                    "url": "https://mcp.mcd.cn"
                }
            }
        });
        let eps = extract_mcp_endpoints(&payload, &HashMap::new());
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].transport, "streamablehttp");
        assert!(!eps[0].auth_required);
        assert_eq!(eps[0].source, "mcpServers");
    }

    #[test]
    fn select_endpoints_for_import_requires_choice_when_multiple() {
        let eps = vec![
            ParsedEndpoint {
                name: "a".into(),
                transport: "sse".into(),
                url: "https://a.example/sse".into(),
                headers: HashMap::new(),
                auth_required: true,
                source: "operational_urls".into(),
            },
            ParsedEndpoint {
                name: "b".into(),
                transport: "streamablehttp".into(),
                url: "https://b.example/mcp".into(),
                headers: HashMap::new(),
                auth_required: false,
                source: "mcpServers".into(),
            },
        ];
        assert!(select_endpoints_for_import(&eps, "").is_err());
        let picked = select_endpoints_for_import(&eps, "https://b.example/mcp").expect("pick");
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].url, "https://b.example/mcp");
    }
}
