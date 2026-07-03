use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use tokio::sync::{oneshot, Mutex};
use xiaozhi_config::{McpGlobalConfig, McpServerEntry};
use crate::streamable_http::StreamableHttpClient;
use crate::types::McpTool;

#[derive(Clone)]
struct GlobalToolBinding {
    server_name: String,
    tool: McpTool,
    call_url: String,
    transport: String,
    headers: HashMap<String, String>,
}

pub struct GlobalMcpHub {
    bindings: DashMap<String, GlobalToolBinding>,
    http_sessions: DashMap<String, String>,
    client: Client,
    next_id: AtomicU64,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    shutdown: Arc<AtomicBool>,
}

impl GlobalMcpHub {
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }

    pub fn start(config: &McpGlobalConfig) -> Arc<Self> {
        let hub = Arc::new(Self {
            bindings: DashMap::new(),
            http_sessions: DashMap::new(),
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .no_proxy()
                .build()
                .unwrap_or_else(|_| Client::new()),
            next_id: AtomicU64::new(100),
            pending: Arc::new(Mutex::new(HashMap::new())),
            shutdown: Arc::new(AtomicBool::new(false)),
        });

        if !config.enabled {
            return hub;
        }

        for server in config.servers.iter().filter(|s| s.enabled && !s.url.is_empty()) {
            let hub = hub.clone();
            let server = server.clone();
            tokio::spawn(async move {
                hub.maintain_server(server).await;
            });
        }

        hub
    }

    pub fn list_tools(&self) -> Vec<McpTool> {
        self.bindings
            .iter()
            .map(|e| e.value().tool.clone())
            .collect()
    }

    pub fn list_tools_with_server(&self) -> Vec<(String, McpTool)> {
        self.bindings
            .iter()
            .map(|e| {
                let binding = e.value();
                (binding.server_name.clone(), binding.tool.clone())
            })
            .collect()
    }

    pub fn tool_count(&self) -> usize {
        self.bindings.len()
    }

    pub fn has_tool(&self, name: &str) -> bool {
        self.bindings.contains_key(name)
    }

    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<String, String> {
        let result = self.call_tool_raw(name, arguments).await?;
        Ok(format_tool_result(Some(&result)))
    }

    pub async fn call_tool_raw(&self, name: &str, arguments: Value) -> Result<Value, String> {
        let binding = self
            .bindings
            .get(name)
            .ok_or_else(|| format!("未找到全局工具: {name}"))?
            .clone();

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": binding.tool.name, "arguments": arguments }
        });

        let session_id = self.http_sessions.get(&binding.server_name).map(|s| s.clone());
        let resp = self
            .rpc_call(
                &binding.call_url,
                &binding.transport,
                id,
                payload,
                &binding.headers,
                session_id.as_deref(),
            )
            .await?;

        if let Some(err) = resp.get("error") {
            return Err(format!("全局 MCP 调用失败: {err}"));
        }
        Ok(resp.get("result").cloned().unwrap_or(json!({})))
    }

    pub async fn read_resource(
        &self,
        tool_name: &str,
        uri: &str,
        arguments: Value,
    ) -> Result<Value, String> {
        let binding = self
            .bindings
            .get(tool_name)
            .ok_or_else(|| format!("未找到全局工具: {tool_name}"))?
            .clone();

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "resources/read",
            "params": {
                "uri": uri,
                "arguments": arguments
            }
        });

        let session_id = self.http_sessions.get(&binding.server_name).map(|s| s.clone());
        let resp = self
            .rpc_call(
                &binding.call_url,
                &binding.transport,
                id,
                payload,
                &binding.headers,
                session_id.as_deref(),
            )
            .await?;

        if let Some(err) = resp.get("error") {
            return Err(format!("MCP 资源读取失败: {err}"));
        }
        Ok(resp.get("result").cloned().unwrap_or(json!({})))
    }

    async fn maintain_server(self: Arc<Self>, server: McpServerEntry) {
        let interval = Duration::from_secs(60);
        loop {
            if self.shutdown.load(Ordering::Relaxed) {
                break;
            }
            let transport = normalize_transport(&server.r#type, &server.url);
            let result = if transport == "sse" {
                self.run_sse_server(&server).await
            } else {
                self.refresh_http_server(&server).await.map(|_| ())
            };

            match result {
                Ok(()) => tracing::debug!("全局 MCP {} 连接结束，将重连", server.name),
                Err(e) => tracing::warn!("全局 MCP {} 异常: {e}", server.name),
            }
            if self.shutdown.load(Ordering::Relaxed) {
                break;
            }
            tokio::time::sleep(interval).await;
        }
    }

    async fn refresh_http_server(&self, server: &McpServerEntry) -> Result<usize, String> {
        let mut client = StreamableHttpClient::new(
            self.client.clone(),
            &server.url,
            server.headers.clone(),
            HashMap::new(),
        );
        client.open_session().await?;
        let result = client.call("tools/list", json!({})).await?;
        if let Some(sid) = client.session_id() {
            self.http_sessions.insert(server.name.clone(), sid);
        }
        let tools = parse_tools_list(Some(&result));
        Ok(self.store_tools(server, &server.url, "http", tools))
    }

    async fn run_sse_server(self: &Arc<Self>, server: &McpServerEntry) -> Result<(), String> {
        let mut req = self
            .client
            .get(&server.url)
            .header("Accept", "text/event-stream");
        req = apply_request_headers(req, &server.headers);
        let resp = req.send().await.map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            return Err(format!("SSE 连接失败: {}", resp.status()));
        }

        let base = base_url(&server.url);
        let hub = Arc::clone(self);
        let pending_for_reader = Arc::clone(&self.pending);
        let (endpoint_tx, endpoint_rx) = oneshot::channel::<String>();
        let mut stream = resp.bytes_stream();

        let read_task = tokio::spawn(async move {
            let mut event = String::new();
            let mut data = String::new();
            let mut endpoint_tx = Some(endpoint_tx);
            while let Some(item) = stream.next().await {
                let chunk = match item {
                    Ok(b) => b,
                    Err(_) => break,
                };
                for line in String::from_utf8_lossy(&chunk).lines() {
                    if let Some(ev) = line.strip_prefix("event:") {
                        event = ev.trim().to_string();
                    } else if let Some(d) = line.strip_prefix("data:") {
                        data = d.trim().to_string();
                    } else if line.is_empty() && !data.is_empty() {
                        if event == "endpoint" {
                            if let Some(tx) = endpoint_tx.take() {
                                let post_url = if data.starts_with("http") {
                                    data.clone()
                                } else {
                                    format!(
                                        "{}/{}",
                                        base.trim_end_matches('/'),
                                        data.trim_start_matches('/')
                                    )
                                };
                                let _ = tx.send(post_url);
                            }
                        } else if event == "message" {
                            if let Ok(json) = serde_json::from_str::<Value>(&data) {
                                if let Some(id) = json.get("id") {
                                    let id_key = match id {
                                        Value::Number(n) => n.to_string(),
                                        Value::String(s) => s.clone(),
                                        other => other.to_string(),
                                    };
                                    if json.get("result").is_some() || json.get("error").is_some() {
                                        if let Some(tx) =
                                            pending_for_reader.lock().await.remove(&id_key)
                                        {
                                            let _ = tx.send(json);
                                        }
                                    }
                                }
                            }
                        }
                        event.clear();
                        data.clear();
                    }
                }
            }
            drop(hub);
        });

        let post_url = tokio::time::timeout(Duration::from_secs(15), endpoint_rx)
            .await
            .map_err(|_| "等待 SSE endpoint 超时".to_string())?
            .map_err(|_| "SSE endpoint 通道关闭".to_string())?;

        let tools = self
            .fetch_tools_rpc(&post_url, "sse", 1, &server.headers)
            .await?;
        let count = self.store_tools(server, &post_url, "sse", tools);
        tracing::info!("全局 MCP {} (SSE) 已同步 {} 个工具", server.name, count);

        let _ = read_task.await;
        Ok(())
    }

    fn store_tools(
        &self,
        server: &McpServerEntry,
        call_url: &str,
        transport: &str,
        tools: Vec<McpTool>,
    ) -> usize {
        let prefix = format!("{}_", server.name);
        self.bindings
            .retain(|_, v| v.server_name != server.name);
        for tool in tools {
            let key = if self.bindings.contains_key(&tool.name) {
                format!("{prefix}{}", tool.name)
            } else {
                tool.name.clone()
            };
            self.bindings.insert(
                key,
                GlobalToolBinding {
                    server_name: server.name.clone(),
                    tool,
                    call_url: call_url.to_string(),
                    transport: transport.to_string(),
                    headers: server.headers.clone(),
                },
            );
        }
        self.bindings
            .iter()
            .filter(|e| e.value().server_name == server.name)
            .count()
    }

    async fn fetch_tools_rpc(
        &self,
        url: &str,
        transport: &str,
        id: u64,
        headers: &HashMap<String, String>,
    ) -> Result<Vec<McpTool>, String> {
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/list",
            "params": {}
        });
        let resp = self
            .rpc_call(url, transport, id, payload, headers, None)
            .await?;
        Ok(parse_tools_list(resp.get("result")))
    }

    async fn rpc_call(
        &self,
        url: &str,
        transport: &str,
        id: u64,
        payload: Value,
        headers: &HashMap<String, String>,
        mcp_session_id: Option<&str>,
    ) -> Result<Value, String> {
        if transport == "sse" {
            self.post_sse_rpc(url, id, payload, headers).await
        } else {
            self.post_json_rpc(url, payload, headers, mcp_session_id).await
        }
    }

    async fn post_json_rpc(
        &self,
        url: &str,
        payload: Value,
        headers: &HashMap<String, String>,
        mcp_session_id: Option<&str>,
    ) -> Result<Value, String> {
        let mut req = apply_request_headers(
            self.client
                .post(url)
                .header("Accept", "application/json, text/event-stream")
                .header("Content-Type", "application/json")
                .json(&payload),
            headers,
        );
        if let Some(sid) = mcp_session_id.filter(|s| !s.is_empty()) {
            req = req.header("mcp-session-id", sid);
        }
        let resp = req.send().await.map_err(|e| e.to_string())?;
        let (body, _) = crate::streamable_http::parse_http_response(resp).await?;
        Ok(body)
    }

    async fn post_sse_rpc(
        &self,
        post_url: &str,
        id: u64,
        payload: Value,
        headers: &HashMap<String, String>,
    ) -> Result<Value, String> {
        let id_key = id.to_string();
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id_key.clone(), tx);

        let req = apply_request_headers(self.client.post(post_url).json(&payload), headers);
        let resp = req.send().await.map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            self.pending.lock().await.remove(&id_key);
            return Err(format!("SSE POST 失败: {}", resp.status()));
        }

        if let Ok(v) = resp.json::<Value>().await {
            if v.get("result").is_some() || v.get("error").is_some() {
                self.pending.lock().await.remove(&id_key);
                return Ok(v);
            }
        }

        match tokio::time::timeout(Duration::from_secs(15), rx).await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(_)) => Err("SSE 响应通道关闭".into()),
            Err(_) => {
                self.pending.lock().await.remove(&id_key);
                Err("SSE 请求超时".into())
            }
        }
    }
}

