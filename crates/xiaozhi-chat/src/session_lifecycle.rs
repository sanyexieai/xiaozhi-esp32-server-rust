//! 会话关闭 / goodbye 保留 / 音频空闲超时（对齐 Go `chat.go` + `asr.go`）

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use xiaozhi_protocol::messages::ServerMessage;

use crate::manager::ChatManager;
use crate::state::ListenPhase;

const DEFAULT_RETAINED_SESSION_IDLE_TTL: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionCloseReason {
    AudioIdleTimeout,
    RetainedIdleTimeout,
    ExplicitExit,
    FatalError,
    ManagerShutdown,
}

impl SessionCloseReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::AudioIdleTimeout => "audio_idle_timeout",
            Self::RetainedIdleTimeout => "retained_idle_timeout",
            Self::ExplicitExit => "explicit_exit",
            Self::FatalError => "fatal_error",
            Self::ManagerShutdown => "manager_shutdown",
        }
    }

    fn should_send_mqtt_goodbye(self) -> bool {
        matches!(
            self,
            Self::AudioIdleTimeout | Self::ExplicitExit | Self::FatalError
        )
    }
}

impl ChatManager {
    pub fn max_idle_duration_ms(&self) -> u64 {
        let v = self.app_config.chat.max_idle_duration;
        if v == 0 {
            u64::MAX
        } else {
            v
        }
    }

    pub async fn on_device_activity(&self) {
        self.cancel_retained_session_cleanup("收到设备活动消息").await;
    }

    pub async fn cancel_retained_session_cleanup(&self, reason: &str) {
        let mut guard = self.retained_cleanup.lock().await;
        if let Some(handle) = guard.take() {
            handle.abort();
            tracing::debug!(
                device_id = %self.device_id(),
                reason,
                "已取消 goodbye 保留态清理定时器"
            );
        }
    }

    /// 对齐 Go `HandleGoodByeMessage`：重置会话并清理音频链路
    pub async fn handle_device_goodbye(self: Arc<Self>) {
        tracing::info!(
            device_id = %self.device_id(),
            "收到设备端 goodbye，保留 ChatSession 并重置为静默态"
        );
        if let Some(tts) = self.tts_manager().await {
            tts.interrupt_and_stop_sync(true, "HandleGoodByeMessage")
                .await;
        }
        self.invalidate_listen_start();
        {
            let mut guard = self.session.lock().await;
            if let Some(session) = guard.as_mut() {
                session.reset_to_silent_state(self.as_ref()).await;
            }
        }
        self.reset_speak_path_after_goodbye().await;
        self.schedule_retained_session_cleanup("peer_goodbye")
            .await;
    }

