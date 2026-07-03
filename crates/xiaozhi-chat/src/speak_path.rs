//! MQTT 主动播报链路：`speak_request` ↔ `speak_ready`（对齐 Go `chat.go`）

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::Mutex;
use xiaozhi_core::Result;
use xiaozhi_protocol::messages::ServerMessage;

use crate::manager::ChatManager;
use crate::outbound::SpeakDelivery;
use crate::state::ListenPhase;
use crate::tts_manager::TtsPlaybackStartHook;
use crate::tts_turn_policy::TtsTurnEndPolicy;

pub(crate) struct PendingSpeakRequest {
    session_id: String,
    notify: tokio::sync::Notify,
    error: Mutex<Option<String>>,
}

impl ChatManager {
    pub fn set_mqtt_transport(&self, is_mqtt: bool) {
        self.is_mqtt_transport
            .store(is_mqtt, Ordering::SeqCst);
    }

    pub fn is_mqtt_transport(&self) -> bool {
        self.is_mqtt_transport.load(Ordering::SeqCst)
    }

    pub fn set_udp_binding_active(&self, active: bool) {
        self.has_udp_binding.store(active, Ordering::SeqCst);
        if !active {
            self.udp_last_active_ms.store(0, Ordering::SeqCst);
        }
    }

    pub fn touch_udp_transport_active(&self) {
        if !self.is_mqtt_transport() {
            return;
        }
        self.has_udp_binding.store(true, Ordering::SeqCst);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        self.udp_last_active_ms.store(now, Ordering::SeqCst);
    }

    /// 对齐 Go `refreshSpeakPathWarmFromTransport`
    pub async fn refresh_speak_path_warm_from_transport(&self) {
        if !self.is_mqtt_transport() || !self.has_udp_binding.load(Ordering::SeqCst) {
            return;
        }
        if self.has_pending_speak_request().await {
            tracing::debug!(
                device_id = %self.device_id(),
                "存在待完成 speak_request，跳过刷新热链路"
            );
            return;
        }
        if self.mqtt_rebootstrap_pending.load(Ordering::SeqCst) {
            tracing::debug!(
                device_id = %self.device_id(),
                "MQTT 会话重建标记存在，跳过刷新热链路"
            );
            return;
        }
        let transport_ts = self.udp_last_active_ms.load(Ordering::SeqCst);
        if transport_ts > 0 {
            self.last_speak_path_warm_at_ms
                .store(transport_ts, Ordering::SeqCst);
            return;
        }
        self.mark_speak_path_warm();
    }

    pub async fn reset_speak_path_after_server_session_close(&self, reason: &str) {
        self.reset_speak_path_after_session_reset(&format!(
            "服务端关闭会话导致主动播报链路已重置: {reason}"
        ))
        .await;
    }

    pub async fn refresh_device_config_on_hello(self: &Arc<Self>) -> Result<()> {
        self.reload_device_config().await
    }

    /// 主动播报 / 对话输出进行中：跳过 MQTT transport 重置与 goodbye 重握手，避免打断 TTS。
    pub async fn should_protect_active_speak_flow(&self) -> bool {
        if !self.is_mqtt_transport() {
            return false;
        }
        if self.is_injected_speech_guard_active() {
            return true;
        }
        if self.has_pending_speak_request().await {
            return true;
        }
        self.is_conversation_active().await
    }

    pub fn is_injected_speech_guard_active(&self) -> bool {
        self.injected_speech_guard.load(Ordering::SeqCst)
    }

    pub fn begin_injected_speech_guard(&self) {
        self.injected_speech_guard.store(true, Ordering::SeqCst);
    }

    pub fn clear_injected_speech_guard(&self) {
        self.injected_speech_guard.store(false, Ordering::SeqCst);
    }

    /// TTS 播放中收到 duplicate hello 时保留现有 UDP 会话，避免音频断流
    pub async fn should_preserve_udp_session_on_hello(&self) -> bool {
        if !self.is_mqtt_transport() || !self.has_udp_binding.load(Ordering::SeqCst) {
            return false;
        }
        self.is_conversation_active().await
    }

    /// 设备 hello 已建立/刷新 UDP 后，清除 MQTT 重建标记以恢复热链路
    pub fn on_hardware_hello_received(&self) {
        self.clear_mqtt_rebootstrap_pending("hello");
    }

