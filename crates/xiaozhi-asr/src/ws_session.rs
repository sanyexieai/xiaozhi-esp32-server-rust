//! WebSocket 连接复用 + 任务串行（对齐 Go `connMu`/`taskMu` 或 FunASR `sendMutex`）

use futures_util::{SinkExt, StreamExt};
use futures_util::stream::{SplitSink, SplitStream};
use std::future::Future;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::{Mutex, OwnedMutexGuard};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use xiaozhi_core::Result;

pub type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;
pub type WsWrite = SplitSink<WsStream, Message>;
pub type WsRead = SplitStream<WsStream>;

pub struct WsConnParts {
    pub write: WsWrite,
    pub read: WsRead,
}

impl WsConnParts {
    pub fn split(stream: WsStream) -> Self {
        let (write, read) = stream.split();
        Self { write, read }
    }
}

/// 单次流式任务结束时：复用或作废底层 WebSocket
pub enum TaskSessionOutcome {
    Reuse(WsConnParts),
    Invalidate,
}

/// 可复用 WebSocket 会话池（每个 ASR Provider 实例持有一份）
#[derive(Clone, Default)]
pub struct ReusableWsSession {
    conn_mu: Arc<Mutex<Option<WsConnParts>>>,
    task_mu: Arc<Mutex<()>>,
}

impl ReusableWsSession {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn conn_mu(&self) -> Arc<Mutex<Option<WsConnParts>>> {
        Arc::clone(&self.conn_mu)
    }

    /// 对齐 Go `taskMu.Lock` / FunASR `sendMutex.Lock`
    pub async fn acquire_task(&self) -> OwnedMutexGuard<()> {
        self.task_mu.clone().lock_owned().await
    }

    pub async fn close(&self) {
        Self::invalidate_conn(&self.conn_mu).await;
    }

    /// 对齐 Go `getConn`：取出缓存连接，否则 `connect` 新建
    pub async fn take_or_connect<F, Fut>(
        conn_mu: &Mutex<Option<WsConnParts>>,
        connect: F,
    ) -> Result<WsConnParts>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<WsStream>>,
    {
        let mut guard = conn_mu.lock().await;
        if let Some(parts) = guard.take() {
            return Ok(parts);
        }
        drop(guard);
        let stream = connect().await?;
        Ok(WsConnParts::split(stream))
    }

    pub async fn restore_conn(conn_mu: &Mutex<Option<WsConnParts>>, parts: WsConnParts) {
        let mut guard = conn_mu.lock().await;
        *guard = Some(parts);
    }

    /// 对齐 Go `invalidateConn` / FunASR `clearConnection`
    pub async fn invalidate_conn(conn_mu: &Mutex<Option<WsConnParts>>) {
        let mut guard = conn_mu.lock().await;
        if let Some(mut parts) = guard.take() {
            let _ = parts.write.close().await;
        }
    }

    /// 任务结束：恢复或作废连接，并释放 task 锁
    pub async fn finish_task(
        conn_mu: &Mutex<Option<WsConnParts>>,
        _task_guard: OwnedMutexGuard<()>,
        outcome: TaskSessionOutcome,
    ) {
        match outcome {
            TaskSessionOutcome::Reuse(parts) => Self::restore_conn(conn_mu, parts).await,
            TaskSessionOutcome::Invalidate => Self::invalidate_conn(conn_mu).await,
        }
    }
}
