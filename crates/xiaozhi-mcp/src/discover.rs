//! 一次性探测远程 MCP 端点工具列表（Manager `discover-tools`）

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use tokio::sync::{oneshot, Mutex};

use crate::global_hub::{base_url, normalize_transport, parse_tools_list};
use crate::streamable_http::StreamableHttpClient;
use crate::types::McpTool;

struct DiscoverClient {
    client: Client,
    headers: HashMap<String, String>,
    cookies: HashMap<String, String>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    next_id: AtomicU64,
}

impl DiscoverClient {
    fn new(headers: HashMap<String, String>, cookies: HashMap<String, String>) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(20))
                .no_proxy()
                .user_agent(concat!("xiaozhi-manager/", env!("CARGO_PKG_VERSION")))
                .build()
                .unwrap_or_else(|_| Client::new()),
            headers,
            cookies,
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_id: AtomicU64::new(1),
        }
    }

    fn apply_headers(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let mut builder = req;
        for (k, v) in &self.headers {
            builder = builder.header(k.as_str(), v.as_str());
        }
        if !self.cookies.is_empty() {
            let cookie = self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ");
            builder = builder.header("Cookie", cookie);
        }
        builder
    }

    async fn discover(&self, transport: &str, url: &str) -> Result<Vec<McpTool>, String> {
        let transport = normalize_transport(transport, url);
        if transport == "sse" {
            let (post_url, _sse_reader) = self.open_sse_session(url).await?;
            self.initialize_if_needed(&post_url, "sse").await?;
            return self.fetch_tools(&post_url, "sse").await;
        }

        let mut client = StreamableHttpClient::new(
            self.client.clone(),
            url,
            self.headers.clone(),
            self.cookies.clone(),
        );
        client.open_session().await?;
        let result = client.call("tools/list", json!({})).await?;
        let mut tools = parse_tools_list(Some(&result));
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(tools)
    }

    async fn initialize_if_needed(&self, url: &str, transport: &str) -> Result<(), String> {
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
        let resp = self.rpc_call(url, transport, id, payload).await?;
        if resp.get("error").is_some() {
            tracing::debug!("MCP initialize 返回 error，继续尝试 tools/list: {resp}");
        }
        Ok(())
    }

    async fn fetch_tools(&self, url: &str, transport: &str) -> Result<Vec<McpTool>, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/list",
            "params": {}
        });
        let resp = self.rpc_call(url, transport, id, payload).await?;
        if let Some(err) = resp.get("error") {
            return Err(format!("获取工具列表失败: {err}"));
        }
        let mut tools = parse_tools_list(resp.get("result"));
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(tools)
    }

    async fn open_sse_session(
        &self,
        url: &str,
    ) -> Result<(String, tokio::task::JoinHandle<()>), String> {
        let resp = self
            .apply_headers(
                self.client
                    .get(url)
                    .header("Accept", "text/event-stream"),
            )
            .send()
            .await
            .map_err(|e| format!("SSE 连接失败: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!("SSE 连接失败: HTTP {}", resp.status()));
        }

        let base = base_url(url);
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
        });

        let post_url = tokio::time::timeout(Duration::from_secs(15), endpoint_rx)
            .await
            .map_err(|_| "等待 SSE endpoint 超时".to_string())?
            .map_err(|_| "SSE endpoint 通道关闭".to_string())?;

        Ok((post_url, read_task))
    }

    async fn rpc_call(
        &self,
        url: &str,
        transport: &str,
        id: u64,
        payload: Value,
    ) -> Result<Value, String> {
        if transport == "sse" {
            self.post_sse_rpc(url, id, payload).await
        } else {
            self.post_json_rpc(url, payload).await
        }
    }

    async fn post_json_rpc(&self, url: &str, payload: Value) -> Result<Value, String> {
        let resp = self
            .apply_headers(
                self.client
                    .post(url)
                    .header("Accept", "application/json, text/event-stream")
                    .json(&payload),
            )
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| e.to_string())?;
        if !status.is_success() {
            let detail = text.trim();
            if detail.is_empty() {
                return Err(format!("HTTP {status} {url}"));
            }
            return Err(format!("HTTP {status} {url}: {detail}"));
        }
        if text.trim().is_empty() {
            return Ok(json!({}));
        }
        serde_json::from_str(&text).map_err(|e| format!("解析响应失败: {e}; body={text}"))
    }

    async fn post_sse_rpc(&self, post_url: &str, id: u64, payload: Value) -> Result<Value, String> {
        let id_key = id.to_string();
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id_key.clone(), tx);

        let resp = self
            .apply_headers(self.client.post(post_url).json(&payload))
            .send()
            .await
            .map_err(|e| e.to_string())?;

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

/// 探测 MCP 端点可用工具（对齐 Go `listImportedServiceTools`）
pub async fn discover_mcp_tools(
    transport: &str,
    url: &str,
    headers: &HashMap<String, String>,
    cookies: &HashMap<String, String>,
) -> Result<Vec<McpTool>, String> {
    let transport = transport.trim();
    let url = url.trim();
    if url.is_empty() {
        return Err("url 不能为空".into());
    }
    let normalized = normalize_transport(transport, url);
    if normalized != "sse" && normalized != "http" {
        return Err("transport 仅支持 sse/streamablehttp".into());
    }
    DiscoverClient::new(headers.clone(), cookies.clone())
        .discover(transport, url)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_url() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt
            .block_on(discover_mcp_tools("http", "", &HashMap::new(), &HashMap::new()))
            .unwrap_err();
        assert!(err.contains("url"));
    }
}