    /// 对齐 Go `markMqttConversationStateStale`
    pub async fn mark_mqtt_conversation_state_stale(self: &Arc<Self>, reason: &str) {
        if !self.is_mqtt_transport() {
            return;
        }
        if self.should_protect_active_speak_flow().await {
            tracing::info!(
                device_id = %self.device_id(),
                reason,
                "活跃播报流程中，跳过 MQTT 会话重建归一化"
            );
            return;
        }

        let already = self
            .mqtt_rebootstrap_pending
            .swap(true, Ordering::SeqCst);
        if already {
            tracing::debug!(
                device_id = %self.device_id(),
                reason,
                "MQTT 会话重建标记已存在，跳过重复归一化"
            );
            return;
        }

        {
            let mut guard = self.session.lock().await;
            if let Some(session) = guard.as_mut() {
                tracing::info!(
                    device_id = %self.device_id(),
                    reason,
                    "MQTT 链路重建，重置当前会话状态"
                );
                session.reset_to_silent_state(self.as_ref()).await;
            }
        }
        self.reset_speak_path_after_session_reset(&format!("MQTT 链路重建: {reason}"))
            .await;
        Arc::clone(self)
            .schedule_retained_session_cleanup(&format!("mqtt_{reason}"))
            .await;
    }

    pub async fn handle_mqtt_transport_ready(self: &Arc<Self>) {
        if self.should_protect_active_speak_flow().await {
            tracing::info!(
                device_id = %self.device_id(),
                "MQTT transport ready 期间存在活跃播报，跳过重置以免打断播放"
            );
            return;
        }
        self.mark_mqtt_conversation_state_stale("transport_ready")
            .await;
        self.device_mcp.lock().await.reset_on_transport_ready();
        self.warmup_mcp();
        tracing::info!(
            device_id = %self.device_id(),
            "MQTT transport ready，已重置会话态与 MCP runtime"
        );
    }

    pub async fn handle_speak_ready_message(
        self: &Arc<Self>,
        session_id: Option<&str>,
        state: Option<&str>,
        udp_config: Option<&xiaozhi_protocol::messages::SpeakReadyUdpConfig>,
    ) -> Result<()> {
        if !self.is_mqtt_transport() {
            return Ok(());
        }
        if let Some(cfg) = udp_config {
            if !cfg.ready {
                tracing::warn!(
                    device_id = %self.device_id(),
                    "speak_ready udp_config.ready=false，忽略"
                );
                return Ok(());
            }
        }
        if let Some(s) = state {
            if s != xiaozhi_core::message::READY {
                tracing::debug!(
                    device_id = %self.device_id(),
                    state = %s,
                    "speak_ready 状态不是 ready，忽略"
                );
                return Ok(());
            }
        }

        let pending = {
            let guard = self.pending_speak_request.lock().await;
            guard.clone()
        };
        let Some(pending) = pending else {
            tracing::debug!(
                device_id = %self.device_id(),
                "收到无待处理请求的 speak_ready，忽略"
            );
            return Ok(());
        };

        if !pending.session_id.is_empty() {
            let got = session_id.unwrap_or("").trim();
            if !got.is_empty() && got != pending.session_id {
                tracing::warn!(
                    device_id = %self.device_id(),
                    got,
                    want = %pending.session_id,
                    "speak_ready session_id 不匹配"
                );
                return Ok(());
            }
        }

        self.mark_speak_path_warm();
        self.clear_mqtt_rebootstrap_pending("speak_ready");
        self.finish_pending_speak_request(&pending, None).await;
        let reuse_existing = udp_config.map(|c| c.reuse_existing).unwrap_or(false);
        tracing::info!(
            device_id = %self.device_id(),
            reuse_existing,
            "speak_ready 已就绪"
        );
        Ok(())
    }

    pub async fn prepare_speak_path_for_injected_speech(
        self: &Arc<Self>,
        preview_text: &str,
        auto_listen: bool,
    ) -> Result<()> {
        if !self.is_mqtt_transport() {
            return Ok(());
        }
        self.prepare_speak_path_for_injected_speech_once(preview_text, auto_listen)
            .await
    }

    async fn prepare_speak_path_for_injected_speech_once(
        self: &Arc<Self>,
        preview_text: &str,
        auto_listen: bool,
    ) -> Result<()> {
        if !self.is_mqtt_transport() {
            return Ok(());
        }
        if !self.should_send_speak_request().await {
            tracing::debug!(
                device_id = %self.device_id(),
                "注入消息复用现有播报链路，跳过 speak_request"
            );
            return Ok(());
        }

        let session_id = self.ensure_client_session_id().await?;
        let (pending, created) = self.get_or_create_pending_speak_request(session_id).await;
        if created {
            let msg = ServerMessage::speak_request(preview_text, &pending.session_id, auto_listen);
            let sent = self.push_hardware_command(&msg);
            if !sent {
                self.finish_pending_speak_request(
                    &pending,
                    Some("speak_request 下发失败（无硬件 MQTT 通道）".into()),
                )
                .await;
                return Err(xiaozhi_core::Error::Session(
                    "speak_request 下发失败（无硬件 MQTT 通道，请确认设备 MQTT 在线）".into(),
                ));
            }
            tracing::info!(
                device_id = %self.device_id(),
                session_id = %pending.session_id,
                "已发送 speak_request"
            );
        }

        let timeout = Duration::from_millis(self.app_config.chat.speak_ready_timeout_ms.max(1000));
        tokio::select! {
            _ = pending.notify.notified() => {
                if let Some(err) = pending.error.lock().await.clone() {
                    return Err(xiaozhi_core::Error::Session(err));
                }
            }
            _ = tokio::time::sleep(timeout) => {
                self.finish_pending_speak_request(
                    &pending,
                    Some("等待 speak_ready 超时".into()),
                )
                .await;
                return Err(xiaozhi_core::Error::Session("等待 speak_ready 超时".into()));
            }
        }

        if self.requires_hello_bootstrap_for_session().await {
            self.wait_for_injected_speech_session().await?;
        }
        Ok(())
    }

