use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::{Mutex, RwLock};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        client::IntoClientRequest,
        http::HeaderValue,
        Message,
    },
    MaybeTlsStream, WebSocketStream,
};
use tracing::{error, info, warn};
use uuid::Uuid;
use xiaozhi_auth::create_manager_ws_token;
use xiaozhi_config::AppConfig;

use crate::bridge::{BridgeDispatcher, WsRequest, WsResponse};

type WsWrite = futures_util::stream::SplitSink<
    WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;

type PendingMap = Arc<RwLock<HashMap<String, tokio::sync::oneshot::Sender<WsResponse>>>>;

pub struct ManagerWsClient {
    ws_url: String,
    endpoint_auth_token: String,
    client_uuid: String,
    dispatcher: Arc<BridgeDispatcher>,
    write: Arc<Mutex<Option<WsWrite>>>,
    pending: PendingMap,
    connected: Arc<AtomicBool>,
}

impl ManagerWsClient {
    pub fn new(config: &AppConfig, dispatcher: Arc<BridgeDispatcher>) -> Arc<Self> {
        Arc::new(Self {
            ws_url: backend_to_ws_url(&config.manager.backend_url),
            endpoint_auth_token: config.manager.endpoint_auth_token.clone(),
            client_uuid: Uuid::new_v4().to_string(),
            dispatcher,
            write: Arc::new(Mutex::new(None)),
            pending: Arc::new(RwLock::new(HashMap::new())),
            connected: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    pub async fn send_request(
        &self,
        method: &str,
        path: &str,
        body: serde_json::Value,
        timeout: Duration,
    ) -> anyhow::Result<WsResponse> {
        if !self.is_connected() {
            anyhow::bail!("Manager WS 未连接");
        }

        let id = Uuid::new_v4().to_string();
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.pending.write().await.insert(id.clone(), tx);

        let req = WsRequest {
            id: id.clone(),
            method: method.to_string(),
            path: path.to_string(),
            headers: HashMap::new(),
            body,
        };
        let payload = serde_json::to_string(&req)?;
        if let Err(e) = self.send_text(payload).await {
            self.pending.write().await.remove(&id);
            return Err(e);
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(resp)) => Ok(resp),
            Ok(Err(_)) => {
                self.pending.write().await.remove(&id);
                anyhow::bail!("WS 响应通道已关闭")
            }
            Err(_) => {
                self.pending.write().await.remove(&id);
                anyhow::bail!("WS 请求超时")
            }
        }
    }

    async fn send_text(&self, text: String) -> anyhow::Result<()> {
        let mut guard = self.write.lock().await;
        let write = guard
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Manager WS 未连接"))?;
        write
            .send(Message::Text(text.into()))
            .await
            .map_err(|e| anyhow::anyhow!("Manager WS 发送失败: {e}"))
    }

    pub async fn run_forever(self: Arc<Self>) {
        let mut backoff = Duration::from_secs(1);
        loop {
            match self.connect_once().await {
                Ok(()) => {
                    backoff = Duration::from_secs(1);
                    warn!("Manager WS 连接已断开，准备重连...");
                }
                Err(e) => {
                    error!("Manager WS 连接失败: {e}");
                }
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(30));
        }
    }

    async fn connect_once(self: &Arc<Self>) -> anyhow::Result<()> {
        let token = create_manager_ws_token(&self.endpoint_auth_token, &self.client_uuid, 3600)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;

        let mut request = self
            .ws_url
            .as_str()
            .into_client_request()
            .map_err(|e| anyhow::anyhow!("WS 请求构建失败: {e}"))?;
        request.headers_mut().insert(
            "Authorization",
            HeaderValue::from_str(&format!("Bearer {token}"))?,
        );
        request
            .headers_mut()
            .insert("UUID", HeaderValue::from_str(&self.client_uuid)?);

        info!("正在连接 Manager WS: {}", self.ws_url);
        let (ws, _) = connect_async(request).await?;
        info!("Manager WS 已连接 (uuid={})", self.client_uuid);

        let (write, mut read) = ws.split();
        *self.write.lock().await = Some(write);
        self.connected.store(true, Ordering::SeqCst);

        // 启动时若 manager 尚未就绪会沿用 config.yaml；重连后主动拉取 DB 配置
        crate::shared_config::load_from_manager(
            &self.dispatcher.config,
            self.dispatcher.config_provider.as_ref(),
        )
        .await;

        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                        if v.get("type").and_then(|t| t.as_str()) == Some("system_config") {
                            if let Some(data) = v.get("data") {
                                self.dispatcher.apply_system_config(data.clone()).await;
                            }
                            continue;
                        }
                        if v.get("method").is_some() {
                            if let Ok(req) = serde_json::from_value::<WsRequest>(v) {
                                if req.method == "POST" && req.path == "/api/openclaw/chat" {
                                    let openclaw = self.dispatcher.openclaw.clone();
                                    let client = self.clone();
                                    tokio::spawn(async move {
                                        crate::openclaw_chat::handle_openclaw_chat(
                                            &openclaw,
                                            req,
                                            |resp| {
                                                let client = client.clone();
                                                async move {
                                                    if let Ok(payload) =
                                                        serde_json::to_string(&resp)
                                                    {
                                                        let _ = client.send_text(payload).await;
                                                    }
                                                }
                                            },
                                        )
                                        .await;
                                    });
                                    continue;
                                }
                                let resp = self.dispatcher.handle(req).await;
                                if let Ok(payload) = serde_json::to_string(&resp) {
                                    if self.send_text(payload).await.is_err() {
                                        break;
                                    }
                                }
                            }
                            continue;
                        }
                        if v.get("status").is_some() {
                            if let Ok(resp) = serde_json::from_value::<WsResponse>(v) {
                                if let Some(tx) = self.pending.write().await.remove(&resp.id) {
                                    let _ = tx.send(resp);
                                }
                            }
                        }
                    }
                }
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }

        *self.write.lock().await = None;
        self.connected.store(false, Ordering::SeqCst);
        Ok(())
    }
}

fn backend_to_ws_url(backend_url: &str) -> String {
    let url = backend_url.trim_end_matches('/');
    if let Some(rest) = url.strip_prefix("https://") {
        format!("wss://{rest}/ws")
    } else if let Some(rest) = url.strip_prefix("http://") {
        format!("ws://{rest}/ws")
    } else {
        format!("ws://{url}/ws")
    }
}
