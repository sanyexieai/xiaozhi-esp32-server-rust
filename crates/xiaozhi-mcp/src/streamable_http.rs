//! Streamable HTTP MCP 传输：initialize → mcp-session-id → notifications/initialized → RPC

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use reqwest::{Client, Response, StatusCode};
use serde_json::{json, Value};

const MCP_SESSION_HEADER: &str = "mcp-session-id";

pub struct StreamableHttpClient {
    client: Client,
    url: String,
    headers: HashMap<String, String>,
    cookies: HashMap<String, String>,
    /// 服务端在 initialize 响应头中下发的 session（魔搭 inference 等需要）
    session_id: Option<String>,
    /// 已完成 initialize 握手（含无 session 的无状态服务，如麦当劳官方端点）
    initialized: bool,
    next_id: AtomicU64,
}

impl StreamableHttpClient {
    pub fn new(
        client: Client,
        url: impl Into<String>,
        headers: HashMap<String, String>,
        cookies: HashMap<String, String>,
    ) -> Self {
        Self {
            client,
            url: url.into(),
            headers,
            cookies,
            session_id: None,
            initialized: false,
            next_id: AtomicU64::new(1),
        }
    }

    pub async fn open_session(&mut self) -> Result<(), String> {
        if self.initialized {
            return Ok(());
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "xiaozhi-manager", "version": "1.0.0" }
            }
        });

        let (body, session_id) = self.post_message(payload).await?;
        if let Some(sid) = session_id {
            self.session_id = Some(sid);
        }

        if let Some(err) = body.get("error") {
            return Err(format!("MCP initialize 失败: {err}"));
        }
        if body.get("result").is_none() && self.session_id.is_none() {
            return Err("MCP initialize 无 result 且未返回 mcp-session-id".into());
        }

        let initialized = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        });
        let _ = self.post_message(initialized).await?;
        self.initialized = true;
        Ok(())
    }

    pub fn session_id(&self) -> Option<String> {
        self.session_id.clone()
    }

    pub async fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        if !self.initialized {
            self.open_session().await?;
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        let (body, session_id) = self.post_message(payload).await?;
        if let Some(sid) = session_id {
            self.session_id = Some(sid);
        }
        if let Some(err) = body.get("error") {
            return Err(format!("MCP {method} 失败: {err}"));
        }
        Ok(body.get("result").cloned().unwrap_or(Value::Null))
    }

    async fn post_message(&self, payload: Value) -> Result<(Value, Option<String>), String> {
        let mut req = self
            .client
            .post(&self.url)
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .json(&payload);

        req = apply_headers(req, &self.headers, &self.cookies);
        if let Some(sid) = &self.session_id {
            req = req.header(MCP_SESSION_HEADER, sid.as_str());
        }

        let resp = req.send().await.map_err(|e| e.to_string())?;
        parse_http_response(resp).await
    }
}

pub fn apply_headers(
    mut req: reqwest::RequestBuilder,
    headers: &HashMap<String, String>,
    cookies: &HashMap<String, String>,
) -> reqwest::RequestBuilder {
    for (k, v) in headers {
        if k.eq_ignore_ascii_case(MCP_SESSION_HEADER) {
            continue;
        }
        req = req.header(k.as_str(), v.as_str());
    }
    if !cookies.is_empty() {
        let cookie = cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ");
        req = req.header("Cookie", cookie);
    }
    req
}

pub async fn parse_http_response(resp: Response) -> Result<(Value, Option<String>), String> {
    let status = resp.status();
    let session_id = extract_session_id(&resp);
    let text = resp.text().await.map_err(|e| e.to_string())?;

    if !status.is_success() && status != StatusCode::ACCEPTED {
        let detail = text.trim();
        if detail.is_empty() {
            return Err(format!("HTTP {status}"));
        }
        return Err(format!("HTTP {status}: {detail}"));
    }

    if text.trim().is_empty() {
        return Ok((json!({}), session_id));
    }

    let body = parse_json_rpc_payload(&text)?;
    Ok((body, session_id))
}

pub fn extract_session_id(resp: &Response) -> Option<String> {
    for (name, value) in resp.headers().iter() {
        if name.as_str().eq_ignore_ascii_case(MCP_SESSION_HEADER) {
            return value.to_str().ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        }
    }
    None
}

pub fn parse_json_rpc_payload(text: &str) -> Result<Value, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(json!({}));
    }
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return serde_json::from_str(trimmed)
            .map_err(|e| format!("解析 JSON-RPC 响应失败: {e}; body={trimmed}"));
    }

    let mut last_data: Option<&str> = None;
    for line in trimmed.lines() {
        let line = line.trim();
        if let Some(data) = line.strip_prefix("data:") {
            last_data = Some(data.trim());
        }
    }
    if let Some(data) = last_data {
        if data.is_empty() {
            return Ok(json!({}));
        }
        return serde_json::from_str(data)
            .map_err(|e| format!("解析 SSE data 失败: {e}; data={data}"));
    }

    Err(format!("无法解析 MCP 响应: {trimmed}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sse_initialize_result() {
        let body = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2024-11-05\"}}\n\n";
        let v = parse_json_rpc_payload(body).unwrap();
        assert!(v.get("result").is_some());
    }

    #[test]
    fn parses_plain_json() {
        let body = r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[]}}"#;
        let v = parse_json_rpc_payload(body).unwrap();
        assert!(v.get("result").is_some());
    }
}