    pub async fn has_pending_speak_request(&self) -> bool {
        self.pending_speak_request.lock().await.is_some()
    }

    pub async fn reset_speak_path_after_goodbye(&self) {
        self.reset_speak_path_after_session_reset("设备端 goodbye").await;
    }

    async fn reset_speak_path_after_mqtt_rebootstrap(&self, reason: &str) {
        self.reset_speak_path_after_session_reset(&format!("MQTT 链路重建: {reason}"))
            .await;
    }

    async fn reset_speak_path_after_session_reset(&self, reason: &str) {
        self.last_speak_path_warm_at_ms
            .store(0, Ordering::SeqCst);
        let pending = self.pending_speak_request.lock().await.take();
        if let Some(p) = pending {
            self.finish_pending_speak_request(&p, Some(reason.to_string()))
                .await;
        }
    }

    fn clear_mqtt_rebootstrap_pending(&self, reason: &str) {
        if self.mqtt_rebootstrap_pending.swap(false, Ordering::SeqCst) {
            tracing::debug!(
                device_id = %self.device_id(),
                reason,
                "清除 MQTT 会话重建标记"
            );
        }
    }

    fn mark_speak_path_warm(&self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        self.last_speak_path_warm_at_ms
            .store(now, Ordering::SeqCst);
    }

    fn current_speak_path_warm_at_ms(&self) -> i64 {
        let mut latest = self.last_speak_path_warm_at_ms.load(Ordering::SeqCst);
        let transport_ts = self.udp_last_active_ms.load(Ordering::SeqCst);
        if transport_ts > latest {
            latest = transport_ts;
        }
        latest
    }

    async fn should_send_speak_request(&self) -> bool {
        if !self.is_mqtt_transport() {
            return false;
        }
        if !self.has_hardware_endpoint() {
            return false;
        }
        if self.requires_hello_bootstrap_for_session().await {
            return true;
        }
        if self.mqtt_rebootstrap_pending.load(Ordering::SeqCst) {
            return true;
        }
        if self.is_conversation_active().await {
            return false;
        }
        let warm_at = self.current_speak_path_warm_at_ms();
        if warm_at <= 0 {
            return true;
        }
        let reuse_ms = self.app_config.chat.speak_request_reuse_window_ms;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        (now - warm_at) as u64 > reuse_ms
    }

    async fn requires_hello_bootstrap_for_session(&self) -> bool {
        if self.session.lock().await.is_some() {
            return false;
        }
        if !self.is_hello_inited() {
            return true;
        }
        self.requires_fresh_hello()
    }

    async fn is_conversation_active(&self) -> bool {
        let guard = self.session.lock().await;
        let Some(session) = guard.as_ref() else {
            return false;
        };
        let state = session.state();
        if state.welcome_playing || state.welcome_speaking {
            return true;
        }
        let phase = state.listen_phase;
        if !matches!(phase, ListenPhase::Idle) {
            return true;
        }
        if let Some(tts) = self.tts_manager.lock().await.as_ref() {
            if tts.is_tts_active() {
                return true;
            }
        }
        false
    }

    fn push_hardware_command(&self, msg: &ServerMessage) -> bool {
        let Ok(data) = serde_json::to_vec(msg) else {
            return false;
        };
        self.send_hardware_outbound_command(data)
    }

    async fn ensure_client_session_id(&self) -> Result<String> {
        let sid = {
            let guard = self.session.lock().await;
            guard
                .as_ref()
                .map(|s| s.state().session_id.clone())
                .filter(|s| !s.is_empty())
        };
        if let Some(id) = sid {
            return Ok(id);
        }
        if let Some(id) = self
            .hello_session_id
            .lock()
            .await
            .clone()
            .filter(|s| !s.is_empty())
        {
            return Ok(id);
        }
        // 离线唤醒 / 会话已释放：为 speak_request 生成临时 session_id，等设备 hello 后对齐
        let session_id = uuid::Uuid::new_v4().to_string();
        tracing::info!(
            device_id = %self.device_id(),
            session_id = %session_id,
            "主动播报链路无可用 session_id，已生成唤醒用 session_id"
        );
        self.prepare_session(session_id.clone()).await;
        Ok(session_id)
    }