    pub(crate) async fn schedule_retained_session_cleanup(self: Arc<Self>, reason: &str) {
        let session_id = {
            let guard = self.session.lock().await;
            guard
                .as_ref()
                .map(|s| s.state().session_id.clone())
                .unwrap_or_default()
        };
        if session_id.is_empty() {
            return;
        }
        let ttl_ms = self.app_config.chat.retained_session_idle_timeout_ms;
        let ttl = if ttl_ms == 0 {
            DEFAULT_RETAINED_SESSION_IDLE_TTL
        } else {
            Duration::from_millis(ttl_ms)
        };
        self.cancel_retained_session_cleanup("reschedule_retained_cleanup")
            .await;
        let mgr = Arc::clone(&self);
        let target = session_id.clone();
        let reason = reason.to_string();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(ttl).await;
            let current = {
                let guard = mgr.session.lock().await;
                guard
                    .as_ref()
                    .map(|s| s.state().session_id.clone())
                    .unwrap_or_default()
            };
            if current != target {
                return;
            }
            tracing::info!(
                device_id = %mgr.device_id(),
                ttl_secs = ttl.as_secs(),
                reason,
                "ChatSession 保留态空闲超时，执行彻底清理"
            );
            mgr.close_session_with_reason(SessionCloseReason::RetainedIdleTimeout)
                .await;
        });
        *self.retained_cleanup.lock().await = Some(handle);
    }

    /// 管理端调试：主动结束会话并下发 MQTT goodbye（对齐设备回主页）
    pub async fn request_explicit_goodbye(self: Arc<Self>) {
        self.close_session_with_reason(SessionCloseReason::ExplicitExit)
            .await;
    }

    pub async fn close_session_with_reason(self: Arc<Self>, reason: SessionCloseReason) {
        if self.session_closing.swap(true, Ordering::SeqCst) {
            tracing::debug!(
                device_id = %self.device_id(),
                reason = reason.as_str(),
                "会话关闭已在进行中，跳过重复请求"
            );
            return;
        }

        if let Some(handle) = self.idle_watchdog.lock().await.take() {
            handle.abort();
        }
        self.cancel_retained_session_cleanup("session_closed").await;

        let closing_session_id = {
            let guard = self.session.lock().await;
            guard
                .as_ref()
                .map(|s| s.state().session_id.clone())
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    self.hello_session_id
                        .try_lock()
                        .ok()
                        .and_then(|g| g.clone())
                        .filter(|s| !s.is_empty())
                })
        };

        if let Some(tts) = self.tts_manager().await {
            tts.interrupt_and_stop_sync(true, reason.as_str())
                .await;
        }

        let taken_session = {
            let mut guard = self.session.lock().await;
            guard.take()
        };
        let Some(mut session) = taken_session else {
            self.session_closing.store(false, Ordering::SeqCst);
            return;
        };

        self.clear_injected_speech_guard();

        match reason {
            SessionCloseReason::ManagerShutdown | SessionCloseReason::RetainedIdleTimeout => {
                if let Some(tts) = self.tts_manager().await {
                    tts.end_exclusive_media_playback();
                }
                self.media_player.stop().await;
            }
            _ => {
                self.media_player.detach_session(true).await;
            }
        }

        *self.tts_manager.lock().await = None;
        *self.llm_manager.lock().await = None;
        *self.session_media.lock().await = None;
        *self.session_abort.lock().await = None;

        // 释放关闭门闩，允许 hello / init_session 立即重建会话；慢清理放后台。
        self.session_closing.store(false, Ordering::SeqCst);

        tracing::info!(
            device_id = %self.device_id(),
            reason = reason.as_str(),
            "ChatSession 已关闭（资源已释放）"
        );

        let mgr = Arc::clone(&self);
        let sid = closing_session_id;
        if reason == SessionCloseReason::ManagerShutdown {
            mgr.send_mqtt_goodbye_for_close(reason, sid).await;
            session.reset_to_silent_state(mgr.as_ref()).await;
            drop(session);
            mgr.apply_session_closed_side_effects(reason).await;
            return;
        }

        tokio::spawn(async move {
            mgr.send_mqtt_goodbye_for_close(reason, sid).await;
            session.reset_to_silent_state(mgr.as_ref()).await;
            drop(session);
            mgr.apply_session_closed_side_effects(reason).await;
        });
    }

    /// 关闭流程卡住时强制恢复，避免 Web 模拟器重连长期失败。
    pub(crate) async fn force_recover_stuck_session_close(&self, reason: &str) {
        if !self.session_closing.load(Ordering::SeqCst) {
            return;
        }
        tracing::warn!(
            device_id = %self.device_id(),
            reason,
            "强制结束卡住的会话关闭流程"
        );
        if let Some(handle) = self.idle_watchdog.lock().await.take() {
            handle.abort();
        }
        self.cancel_retained_session_cleanup(reason).await;
        if let Some(tts) = self.tts_manager().await {
            tts.interrupt_and_stop_sync(true, reason).await;
        }
        {
            let mut guard = self.session.lock().await;
            guard.take();
        }
        *self.tts_manager.lock().await = None;
        *self.llm_manager.lock().await = None;
        *self.session_media.lock().await = None;
        *self.session_abort.lock().await = None;
        self.clear_injected_speech_guard();
        self.session_closing.store(false, Ordering::SeqCst);
        self.mark_need_fresh_hello();
    }

    async fn send_mqtt_goodbye_for_close(
        &self,
        reason: SessionCloseReason,
        closing_session_id: Option<String>,
    ) {
        if reason == SessionCloseReason::ManagerShutdown
            || reason == SessionCloseReason::RetainedIdleTimeout
            || !reason.should_send_mqtt_goodbye()
        {
            return;
        }
        let session_id = closing_session_id.or_else(|| self.session_id());
        if self.outbound_tx().is_none() {
            tracing::warn!(
                device_id = %self.device_id(),
                reason = reason.as_str(),
                "会话关闭但无 outbound 通道，无法下发 goodbye"
            );
            return;
        }
        if self
            .push_messages(&[ServerMessage::goodbye(session_id.clone())])
            .await
            .unwrap_or(false)
        {
            tracing::info!(
                device_id = %self.device_id(),
                reason = reason.as_str(),
                session_id = session_id.as_deref().unwrap_or(""),
                "MQTT goodbye 已下发"
            );
        } else {
            tracing::warn!(
                device_id = %self.device_id(),
                reason = reason.as_str(),
                session_id = session_id.as_deref().unwrap_or(""),
                "MQTT goodbye 下发失败"
            );
        }
    }

    async fn apply_session_closed_side_effects(&self, reason: SessionCloseReason) {
        if reason == SessionCloseReason::ManagerShutdown {
            return;
        }
        if reason == SessionCloseReason::RetainedIdleTimeout {
            self.mark_need_fresh_hello();
            self.reset_speak_path_after_server_session_close(reason.as_str())
                .await;
            return;
        }
        if !reason.should_send_mqtt_goodbye() {
            return;
        }
        self.mark_need_fresh_hello();
        self.reset_speak_path_after_server_session_close(reason.as_str())
            .await;
    }

    pub async fn spawn_audio_idle_watchdog(self: Arc<Self>) {
        if let Some(handle) = self.idle_watchdog.lock().await.take() {
            handle.abort();
        }
        if self.max_idle_duration_ms() == u64::MAX {
            return;
        }
        let mgr = Arc::clone(&self);
        let handle = tokio::spawn(async move {
            crate::asr_manager::run_audio_idle_timeout_watchdog(mgr).await;
        });
        *self.idle_watchdog.lock().await = Some(handle);
    }

    pub(crate) fn should_count_audio_idle_timeout_for(
        &self,
        session: &crate::session::ChatSession,
        tts_active: bool,
    ) -> bool {
        if tts_active {
            return false;
        }
        let state = session.state();
        if state.welcome_playing || state.welcome_speaking {
            return false;
        }
        !matches!(
            state.listen_phase,
            ListenPhase::Speaking | ListenPhase::Processing
        )
    }

    pub(crate) async fn trigger_audio_idle_timeout(self: Arc<Self>) {
        let tts_active = self
            .tts_manager()
            .await
            .map(|t| t.is_tts_active())
            .unwrap_or(false);
        let output_active = {
            let guard = self.session.lock().await;
            guard.as_ref().is_some_and(|s| {
                let st = s.state();
                tts_active
                    || st.welcome_playing
                    || st.welcome_speaking
                    || matches!(
                        st.listen_phase,
                        ListenPhase::Speaking | ListenPhase::Processing
                    )
            })
        };
        if output_active {
            tracing::debug!(
                device_id = %self.device_id(),
                "播报/处理进行中，忽略音频空闲超时"
            );
            let mut guard = self.session.lock().await;
            if let Some(session) = guard.as_mut() {
                session.state_mut().clear_audio_idle_timeout_pending();
            }
            return;
        }

        let already_pending = {
            let guard = self.session.lock().await;
            guard
                .as_ref()
                .map(|s| s.state().audio_idle_timeout_pending())
                .unwrap_or(false)
        };
        if already_pending {
            self.close_audio_idle_timeout_if_pending().await;
            return;
        }

        let has_asr = {
            let guard = self.session.lock().await;
            guard
                .as_ref()
                .map(|s| s.asr_pipeline().has_open_audio_input())
                .unwrap_or(false)
        };
        if has_asr {
            tracing::info!(
                device_id = %self.device_id(),
                "音频空闲超时，触发 ASR 收口"
            );
            let mut guard = self.session.lock().await;
            if let Some(session) = guard.as_mut() {
                session.asr_pipeline_mut().on_voice_silence();
            }
            return;
        }
        tracing::info!(
            device_id = %self.device_id(),
            "音频空闲超时，当前无活动 ASR 流，关闭会话"
        );
        self.close_session_with_reason(SessionCloseReason::AudioIdleTimeout)
            .await;
    }

    pub(crate) async fn close_audio_idle_timeout_if_pending(self: Arc<Self>) {
        let pending = {
            let guard = self.session.lock().await;
            guard
                .as_ref()
                .map(|s| s.state().audio_idle_timeout_pending())
                .unwrap_or(false)
        };
        if !pending {
            return;
        }
        {
            let mut guard = self.session.lock().await;
            if let Some(session) = guard.as_mut() {
                session.state_mut().clear_audio_idle_timeout_pending();
            }
        }
        self.close_session_with_reason(SessionCloseReason::AudioIdleTimeout)
            .await;
    }
}
