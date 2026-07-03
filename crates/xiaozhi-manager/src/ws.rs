use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot, RwLock};
use uuid::Uuid;

use crate::db::Database;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsRequest {
    pub id: String,
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub body: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsResponse {
    pub id: String,
    pub status: i32,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub body: serde_json::Value,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
}

impl WsResponse {
    pub fn ok(id: String, body: serde_json::Value) -> Self {
        Self {
            id,
            status: 200,
            headers: HashMap::new(),
            body,
            error: String::new(),
        }
    }

    pub fn err(id: String, status: i32, message: impl Into<String>) -> Self {
        Self {
            id,
            status,
            headers: HashMap::new(),
            body: serde_json::Value::Null,
            error: message.into(),
        }
    }
}

#[derive(Debug, Clone)]
enum HubOutbound {
    Request(WsRequest),
    Push(serde_json::Value),
    Response(WsResponse),
}

type PendingMap = Arc<RwLock<HashMap<String, oneshot::Sender<WsResponse>>>>;
type StreamPendingMap =
    Arc<RwLock<HashMap<String, mpsc::UnboundedSender<WsResponse>>>>;

#[derive(Clone)]
pub struct WsHub {
    clients: Arc<RwLock<HashMap<String, mpsc::UnboundedSender<HubOutbound>>>>,
    pending: PendingMap,
    stream_pending: StreamPendingMap,
    db: Arc<Database>,
}

impl WsHub {
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            clients: Arc::new(RwLock::new(HashMap::new())),
            pending: Arc::new(RwLock::new(HashMap::new())),
            stream_pending: Arc::new(RwLock::new(HashMap::new())),
            db,
        }
    }

    pub async fn client_count(&self) -> usize {
        self.clients.read().await.len()
    }

    fn handle_server_request(db: &Database, req: WsRequest) -> WsResponse {
        let device_id = req
            .body
            .get("device_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();

        match (req.method.as_str(), req.path.as_str()) {
            ("POST", "/api/device/active") => {
                if device_id.is_empty() {
                    return WsResponse::err(req.id, 400, "缺少device_id参数");
                }
                tracing::info!("处理设备上线请求，device_id: {device_id}");
                match db.set_device_presence(device_id, true) {
                    Ok(true) => {
                        let now = chrono::Utc::now().to_rfc3339();
                        WsResponse::ok(
                            req.id,
                            serde_json::json!({
                                "device_id": device_id,
                                "online": true,
                                "last_active_at": now,
                                "message": "设备上线状态更新成功",
                            }),
                        )
                    }
                    Ok(false) => WsResponse::err(req.id, 404, "设备不存在"),
                    Err(e) => WsResponse::err(req.id, 500, format!("更新设备上线状态失败: {e}")),
                }
            }
            ("POST", "/api/device/inactive") => {
                if device_id.is_empty() {
                    return WsResponse::err(req.id, 400, "缺少device_id参数");
                }
                tracing::info!("处理设备离线请求，device_id: {device_id}");
                match db.set_device_inactive(device_id) {
                    Ok(true) => WsResponse::ok(
                        req.id,
                        serde_json::json!({
                            "device_id": device_id,
                            "online": false,
                            "last_active_at": serde_json::Value::Null,
                            "message": "设备离线状态更新成功",
                        }),
                    ),
                    Ok(false) => WsResponse::err(req.id, 404, "设备不存在"),
                    Err(e) => WsResponse::err(req.id, 500, format!("更新设备离线状态失败: {e}")),
                }
            }
            _ => WsResponse::err(req.id, 404, "Unknown endpoint"),
        }
    }

    pub async fn handle_socket(&self, uuid: String, socket: WebSocket) {
        let (mut sender, mut receiver) = socket.split();
        let (tx, mut rx) = mpsc::unbounded_channel::<HubOutbound>();
        self.clients.write().await.insert(uuid.clone(), tx.clone());
        tracing::info!("WS 客户端已连接: {uuid}");

        // server 若在 manager 就绪前启动会回退 config.yaml；连接后立即推送 DB 配置
        if let Ok(bundle) = crate::system_configs::build_system_configs_data(&self.db) {
            let payload = serde_json::json!({
                "type": "system_config",
                "data": bundle,
            });
            let _ = tx.send(HubOutbound::Push(payload));
        }

        let pending = self.pending.clone();
        let stream_pending = self.stream_pending.clone();
        let clients = self.clients.clone();
        let uuid_for_cleanup = uuid.clone();
        let db = self.db.clone();

        let write_task = tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                let text = match msg {
                    HubOutbound::Request(req) => serde_json::to_string(&req).ok(),
                    HubOutbound::Response(resp) => serde_json::to_string(&resp).ok(),
                    HubOutbound::Push(v) => serde_json::to_string(&v).ok(),
                };
                if let Some(text) = text {
                    if sender.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
            }
        });

        while let Some(Ok(msg)) = receiver.next().await {
            match msg {
                Message::Text(text) => {
                    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
                        continue;
                    };
                    if v.get("method").is_some() {
                        if let Ok(req) = serde_json::from_value::<WsRequest>(v) {
                            let resp = Self::handle_server_request(&db, req);
                            let _ = tx.send(HubOutbound::Response(resp));
                        }
                        continue;
                    }
                    if v.get("status").is_some() {
                        if let Ok(resp) = serde_json::from_value::<WsResponse>(v) {
                            if let Some(tx) = stream_pending.read().await.get(&resp.id).cloned()
                            {
                                let _ = tx.send(resp.clone());
                                if resp.status != 206 {
                                    stream_pending.write().await.remove(&resp.id);
                                }
                                continue;
                            }
                            if let Some(done) = pending.write().await.remove(&resp.id) {
                                let _ = done.send(resp);
                            }
                        }
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }

        write_task.abort();
        clients.write().await.remove(&uuid_for_cleanup);
        tracing::info!("WS 客户端已断开: {uuid_for_cleanup}");
    }

    pub async fn broadcast_stream_request(
        &self,
        method: &str,
        path: &str,
        mut body: serde_json::Value,
        timeout: Duration,
    ) -> Result<(mpsc::UnboundedReceiver<WsResponse>, String), String> {
        let clients = self.clients.read().await;
        if clients.is_empty() {
            return Err("没有已连接的主服务客户端，请确认 xiaozhi-server 已启动".into());
        }
        if let Some(obj) = body.as_object_mut() {
            obj.insert("stream_events".to_string(), serde_json::json!(true));
        }
        let id = Uuid::new_v4().to_string();
        let (tx, rx) = mpsc::unbounded_channel();
        self.stream_pending.write().await.insert(id.clone(), tx);
        let req = WsRequest {
            id: id.clone(),
            method: method.to_string(),
            path: path.to_string(),
            headers: HashMap::new(),
            body,
        };
        for client_tx in clients.values() {
            let _ = client_tx.send(HubOutbound::Request(req.clone()));
        }
        drop(clients);

        let stream_pending = self.stream_pending.clone();
        let request_id = id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            stream_pending.write().await.remove(&request_id);
        });

        Ok((rx, id))
    }

    pub async fn broadcast_request(
        &self,
        method: &str,
        path: &str,
        body: serde_json::Value,
        timeout: Duration,
    ) -> Result<WsResponse, String> {
        let clients = self.clients.read().await;
        if clients.is_empty() {
            return Err("没有已连接的主服务客户端，请确认 xiaozhi-server 已启动".into());
        }
        let id = Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        self.pending.write().await.insert(id.clone(), tx);
        let req = WsRequest {
            id: id.clone(),
            method: method.to_string(),
            path: path.to_string(),
            headers: HashMap::new(),
            body,
        };
        for client_tx in clients.values() {
            let _ = client_tx.send(HubOutbound::Request(req.clone()));
        }
        drop(clients);

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(resp)) => Ok(resp),
            Ok(Err(_)) => Err("WS 响应通道已关闭".into()),
            Err(_) => {
                self.pending.write().await.remove(&id);
                Err("WS 请求超时".into())
            }
        }
    }

    pub async fn broadcast_system_config(&self, data: serde_json::Value) {
        let payload = serde_json::json!({
            "type": "system_config",
            "data": data,
        });
        let clients = self.clients.read().await;
        for client_tx in clients.values() {
            let _ = client_tx.send(HubOutbound::Push(payload.clone()));
        }
    }

    pub async fn endpoint_status(&self, agent_id: &str) -> serde_json::Value {
        let count = self.client_count().await;
        let connected = count > 0;
        serde_json::json!({
            "endpoint": format!("/ws/agent/{agent_id}"),
            "status": if connected { "connected" } else { "disconnected" },
            "connected": connected,
            "client_count": count,
            "status_message": if connected { "主服务已连接" } else { "等待主服务连接" },
        })
    }
}