    async fn get_or_create_pending_speak_request(
        &self,
        session_id: String,
    ) -> (Arc<PendingSpeakRequest>, bool) {
        let mut guard = self.pending_speak_request.lock().await;
        if let Some(existing) = guard.as_ref() {
            return (Arc::clone(existing), false);
        }
        let pending = Arc::new(PendingSpeakRequest {
            session_id,
            notify: tokio::sync::Notify::new(),
            error: Mutex::new(None),
        });
        *guard = Some(Arc::clone(&pending));
        (pending, true)
    }

    async fn finish_pending_speak_request(
        &self,
        pending: &Arc<PendingSpeakRequest>,
        err: Option<String>,
    ) {
        let mut guard = self.pending_speak_request.lock().await;
        if guard
            .as_ref()
            .is_some_and(|p| Arc::ptr_eq(p, pending))
        {
            *guard = None;
        }
        drop(guard);
        if let Some(msg) = err {
            *pending.error.lock().await = Some(msg);
        }
        pending.notify.notify_waiters();
    }

    async fn wait_for_injected_speech_session(self: &Arc<Self>) -> Result<()> {
        let deadline = Instant::now()
            + Duration::from_millis(self.app_config.chat.speak_ready_timeout_ms.max(1000));
        while Instant::now() < deadline {
            if self.session.lock().await.is_some() {
                return Ok(());
            }
            // speak_request 唤醒后需等设备 hello 建立会话；勿因 need_fresh_hello 提前失败
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Err(xiaozhi_core::Error::Session(
            "等待 ChatSession 建立超时（设备可能未 hello）".into(),
        ))
    }

    pub async fn exit_chat(self: Arc<Self>) {
        let mut delivery = SpeakDelivery::default();
        {
            let mut guard = self.session.lock().await;
            let Some(session) = guard.as_mut() else {
                return;
            };
            if session
                .speak_text_for_delivery("好的，再见！期待下次与您聊天～", &mut delivery)
                .await
                .is_err()
            {
                return;
            }
        }
        let _ = self.push_delivery(&delivery).await;
        self.close_session_with_reason(crate::SessionCloseReason::ExplicitExit)
            .await;
    }

    /// 对齐 Go `newInjectedSpeechStartHook`：TTS 实际开播时标记热链路
    pub fn mark_injected_speech_playback_started(&self) {
        if self.is_mqtt_transport() {
            self.mark_speak_path_warm();
        }
    }

    pub fn injected_speech_playback_hook(self: &Arc<Self>) -> Option<TtsPlaybackStartHook> {
        if !self.is_mqtt_transport() {
            return None;
        }
        let once = Arc::new(AtomicBool::new(false));
        let weak = Arc::downgrade(self);
        Some(Arc::new(move || {
            if once.swap(true, Ordering::SeqCst) {
                return;
            }
            if let Some(mgr) = weak.upgrade() {
                mgr.mark_injected_speech_playback_started();
            }
        }))
    }

    /// 对齐 Go `handleTTSTurnEndPolicy`（`ttsTurnEndPolicyGoodbyeAndIdle`）
    pub async fn handle_tts_turn_end_policy(self: Arc<Self>, policy: TtsTurnEndPolicy) {
        if policy != TtsTurnEndPolicy::GoodbyeAndIdle {
            self.clear_injected_speech_guard();
            self.try_resume_pending_listen_start().await;
            return;
        }
        if !self.is_mqtt_transport() && !self.has_hardware_endpoint() {
            self.clear_injected_speech_guard();
            self.try_resume_pending_listen_start().await;
            return;
        }
        tokio::time::sleep(Duration::from_millis(
            crate::tts_manager::TTS_PLAYBACK_COMPLETION_GRACE_MS,
        ))
        .await;
        let has_session = self.session.lock().await.is_some();
        if has_session {
            Arc::clone(&self)
                .close_session_with_reason(crate::SessionCloseReason::ExplicitExit)
                .await;
        } else {
            tracing::info!(
                device_id = %self.device_id(),
                "主动播报结束时会话已不存在，仍下发 goodbye"
            );
            self.mark_need_fresh_hello();
            if self
                .push_messages(&[ServerMessage::goodbye(self.session_id())])
                .await
                .unwrap_or(false)
            {
                tracing::info!(
                    device_id = %self.device_id(),
                    "MQTT goodbye 已下发（主动播报收尾）"
                );
            }
        }
        self.clear_injected_speech_guard();
    }
}