fn apply_request_headers(
    req: reqwest::RequestBuilder,
    headers: &HashMap<String, String>,
) -> reqwest::RequestBuilder {
    let mut builder = req;
    for (k, v) in headers {
        builder = builder.header(k, v);
    }
    builder
}

pub(crate) fn base_url(url: &str) -> String {
    if let Some(idx) = url.find("://") {
        if let Some(path) = url[idx + 3..].find('/') {
            return url[..idx + 3 + path].to_string();
        }
    }
    url.trim_end_matches('/').to_string()
}

pub(crate) fn normalize_transport(kind: &str, url: &str) -> String {
    match kind.to_lowercase().as_str() {
        "sse" => "sse".to_string(),
        "streamablehttp" | "streamable_http" | "http" => "http".to_string(),
        _ if url.contains("/sse") => "sse".to_string(),
        _ if url.ends_with("/mcp") || url.contains("/mcp?") => "http".to_string(),
        _ => "http".to_string(),
    }
}

pub(crate) fn parse_tools_list(result: Option<&Value>) -> Vec<McpTool> {
    let Some(result) = result else {
        return Vec::new();
    };
    result
        .get("tools")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|t| {
            Some(McpTool {
                name: t.get("name")?.as_str()?.to_string(),
                description: t
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                input_schema: t
                    .get("inputSchema")
                    .or_else(|| t.get("input_schema"))
                    .cloned()
                    .unwrap_or(json!({})),
            })
        })
        .collect()
}

fn format_tool_result(result: Option<&Value>) -> String {
    let Some(result) = result else {
        return "ok".to_string();
    };
    if let Some(items) = result.get("content").and_then(|v| v.as_array()) {
        let texts: Vec<String> = items
            .iter()
            .filter_map(|item| {
                if item.get("type")?.as_str()? == "text" {
                    item.get("text")?.as_str().map(String::from)
                } else {
                    None
                }
            })
            .collect();
        if !texts.is_empty() {
            return texts.join("\n");
        }
    }
    result.to_string()
}
