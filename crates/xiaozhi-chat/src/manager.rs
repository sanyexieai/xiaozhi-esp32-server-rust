use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicU8, Ordering};
use std::sync::Mutex as StdMutex;
use std::sync::OnceLock;
use std::time::Duration;
use std::collections::HashMap;

use dashmap::DashMap;
use tokio::sync::{Mutex, mpsc};
use xiaozhi_asr::create_asr;
use xiaozhi_config::AppConfig;
use xiaozhi_config_provider::UserConfigProvider;
use xiaozhi_core::Result;
use xiaozhi_history::{HistoryClient, HistoryMessageInput};
use xiaozhi_llm::{create_llm, ToolInfo};
use xiaozhi_memory::create_memory;
use xiaozhi_mcp::{McpManager, McpRequest, prepare_tools_for_llm};
use xiaozhi_openclaw::OpenClawManager;
use xiaozhi_rag::KnowledgeClient;
use xiaozhi_speaker::create_speaker;
use xiaozhi_protocol::messages::ServerMessage;
use xiaozhi_tts::create_tts;
use xiaozhi_vad::create_vad;

use crate::speak_path::PendingSpeakRequest;
use crate::device_mcp::{
    call_device_tool, call_device_tool_raw, has_mcp_feature, refresh_device_tools_json,
    run_device_mcp_init, run_device_mcp_init_json, DeviceMcpRuntime, McpInboundAction,
};
use crate::knowledge::{
    collect_searchable_kb_ids, default_knowledge_search_threshold, format_knowledge_hits_for_llm,
    has_available_knowledge_bases,
};
use crate::chat_queue::{ChatTextJob, ChatTextQueue};
use crate::endpoint_hub::{EndpointHub, EndpointKind, EndpointRegistration, TtsAudioRoute};
use crate::llm_manager::{LlmManager, LlmTurnResult};
use crate::resource_pools::SharedResourcePools;
use crate::session_media::SessionMedia;
use crate::media_player::SessionMediaPlayer;
use crate::openclaw_warmup::OpenClawWarmupController;
use crate::outbound::{OutboundFrame, SpeakDelivery};

use crate::session::{ChatSession, UplinkPcm};
use crate::signal_log::SignalLog;
use crate::state::{ClientState, ListenPhase};
use crate::tts_manager::TtsManager;
use crate::tts_turn_policy::injected_speech_tts_turn_end_policy;

const MAX_PRE_LISTEN_FRAMES: usize = 12;
const DETECT_LLM_DEBOUNCE_MS: u64 = 300;

pub struct McpToolLlmOutcome {
    pub text: String,
    pub stop_llm: bool,
}

pub struct ChatManager {
    pub(crate) device_id: String,
    pub(crate) session: Mutex<Option<ChatSession>>,
    init_lock: Mutex<()>,
    pub(crate) app_config: AppConfig,
    pub(crate) config_provider: Arc<dyn UserConfigProvider>,
    history: Arc<HistoryClient>,
    openclaw: Arc<OpenClawManager>,
    mcp_manager: Arc<McpManager>,
    knowledge_client: Arc<KnowledgeClient>,
    pre_listen_pcm: Mutex<Vec<Vec<f32>>>,
    /// 多端出站：Web / 硬件 / 工具链
    endpoint_hub: EndpointHub,
    session_abort: Mutex<Option<Arc<AtomicBool>>>,
    pub(crate) device_mcp: Mutex<DeviceMcpRuntime>,
    mcp_session_id: Mutex<String>,
    resource_pools: Option<Arc<SharedResourcePools>>,
    binary_protocol_version: AtomicU8,
    need_fresh_hello: AtomicBool,
    hello_inited: AtomicBool,
    mcp_feature_enabled: AtomicBool,
    listen_start_seq: AtomicU64,
    realtime_listen_active: AtomicBool,
    detect_llm_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    chat_queue: OnceLock<ChatTextQueue>,
    llm_tool_aliases: Mutex<HashMap<String, String>>,
    pub(crate) tts_manager: Mutex<Option<Arc<TtsManager>>>,
    llm_manager: Mutex<Option<Arc<LlmManager>>>,
    session_media: Mutex<Option<SessionMedia>>,
    idle_watchdog: Mutex<Option<tokio::task::JoinHandle<()>>>,
    retained_cleanup: Mutex<Option<tokio::task::JoinHandle<()>>>,
    pub(crate) is_mqtt_transport: AtomicBool,
    pub(crate) mqtt_rebootstrap_pending: AtomicBool,
    pub(crate) last_speak_path_warm_at_ms: AtomicI64,
    pub(crate) pending_speak_request: Mutex<Option<Arc<PendingSpeakRequest>>>,
    pub(crate) hello_session_id: Mutex<Option<String>>,
    pub(crate) has_udp_binding: AtomicBool,
    pub(crate) udp_last_active_ms: AtomicI64,
    /// MQTT 主动注入播报进行中（含 speak_ready 等待至 TTS 结束），用于防 goodbye / listen start 打断
    pub(crate) injected_speech_guard: AtomicBool,
    /// 会话正在关闭，阻止 ensure_session 在 take 与清理之间重建会话
    session_closing: AtomicBool,
    openclaw_warmup: Mutex<OpenClawWarmupController>,
    media_player: SessionMediaPlayer,
    signal_log: Arc<SignalLog>,
}

impl ChatManager {
    pub async fn new(
        device_id: String,
        app_config: AppConfig,
        config_provider: Arc<dyn UserConfigProvider>,
        history: Arc<HistoryClient>,
        openclaw: Arc<OpenClawManager>,
        mcp_manager: Arc<McpManager>,
        knowledge_client: Arc<KnowledgeClient>,
        resource_pools: Option<Arc<SharedResourcePools>>,
    ) -> Result<Self> {
        Ok(Self {
            device_id: device_id.clone(),
            session: Mutex::new(None),
            hello_session_id: Mutex::new(None),
            init_lock: Mutex::new(()),
            app_config,
            config_provider,
            history,
            openclaw,
            mcp_manager,
            knowledge_client,
            pre_listen_pcm: Mutex::new(Vec::new()),
            endpoint_hub: EndpointHub::new(),
            session_abort: Mutex::new(None),
            device_mcp: Mutex::new(DeviceMcpRuntime::default()),
            mcp_session_id: Mutex::new(String::new()),
            resource_pools,
            binary_protocol_version: AtomicU8::new(1),
            need_fresh_hello: AtomicBool::new(true),
            hello_inited: AtomicBool::new(false),
            mcp_feature_enabled: AtomicBool::new(false),
            listen_start_seq: AtomicU64::new(0),
            realtime_listen_active: AtomicBool::new(false),
            detect_llm_handle: Mutex::new(None),
            chat_queue: OnceLock::new(),
            llm_tool_aliases: Mutex::new(HashMap::new()),
            tts_manager: Mutex::new(None),
            llm_manager: Mutex::new(None),
            session_media: Mutex::new(None),
            idle_watchdog: Mutex::new(None),
            retained_cleanup: Mutex::new(None),
            is_mqtt_transport: AtomicBool::new(false),
            mqtt_rebootstrap_pending: AtomicBool::new(false),
            last_speak_path_warm_at_ms: AtomicI64::new(0),
            pending_speak_request: Mutex::new(None),
            has_udp_binding: AtomicBool::new(false),
            udp_last_active_ms: AtomicI64::new(0),
            injected_speech_guard: AtomicBool::new(false),
            session_closing: AtomicBool::new(false),
            openclaw_warmup: Mutex::new(OpenClawWarmupController::default()),
            media_player: SessionMediaPlayer::new(device_id.clone()),
            signal_log: Arc::new(SignalLog::new()),
        })
    }

    fn init_chat_queue(self: &Arc<Self>) {
        let _ = self
            .chat_queue
            .get_or_init(|| ChatTextQueue::new(Arc::clone(self)));
    }

    pub async fn enqueue_chat_text(self: &Arc<Self>, text: String, send_stt: bool) {
        if text.trim().is_empty() {
            return;
        }
        if self.try_handle_realtime_media_asr(&text).await {
            return;
        }
        self.init_chat_queue();
        let job = ChatTextJob { text, send_stt };
        if let Some(q) = self.chat_queue.get() {
            if !q.try_enqueue(job) {
                tracing::warn!(device_id = %self.device_id, "chatTextQueue 已满，丢弃消息");
            }
        }
    }

    pub async fn is_session_realtime(&self) -> bool {
        self.session
            .lock()
            .await
            .as_ref()
            .map(|s| s.state().is_realtime())
            .unwrap_or(false)
    }

    pub async fn is_allowed_asr_restart(&self, start_seq: u64) -> bool {
        if !self.is_current_listen_start(start_seq) {
            return false;
        }
        let guard = self.session.lock().await;
        let Some(session) = guard.as_ref() else {
            return false;
        };
        if session.state().is_realtime() {
            return matches!(
                session.state().listen_phase,
                ListenPhase::Listening | ListenPhase::Processing | ListenPhase::Speaking
            );
        }
        session.state().listen_phase == ListenPhase::Listening
    }
    pub fn set_binary_protocol_version(&self, version: u8) {
        let v = version.max(1);
        self.binary_protocol_version.store(v, Ordering::Relaxed);
    }

    pub fn binary_protocol_version(&self) -> u8 {
        self.binary_protocol_version.load(Ordering::Relaxed).max(1)
    }

    pub fn config_provider(&self) -> Arc<dyn UserConfigProvider> {
        Arc::clone(&self.config_provider)
    }

    pub fn requires_fresh_hello(&self) -> bool {
        self.need_fresh_hello.load(Ordering::SeqCst)
    }

    pub fn is_hello_inited(&self) -> bool {
        self.hello_inited.load(Ordering::SeqCst)
    }

    pub fn mark_hello_ready(&self) {
        self.hello_inited.store(true, Ordering::SeqCst);
        self.need_fresh_hello.store(false, Ordering::SeqCst);
    }

    /// 对齐 Go `setNeedFreshHello`：仅标记下次 bootstrap 前需重新 hello，不清除 `hello_inited`。
    pub fn mark_need_fresh_hello(&self) {
        self.need_fresh_hello.store(true, Ordering::SeqCst);
    }

    pub fn begin_listen_start(&self, mode: &str) -> u64 {
        if mode == "realtime" {
            self.realtime_listen_active.store(true, Ordering::SeqCst);
        }
        self.listen_start_seq.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn invalidate_listen_start(&self) {
        self.listen_start_seq.fetch_add(1, Ordering::SeqCst);
        self.realtime_listen_active.store(false, Ordering::SeqCst);
    }

    pub fn is_current_listen_start(&self, seq: u64) -> bool {
        self.listen_start_seq.load(Ordering::SeqCst) == seq
    }

    pub fn is_realtime_listen_active(&self) -> bool {
        self.realtime_listen_active.load(Ordering::SeqCst)
    }

    pub async fn cancel_pending_detect_llm(&self) {
        if let Some(handle) = self.detect_llm_handle.lock().await.take() {
            handle.abort();
        }
    }

    pub async fn schedule_detect_llm(self: &Arc<Self>, text: String) {
        self.cancel_pending_detect_llm().await;
        let mgr = Arc::clone(self);
        let handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(DETECT_LLM_DEBOUNCE_MS)).await;
            let phase = mgr
                .session
                .lock()
                .await
                .as_ref()
                .map(|s| s.state().listen_phase);
            if matches!(
                phase,
                Some(ListenPhase::Listening) | Some(ListenPhase::Processing)
            ) {
                tracing::debug!(
                    device_id = %mgr.device_id(),
                    ?phase,
                    "detect 文本在聆听中到达，直接入队对话"
                );
                mgr.enqueue_chat_text(text, true).await;
                return;
            }
            if phase != Some(ListenPhase::Idle) {
                tracing::debug!(
                    device_id = %mgr.device_id(),
                    ?phase,
                    "detect LLM debounce 跳过: 非 Idle 阶段"
                );
                return;
            }
            // MQTT 硬件需先走 speak_request → speak_ready 唤醒播报链路
            if mgr.is_mqtt_transport() && mgr.has_hardware_endpoint() {
                let speak_ready = mgr
                    .prepare_speak_path_for_injected_speech(&text, true)
                    .await
                    .is_ok();
                if speak_ready {
                    mgr.enqueue_chat_text(text, false).await;
                } else if mgr.has_web_endpoint() {
                    tracing::info!(
                        device_id = %mgr.device_id(),
                        "硬件 speak_ready 超时，降级为 Web 端继续对话"
                    );
                    mgr.set_tts_audio_route(TtsAudioRoute::WebOnly);
                    mgr.enqueue_chat_text(text, true).await;
                } else {
                    tracing::warn!(
                        device_id = %mgr.device_id(),
                        "detect LLM 硬件播报链路准备失败且无 Web 端点，已放弃"
                    );
                }
                return;
            }
            mgr.enqueue_chat_text(text, true).await;
        });
        *self.detect_llm_handle.lock().await = Some(handle);
    }

    pub async fn check_device_activated(self: &Arc<Self>) -> Result<bool> {
        if !self.app_config.auth.enable {
            return Ok(true);
        }
        let activated = self
            .config_provider
            .is_device_activated(&self.device_id, "client_id")
            .await?;
        if activated {
            return Ok(true);
        }
        let delivery = {
            let mut guard = self.session.lock().await;
            let Some(session) = guard.as_mut() else {
                return Ok(false);
            };
            session.handle_not_activated().await?
        };
        let _ = self.push_delivery(&delivery).await;
        Ok(false)
    }

    pub async fn handle_listen_message(
        self: &Arc<Self>,
        state: Option<&str>,
        mode: Option<&str>,
        text: Option<&str>,
    ) -> Result<()> {
        if self.session.lock().await.is_none() {
            if let Err(e) = self.clone().ensure_session().await {
                tracing::warn!(
                    device_id = %self.device_id,
                    "listen 前会话初始化失败: {e:#}"
                );
                return Ok(());
            }
        }
        if self.requires_fresh_hello() {
            tracing::warn!(
                device_id = %self.device_id,
                "会话需重新 hello，忽略 listen 消息"
            );
            return Ok(());
        }
        match state {
            Some(xiaozhi_core::message::START) => {
                self.on_listen_start(mode).await;
            }
            Some(xiaozhi_core::message::STOP) => {
                let _ = self.on_listen_stop().await?;
            }
            Some(xiaozhi_core::message::DETECT) => {
                self.on_listen_detect(text).await;
            }
            Some("text") => {
                if !self.check_device_activated().await.unwrap_or(false) {
                    return Ok(());
                }
                let text = text.unwrap_or("").trim();
                if !text.is_empty() {
                    self.enqueue_chat_text(text.to_string(), true).await;
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub async fn dispatch_asr_final_text(self: &Arc<Self>, text: String) -> Result<()> {
        if !text.trim().is_empty() {
            self.enqueue_chat_text(text, true).await;
        }
        Ok(())
    }

    /// Realtime 模式下音乐播放 ASR 门控（对齐 Go `tryHandleRealtimeMcpAudioASR`）
    pub async fn try_handle_realtime_media_asr(self: &Arc<Self>, text: &str) -> bool {
        if !self.is_session_realtime().await {
            return false;
        }
        if !self.media_player.has_realtime_control_context().await {
            return false;
        }

        let trimmed = text.trim();

        if crate::realtime_media_gate::is_media_exit_command(trimmed) {
            tracing::info!(
                device_id = %self.device_id,
                text = %trimmed,
                "realtime 媒体播放门控命中退出指令"
            );
            let mgr = Arc::clone(self);
            tokio::spawn(async move {
                mgr.exit_chat().await;
            });
            return true;
        }

        if let Some(action) = crate::realtime_media_gate::detect_media_control_action(trimmed) {
            let args = serde_json::json!({ "action": action });
            if let Err(e) = self.control_music_playback_tool(args).await {
                tracing::warn!(
                    device_id = %self.device_id,
                    action,
                    text = %trimmed,
                    "realtime 媒体播放门控执行控制动作失败: {e}"
                );
            } else {
                tracing::info!(
                    device_id = %self.device_id,
                    action,
                    text = %trimmed,
                    "realtime 媒体播放门控执行控制动作"
                );
            }
            return true;
        }

        if self.media_player.should_gate_realtime_asr().await {
            tracing::debug!(
                device_id = %self.device_id,
                text = %trimmed,
                "realtime 媒体播放门控忽略 ASR 文本"
            );
            return true;
        }

        false
    }

    pub async fn is_welcome_playing(&self) -> bool {
        let guard = self.session.lock().await;
        guard
            .as_ref()
            .map(|s| s.state().welcome_playing)
            .unwrap_or(false)
    }

    /// 对齐 Go `completeWelcomePlaybackWait`
    pub async fn complete_welcome_playback_wait(&self, natural: bool) {
        let mut guard = self.session.lock().await;
        if let Some(session) = guard.as_mut() {
            session.complete_welcome_playback_wait(natural).await;
        }
    }

    pub async fn dispatch_asr_empty_result(self: &Arc<Self>) -> Result<bool> {
        if self.is_welcome_playing().await {
            tracing::debug!(
                device_id = %self.device_id,
                "欢迎语播放中，跳过 ASR empty recovery"
            );
            return Ok(false);
        }
        let delivery = {
            let mut guard = self.session.lock().await;
            let session = guard
                .as_mut()
                .ok_or_else(|| xiaozhi_core::Error::Session("会话未初始化".into()))?;
            session.empty_listen_recovery_session("没听清楚，请再说一遍").await?
        };
        self.push_delivery(&delivery).await
    }

    pub async fn on_listen_detect(self: &Arc<Self>, text: Option<&str>) {
        if self.requires_fresh_hello() {
            return;
        }
        if !self.check_device_activated().await.unwrap_or(false) {
            return;
        }
        if self.session.lock().await.is_none() {
            if let Err(e) = self.clone().ensure_session().await {
                tracing::error!(device_id = %self.device_id, "detect 前会话初始化失败: {e:#}");
                return;
            }
        }
        let delivery = {
            let mut guard = self.session.lock().await;
            let Some(session) = guard.as_mut() else {
                return;
            };
            match session.handle_listen_detect(text).await {
                Ok(d) => d,
                Err(e) => {
                    tracing::error!(device_id = %self.device_id, "detect 处理失败: {e:#}");
                    return;
                }
            }
        };
        if !delivery.messages.is_empty() || !delivery.audio_frames.is_empty() {
            let _ = self.push_delivery(&delivery).await;
        }
    }

    pub async fn prepare_session(&self, session_id: String) {
        *self.hello_session_id.lock().await = Some(session_id);
    }

    /// 当前服务端会话 ID（Web 恢复会话 / 硬件 hello 对齐用）
    pub async fn active_session_id(&self) -> Option<String> {
        if let Some(session) = self.session.lock().await.as_ref() {
            let sid = session.state().session_id.clone();
            if !sid.is_empty() {
                return Some(sid);
            }
        }
        self.hello_session_id
            .lock()
            .await
            .clone()
            .filter(|s| !s.is_empty())
    }

    pub async fn ensure_session(self: Arc<Self>) -> Result<()> {
        if self.session_closing.load(Ordering::SeqCst) {
            self.wait_session_closing_done().await?;
        }
        if self.session.lock().await.is_some() {
            return Ok(());
        }
        let session_id = {
            let _guard = self.init_lock.lock().await;
            if self.session.lock().await.is_some() {
                return Ok(());
            }
            self.hello_session_id
                .lock()
                .await
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
        };
        self.init_session(session_id).await
    }

    pub fn schedule_mcp_init(self: &Arc<Self>, session_id: String, features: Option<serde_json::Value>) {
        if !has_mcp_feature(&features) {
            return;
        }
        self.mcp_feature_enabled.store(true, Ordering::SeqCst);
        self.schedule_mcp_init_internal(session_id);
    }

    /// 对齐 Go `WarmupMcp`：transport ready 后尝试重新初始化设备 MCP
    pub fn warmup_mcp(self: &Arc<Self>) {
        if !self.mcp_feature_enabled.load(Ordering::SeqCst) {
            return;
        }
        let session_id = self
            .mcp_session_id
            .try_lock()
            .ok()
            .map(|g| g.clone())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                self.hello_session_id
                    .try_lock()
                    .ok()
                    .and_then(|g| g.clone())
            });
        let Some(session_id) = session_id else {
            tracing::debug!(
                device_id = %self.device_id,
                "warmup MCP 跳过：尚无 session_id"
            );
            return;
        };
        self.schedule_mcp_init_internal(session_id);
    }

    fn schedule_mcp_init_internal(self: &Arc<Self>, session_id: String) {
        let vision_url = self.app_config.vision.vision_url.clone();
        if vision_url.trim().is_empty() {
            tracing::warn!("设备 {} 启用 MCP 但未配置 vision.vision_url", self.device_id);
        }
        let mgr = Arc::clone(self);
        tokio::spawn(async move {
            *mgr.mcp_session_id.lock().await = session_id.clone();
            {
                let mut mcp = mgr.device_mcp.lock().await;
                if !mcp.try_begin_init() {
                    return;
                }
            }

            let pending = mgr.device_mcp.lock().await.pending_hub();

            let send = |msg: ServerMessage| {
                let mgr = mgr.clone();
                async move { mgr.push_messages(&[msg]).await.unwrap_or(false) }
            };

            match run_device_mcp_init(&session_id, &vision_url, &pending, send).await {
                Ok(tools) => {
                    let count = tools.len();
                    mgr.device_mcp.lock().await.mark_ready(tools);
                    tracing::info!("设备 {} MCP 就绪，{} 个工具", mgr.device_id, count);
                }
                Err(e) => {
                    mgr.device_mcp.lock().await.mark_failed();
                    tracing::warn!("设备 {} MCP 初始化失败: {e}", mgr.device_id);
                }
            }
        });
    }

    pub async fn collect_llm_tools(&self) -> Vec<ToolInfo> {
        let bases = self.device_knowledge_bases().await;
        self.collect_llm_tools_for_bases(&bases).await
    }

    /// 在已持有 `session` 锁时调用，避免 `collect_llm_tools` 内再次 `session.lock` 死锁。
    pub async fn collect_llm_tools_for_bases(
        &self,
        knowledge_bases: &[xiaozhi_config::user::KnowledgeBaseRef],
    ) -> Vec<ToolInfo> {
        let mut raw = self.mcp_manager.list_all_tools().await;
        let known_names: std::collections::HashSet<_> =
            raw.iter().map(|t| t.name.clone()).collect();

        let device_mcp = self.device_mcp.lock().await;
        for t in device_mcp.device_tools() {
            if known_names.contains(&t.name) {
                continue;
            }
            raw.push(t.clone());
        }
        drop(device_mcp);

        let (normalized, aliases) = prepare_tools_for_llm(raw);
        *self.llm_tool_aliases.lock().await = aliases;

        let mut tools: Vec<ToolInfo> = normalized
            .into_iter()
            .map(|t| ToolInfo {
                name: t.name,
                description: t.description,
                parameters: t.input_schema,
            })
            .collect();

        if !has_available_knowledge_bases(knowledge_bases) {
            let before = tools.len();
            tools.retain(|t| t.name != "search_knowledge");
            if tools.len() != before {
                tracing::info!(
                    device_id = %self.device_id,
                    "未关联可用知识库，已从 LLM 工具列表移除 search_knowledge"
                );
            }
        }

        tools
    }

    async fn device_knowledge_bases(&self) -> Vec<xiaozhi_config::user::KnowledgeBaseRef> {
        self.session
            .lock()
            .await
            .as_ref()
            .map(|s| s.state().device_config.knowledge_bases.clone())
            .unwrap_or_default()
    }

    pub(crate) async fn has_available_knowledge_base(&self) -> bool {
        has_available_knowledge_bases(&self.device_knowledge_bases().await)
    }

    async fn search_knowledge_tool(&self, arguments: serde_json::Value) -> String {
        let query = arguments
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if query.is_empty() {
            return "query 不能为空".to_string();
        }

        let top_k = arguments
            .get("top_k")
            .and_then(|v| v.as_u64())
            .unwrap_or(5)
            .max(1) as usize;

        let selected_ids: Vec<u64> = arguments
            .get("knowledge_base_ids")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.as_u64())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let knowledge_bases = self.device_knowledge_bases().await;
        let kb_ids = collect_searchable_kb_ids(&knowledge_bases, &selected_ids);
        if kb_ids.is_empty() {
            return "未找到足够相关信息".to_string();
        }

        let threshold = default_knowledge_search_threshold(&knowledge_bases, &kb_ids);
        let hits = match self
            .knowledge_client
            .search(&kb_ids, &query, top_k, threshold)
            .await
        {
            Ok(hits) => hits,
            Err(e) => return format!("信息检索失败: {e}"),
        };

        if hits.is_empty() {
            return "未找到足够相关信息".to_string();
        }

        format_knowledge_hits_for_llm(&hits)
    }

    async fn resolve_tool_invoke_name(&self, name: &str) -> String {
        let aliases = self.llm_tool_aliases.lock().await;
        aliases
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_string())
    }

    pub async fn reload_device_config(&self) -> Result<()> {
        let uconfig = self.config_provider().get_user_config(&self.device_id).await?;
        self.openclaw
            .exit_mode(&uconfig.agent_id, &self.device_id);
        let mut guard = self.session.lock().await;
        if let Some(session) = guard.as_mut() {
            let state = session.state_mut();
            state.agent_id = uconfig.agent_id.clone();
            state.device_config = uconfig.clone();
        }
        tracing::info!(
            device_id = %self.device_id,
            agent_id = %uconfig.agent_id,
            "设备配置已刷新"
        );
        Ok(())
    }

    async fn clear_conversation_history(&self) -> Result<String> {
        let mut guard = self.session.lock().await;
        let session = guard
            .as_mut()
            .ok_or_else(|| xiaozhi_core::Error::Session("会话未初始化".into()))?;
        let agent_id = session.state().agent_id.clone();
        session.memory_client().reset_memory(&agent_id).await?;
        session.state_mut().dialogue.clear();
        Ok("对话历史已清空".to_string())
    }

    async fn switch_device_role(&self, arguments: serde_json::Value) -> Result<String> {
        let role_name = arguments
            .get("role_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if role_name.is_empty() {
            return Err(xiaozhi_core::Error::Session("role_name 不能为空".into()));
        }
        let matched = self
            .config_provider()
            .switch_device_role_by_name(&self.device_id, role_name)
            .await?;
        self.reload_device_config().await?;
        Ok(format!("已切换到角色: {matched}"))
    }

    async fn restore_device_default_role(&self) -> Result<String> {
        self.config_provider()
            .restore_device_default_role(&self.device_id)
            .await?;
        self.reload_device_config().await?;
        Ok("已恢复默认角色".to_string())
    }

    async fn play_music_tool(self: &Arc<Self>, arguments: serde_json::Value) -> Result<String> {
        let name = arguments
            .get("name")
            .or_else(|| arguments.get("song_name"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if name.is_empty() {
            return Err(xiaozhi_core::Error::Session("缺少音乐名称".into()));
        }

        let (url, title) = crate::play_music::search_music(&name).await?;
        self.media_player
            .play_url(Arc::clone(self), title.clone(), url)
            .await?;
        Ok(format!("开始播放音乐: {title}"))
    }

    async fn current_agent_id(&self) -> Result<String> {
        let agent_id = self
            .session
            .lock()
            .await
            .as_ref()
            .map(|s| s.state().agent_id.clone())
            .unwrap_or_default();
        let agent_id = agent_id.trim().to_string();
        if agent_id.is_empty() {
            return Err(xiaozhi_core::Error::Session("agentID 不可用".into()));
        }
        Ok(agent_id)
    }

    /// 对齐 Go `flushQueuedMediaAudio`：媒体 pause/stop 后清理 TTS 发送队列
    async fn flush_media_audio_queue(&self, action: &str) {
        if let Some(tts) = self.tts_manager().await {
            tts.interrupt_and_stop_sync(false, &format!("media_control_{action}"))
                .await;
        }
    }

    async fn control_music_playback_tool(
        self: &Arc<Self>,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let action_raw = arguments
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let action = crate::play_music::normalize_music_playback_action(action_raw)
            .ok_or_else(|| xiaozhi_core::Error::Session(format!("不支持的控制动作: {action_raw}")))?;

        match action {
            "resume" => {
                self.media_player
                    .recover_playback_for_user(Arc::clone(self))
                    .await?;
            }
            "pause" => {
                self.media_player.pause().await;
                self.flush_media_audio_queue("pause").await;
            }
            "stop" => {
                if let Some(tts) = self.tts_manager().await {
                    tts.end_exclusive_media_playback();
                }
                self.media_player.stop().await;
                self.flush_media_audio_queue("stop").await;
            }
            "next" => {
                self.media_player.next(Arc::clone(self)).await?;
            }
            "prev" => {
                self.media_player.prev(Arc::clone(self)).await?;
            }
            "play_playlist" => {
                let agent_id = self.current_agent_id().await?;
                self.media_player
                    .play_agent_playlist(Arc::clone(self), &agent_id)
                    .await?;
            }
            "enqueue_current" => {
                let agent_id = self.current_agent_id().await?;
                let added = self
                    .media_player
                    .enqueue_current_to_agent(&agent_id)
                    .await?;
                let _ = self.media_player.resume_if_interrupted_pause().await;
                let state = self.media_player.get_state().await;
                return Ok(crate::media_player::control_result_with_added(
                    &state, action, &added,
                ));
            }
            _ => {}
        }

        let state = self.media_player.get_state().await;
        Ok(crate::media_player::control_result(&state, action))
    }

    pub fn media_player(&self) -> &SessionMediaPlayer {
        &self.media_player
    }

    pub async fn execute_tool(self: &Arc<Self>, name: &str, arguments: serde_json::Value) -> String {
        let invoke_name = self.resolve_tool_invoke_name(name).await;
        if !self.mcp_manager.is_local_tool_enabled(&invoke_name) {
            return format!("本地工具已禁用: {invoke_name}");
        }
        if invoke_name == "exit_conversation" {
            let mgr = Arc::clone(self);
            tokio::spawn(async move {
                mgr.exit_chat().await;
            });
            return "好的，再见！".to_string();
        }
        if invoke_name == "clear_conversation_history" {
            return match self.clear_conversation_history().await {
                Ok(msg) => msg,
                Err(e) => format!("清空历史失败: {e}"),
            };
        }
        if invoke_name == "switch_device_role" {
            return match self.switch_device_role(arguments).await {
                Ok(msg) => msg,
                Err(e) => format!("切换角色失败: {e}"),
            };
        }
        if invoke_name == "restore_device_default_role" {
            return match self.restore_device_default_role().await {
                Ok(msg) => msg,
                Err(e) => format!("恢复默认角色失败: {e}"),
            };
        }
        if invoke_name == "search_knowledge" {
            return self.search_knowledge_tool(arguments).await;
        }
        if invoke_name == "play_music" {
            return match self.play_music_tool(arguments).await {
                Ok(msg) => msg,
                Err(e) => format!("播放音乐失败: {e}"),
            };
        }
        if invoke_name == "control_music_playback" {
            return match self.control_music_playback_tool(arguments).await {
                Ok(v) => v.to_string(),
                Err(e) => format!("音乐控制失败: {e}"),
            };
        }
        let req = McpRequest {
            jsonrpc: "2.0".into(),
            id: serde_json::json!(1),
            method: "tools/call".into(),
            params: serde_json::json!({ "name": invoke_name, "arguments": arguments.clone() }),
        };
        let local_resp = self.mcp_manager.handle_request(req).await;
        if local_resp.error.as_ref().map(|e| e.code) != Some(-32601) {
            if let Some(err) = local_resp.error {
                return format!("工具调用失败: {}", err.message);
            }
            return local_resp
                .result
                .map(|v| v.to_string())
                .unwrap_or_else(|| "ok".to_string());
        }

        if self.mcp_manager.has_global_tool(&invoke_name) {
            match self
                .mcp_manager
                .call_global_tool(&invoke_name, arguments)
                .await
            {
                Ok(text) => return text,
                Err(e) => return e,
            }
        }
        if name != invoke_name && self.mcp_manager.has_global_tool(name) {
            match self.mcp_manager.call_global_tool(name, arguments.clone()).await {
                Ok(text) => return text,
                Err(e) => return e,
            }
        }

        let session_id = {
            let voice = self.session.lock().await;
            voice
                .as_ref()
                .map(|s| s.state().session_id.clone())
                .filter(|s| !s.is_empty())
        };
        let session_id = match session_id {
            Some(id) => id,
            None => self.mcp_session_id.lock().await.clone(),
        };

        let (ready, has_tool, request_id, pending) = {
            let mcp = self.device_mcp.lock().await;
            (
                mcp.is_ready(),
                mcp.has_tool(&invoke_name) || mcp.has_tool(name),
                mcp.allocate_request_id(),
                mcp.pending_hub(),
            )
        };

        if !ready || !has_tool {
            return format!("未找到工具: {name}");
        }
        if session_id.is_empty() {
            return "会话未初始化，无法调用设备工具".to_string();
        }

        let send = |msg: ServerMessage| {
            let mgr = Arc::clone(self);
            async move { mgr.push_messages(&[msg]).await.unwrap_or(false) }
        };

        match call_device_tool(
            &session_id,
            &invoke_name,
            arguments,
            request_id,
            &pending,
            send,
        )
        .await
        {
            Ok(text) => text,
            Err(e) => e,
        }
    }

    pub async fn handle_mcp_payload(self: &Arc<Self>, payload: &serde_json::Value) {
        let action = self.device_mcp.lock().await.handle_inbound(payload).await;
        match action {
            McpInboundAction::None => {}
            McpInboundAction::RefreshTools => {
                self.schedule_tools_refresh();
            }
            McpInboundAction::Respond(resp) => {
                let session_id = {
                    let voice = self.session.lock().await;
                    voice
                        .as_ref()
                        .map(|s| s.state().session_id.clone())
                        .filter(|s| !s.is_empty())
                };
                let session_id = match session_id {
                    Some(id) => id,
                    None => self.mcp_session_id.lock().await.clone(),
                };
                if session_id.is_empty() {
                    tracing::warn!("设备 MCP 请求无 session_id，无法回包");
                    return;
                }
                let msg = ServerMessage::mcp(&session_id, resp);
                if !self.push_messages(&[msg]).await.unwrap_or(false) {
                    tracing::warn!("设备 MCP 响应下发失败");
                }
            }
        }
    }

    pub fn schedule_tools_refresh(self: &Arc<Self>) {
        let mgr = Arc::clone(self);
        tokio::spawn(async move {
            let session_id = {
                let voice = mgr.session.lock().await;
                match voice
                    .as_ref()
                    .map(|s| s.state().session_id.clone())
                    .filter(|s| !s.is_empty())
                {
                    Some(id) => id,
                    None => {
                        drop(voice);
                        mgr.mcp_session_id.lock().await.clone()
                    }
                }
            };
            if session_id.is_empty() {
                tracing::debug!("设备 {} 工具变更通知，但无可用 MCP 通道", mgr.device_id);
                return;
            }
            let (request_id, pending) = {
                let mcp = mgr.device_mcp.lock().await;
                (mcp.allocate_request_id(), mcp.pending_hub())
            };
            let send = |payload: serde_json::Value| {
                let mgr = mgr.clone();
                let sid = session_id.clone();
                async move {
                    mgr.push_messages(&[ServerMessage::mcp(&sid, payload)])
                        .await
                        .unwrap_or(false)
                }
            };
            match refresh_device_tools_json(request_id, &pending, send).await {
                Ok(tools) => {
                    mgr.device_mcp.lock().await.update_tools(tools);
                }
                Err(e) => {
                    tracing::warn!("设备 {} MCP 工具刷新失败: {e}", mgr.device_id);
                }
            }
        });
    }

    pub async fn set_mcp_session_id(&self, session_id: String) {
        *self.mcp_session_id.lock().await = session_id;
    }

    pub async fn refresh_tools_over_json<F, Fut>(
        &self,
        mut send: F,
    ) -> std::result::Result<Vec<xiaozhi_mcp::McpTool>, String>
    where
        F: FnMut(serde_json::Value) -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        let (request_id, pending) = {
            let mcp = self.device_mcp.lock().await;
            (mcp.allocate_request_id(), mcp.pending_hub())
        };
        let tools = refresh_device_tools_json(request_id, &pending, &mut send).await?;
        self.device_mcp.lock().await.update_tools(tools.clone());
        Ok(tools)
    }

    pub async fn list_device_mcp_tools(&self) -> Vec<xiaozhi_mcp::McpTool> {
        self.device_mcp.lock().await.device_tools().to_vec()
    }

    pub async fn is_device_mcp_ready(&self) -> bool {
        self.device_mcp.lock().await.is_ready()
    }

    pub async fn should_schedule_mcp_init(&self) -> bool {
        self.device_mcp.lock().await.should_schedule_init()
    }

    pub async fn try_begin_mcp_init(&self) -> bool {
        self.device_mcp.lock().await.try_begin_init()
    }

    pub async fn mcp_pending_hub(
        &self,
    ) -> Arc<tokio::sync::Mutex<std::collections::HashMap<String, tokio::sync::oneshot::Sender<serde_json::Value>>>>
    {
        self.device_mcp.lock().await.pending_hub()
    }

    pub async fn mcp_handle_inbound(
        &self,
        payload: &serde_json::Value,
    ) -> McpInboundAction {
        self.device_mcp.lock().await.handle_inbound(payload).await
    }

    pub async fn mcp_mark_ready(&self, tools: Vec<xiaozhi_mcp::McpTool>) {
        self.device_mcp.lock().await.mark_ready(tools);
    }

    pub async fn mcp_mark_failed(&self) {
        self.device_mcp.lock().await.mark_failed();
    }

    pub fn register_endpoint(
        &self,
        endpoint_id: impl Into<String>,
        kind: EndpointKind,
        tx: mpsc::UnboundedSender<OutboundFrame>,
    ) {
        self.endpoint_hub.register(
            EndpointRegistration {
                id: endpoint_id.into(),
                kind,
            },
            tx,
        );
    }

    pub fn unregister_endpoint(&self, endpoint_id: &str) -> bool {
        self.endpoint_hub.unregister(endpoint_id)
    }

    pub fn endpoint_count(&self) -> usize {
        self.endpoint_hub.endpoint_count()
    }

    pub fn has_hardware_endpoint(&self) -> bool {
        self.endpoint_hub.has_hardware()
    }

    pub fn has_web_endpoint(&self) -> bool {
        self.endpoint_hub.has_web()
    }

    pub fn set_tts_audio_route(&self, route: TtsAudioRoute) {
        self.endpoint_hub.set_tts_audio_route(route);
    }

    pub fn tts_audio_route(&self) -> TtsAudioRoute {
        self.endpoint_hub.tts_audio_route()
    }

    pub fn list_endpoints(&self) -> Vec<crate::endpoint_hub::EndpointInfo> {
        self.endpoint_hub.list_endpoints()
    }

    pub fn endpoint_snapshot(&self) -> serde_json::Value {
        let endpoints: Vec<serde_json::Value> = self
            .list_endpoints()
            .into_iter()
            .map(|ep| {
                serde_json::json!({
                    "id": ep.id,
                    "kind": ep.kind.as_str(),
                })
            })
            .collect();
        serde_json::json!({
            "device_id": self.device_id,
            "online": !self.endpoint_hub.is_empty(),
            "endpoint_count": self.endpoint_hub.endpoint_count(),
            "has_hardware": self.endpoint_hub.has_hardware(),
            "has_web": self.endpoint_hub.has_web(),
            "tts_audio_route": self.endpoint_hub.tts_audio_route().as_str(),
            "endpoints": endpoints,
        })
    }

    pub async fn debug_runtime_snapshot(&self) -> serde_json::Value {
        use crate::state::ListenPhase;

        let tts_active = self
            .tts_manager()
            .await
            .map(|t| t.is_tts_active())
            .unwrap_or(false);
        let (listen_phase, session_active) = {
            let guard = self.session.lock().await;
            if let Some(session) = guard.as_ref() {
                (session.state().listen_phase, true)
            } else {
                (ListenPhase::Idle, false)
            }
        };
        let listen_phase = match listen_phase {
            ListenPhase::Idle => "idle",
            ListenPhase::Listening => "listening",
            ListenPhase::Processing => "processing",
            ListenPhase::Speaking => "speaking",
        };
        let is_speaking = listen_phase == "speaking" || tts_active;
        let is_listening =
            matches!(listen_phase, "listening" | "processing");
        serde_json::json!({
            "is_mqtt_transport": self.is_mqtt_transport.load(Ordering::SeqCst),
            "hello_inited": self.is_hello_inited(),
            "needs_fresh_hello": self.requires_fresh_hello(),
            "session_active": session_active,
            "listen_phase": listen_phase,
            "tts_active": tts_active,
            "is_speaking": is_speaking,
            "is_listening": is_listening,
            "injected_speech_guard": self.is_injected_speech_guard_active(),
        })
    }

    /// 兼容旧 transport 代码：注册单端点并覆盖同 kind 的旧连接。
    pub async fn set_outbound(&self, tx: mpsc::UnboundedSender<OutboundFrame>) {
        let kind = if self.is_mqtt_transport.load(Ordering::SeqCst) {
            EndpointKind::Hardware
        } else {
            EndpointKind::Web
        };
        let id = match kind {
            EndpointKind::Hardware => "hardware".to_string(),
            EndpointKind::Web => "web-primary".to_string(),
            EndpointKind::Tool => "tool-primary".to_string(),
        };
        self.register_endpoint(id, kind, tx);
    }

    pub fn reset_outbound(&self) {
        self.endpoint_hub.clear();
    }

    pub async fn push_delivery(&self, delivery: &SpeakDelivery) -> Result<bool> {
        if self.endpoint_hub.is_empty() {
            return Ok(false);
        }
        let channel = self.outbound_json_channel();
        for msg in &delivery.messages {
            let data = serde_json::to_vec(msg)?;
            self.signal_log
                .record_server_message(channel, msg)
                .await;
            if self.endpoint_hub.send_command_all(data) == 0 {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub async fn record_inbound_client(&self, channel: &str, msg: &xiaozhi_protocol::messages::ClientMessage) {
        self.signal_log.record_client(channel, msg).await;
    }

    pub async fn record_outbound_server(&self, channel: &str, msg: &ServerMessage) {
        self.signal_log.record_server_message(channel, msg).await;
    }

    pub async fn signal_log_since(&self, after_id: u64) -> Vec<crate::signal_log::SignalEntry> {
        self.signal_log.list_since(after_id).await
    }

    pub async fn clear_signal_log(&self) {
        self.signal_log.clear().await;
    }

    fn outbound_json_channel(&self) -> &'static str {
        if self.has_hardware_endpoint() {
            "mqtt"
        } else {
            "ws"
        }
    }

    fn inbound_audio_channel(&self) -> &'static str {
        if self.is_mqtt_transport.load(Ordering::SeqCst) && self.has_udp_binding.load(Ordering::SeqCst) {
            "udp"
        } else {
            "ws"
        }
    }

    fn outbound_audio_channel(&self) -> &'static str {
        if self.has_hardware_endpoint() {
            "udp"
        } else {
            "ws"
        }
    }

    pub(crate) fn outbound_tx(&self) -> Option<mpsc::UnboundedSender<OutboundFrame>> {
        self.endpoint_hub.primary_sender()
    }

    pub(crate) fn send_outbound_command(&self, data: Vec<u8>) -> bool {
        let channel = self.outbound_json_channel();
        let log = Arc::clone(&self.signal_log);
        let data_for_log = data.clone();
        tokio::spawn(async move {
            log.record_server_json(channel, &data_for_log).await;
        });
        self.endpoint_hub.send_command_all(data) > 0
    }

    pub(crate) fn send_hardware_outbound_command(&self, data: Vec<u8>) -> bool {
        let log = Arc::clone(&self.signal_log);
        let data_for_log = data.clone();
        tokio::spawn(async move {
            log.record_server_json("mqtt", &data_for_log).await;
        });
        self.endpoint_hub.send_command_to("hardware", data)
    }

    pub(crate) fn send_outbound_audio(&self, data: Vec<u8>) -> bool {
        let channel = self.outbound_audio_channel();
        let bytes = data.len();
        let log = Arc::clone(&self.signal_log);
        tokio::spawn(async move {
            log.record_audio("out", channel, bytes).await;
        });
        self.endpoint_hub.send_audio_routed(data) > 0
    }

    pub async fn run_llm_turn(
        &self,
        dialogue: Vec<xiaozhi_llm::ChatMessage>,
        tools: Vec<ToolInfo>,
    ) -> Result<LlmTurnResult> {
        let llm = self
            .llm_manager()
            .await
            .ok_or_else(|| xiaozhi_core::Error::Session("LLM 管理器未初始化".into()))?;
        llm.do_llm_request(dialogue, tools).await
    }

    pub async fn tts_manager(&self) -> Option<Arc<TtsManager>> {
        self.tts_manager.lock().await.clone()
    }

    pub async fn llm_manager(&self) -> Option<Arc<LlmManager>> {
        self.llm_manager.lock().await.clone()
    }

    pub async fn session_media(&self) -> Option<SessionMedia> {
        self.session_media.lock().await.clone()
    }

    pub async fn is_session_aborted(&self) -> bool {
        let guard = self.session_abort.lock().await;
        guard
            .as_ref()
            .map(|abort| abort.load(Ordering::SeqCst))
            .unwrap_or(false)
    }

    pub async fn clear_session_abort(&self) {
        let guard = self.session.lock().await;
        if let Some(session) = guard.as_ref() {
            session.state().clear_abort();
        }
    }

    pub async fn persist_chat_message(self: &Arc<Self>, role: &str, content: &str) {
        let content = content.trim();
        if content.is_empty() {
            return;
        }
        let (device_id, session_id, agent_id) = {
            let guard = self.session.lock().await;
            let Some(session) = guard.as_ref() else {
                return;
            };
            (
                session.state().device_id.clone(),
                session.state().session_id.clone(),
                session
                    .state()
                    .agent_id
                    .parse::<i64>()
                    .ok()
                    .filter(|id| *id > 0),
            )
        };
        self.persist_chat_message_fields(role, content, &device_id, &session_id, agent_id)
            .await;
    }

    /// 调用方已持有 session 锁时使用，避免与 `chat_queue` 死锁
    pub async fn persist_chat_message_fields(
        self: &Arc<Self>,
        role: &str,
        content: &str,
        device_id: &str,
        session_id: &str,
        agent_id: Option<i64>,
    ) {
        let content = content.trim();
        if content.is_empty() {
            return;
        }
        let _ = self
            .history
            .save_message(HistoryMessageInput {
                device_id: device_id.to_string(),
                session_id: session_id.to_string(),
                role: role.to_string(),
                content: content.to_string(),
                agent_id,
                user_id: None,
            })
            .await;
    }

    pub async fn persist_assistant_reply(
        &self,
        full_response: &str,
        dialogue: &mut Vec<xiaozhi_llm::ChatMessage>,
    ) {
        let mut guard = self.session.lock().await;
        let Some(session) = guard.as_mut() else {
            return;
        };
        dialogue.push(xiaozhi_llm::ChatMessage::assistant(full_response));
        session.state_mut().dialogue.push(xiaozhi_llm::ChatMessage::assistant(full_response));
        let agent_id_str = session.state().agent_id.clone();
        let agent_id = agent_id_str
            .parse::<i64>()
            .ok()
            .filter(|id| *id > 0);
        let device_id = session.state().device_id.clone();
        let session_id = session.state().session_id.clone();
        let memory = session.memory_client();
        drop(guard);

        let _ = memory
            .add_message(&agent_id_str, xiaozhi_llm::ChatMessage::assistant(full_response))
            .await;
        let _ = self
            .history
            .save_message(HistoryMessageInput {
                device_id,
                session_id,
                role: "assistant".to_string(),
                content: full_response.to_string(),
                agent_id,
                user_id: None,
            })
            .await;
    }

    pub async fn restore_dialogue_from_history(self: &Arc<Self>, session_id: &str) -> Result<usize> {
        let messages = self.history.fetch_session_dialogue(session_id).await?;
        if messages.is_empty() {
            return Ok(0);
        }
        let mut guard = self.session.lock().await;
        let session = guard
            .as_mut()
            .ok_or_else(|| xiaozhi_core::Error::Session("会话未初始化".into()))?;
        session.state_mut().session_id = session_id.to_string();
        session.state_mut().dialogue.clear();
        for msg in &messages {
            match msg.role.as_str() {
                "user" => {
                    session
                        .state_mut()
                        .dialogue
                        .push(xiaozhi_llm::ChatMessage::user(&msg.content));
                }
                "assistant" => {
                    session
                        .state_mut()
                        .dialogue
                        .push(xiaozhi_llm::ChatMessage::assistant(&msg.content));
                }
                _ => {}
            }
        }
        Ok(messages.len())
    }

    pub async fn execute_mcp_tool(self: &Arc<Self>, name: &str, arguments: serde_json::Value) -> String {
        self.execute_tool(name, arguments).await
    }

    pub async fn execute_mcp_tool_for_llm(
        self: &Arc<Self>,
        name: &str,
        arguments: serde_json::Value,
    ) -> McpToolLlmOutcome {
        let invoke_name = self.resolve_tool_invoke_name(name).await;
        if !self.mcp_manager.is_local_tool_enabled(&invoke_name) {
            return McpToolLlmOutcome {
                text: format!("本地工具已禁用: {invoke_name}"),
                stop_llm: false,
            };
        }

        if let Some(raw) = self
            .invoke_tool_raw(&invoke_name, name, arguments.clone())
            .await
        {
            if let Some(handled) = self.try_play_tool_result_media(&invoke_name, &raw).await {
                return McpToolLlmOutcome {
                    text: if handled {
                        "执行成功".to_string()
                    } else {
                        "执行失败".to_string()
                    },
                    stop_llm: handled,
                };
            }
            return McpToolLlmOutcome {
                text: crate::mcp_tool_media::tool_result_display_text(&raw),
                stop_llm: false,
            };
        }

        McpToolLlmOutcome {
            text: self.execute_tool(name, arguments).await,
            stop_llm: false,
        }
    }

    async fn invoke_tool_raw(
        self: &Arc<Self>,
        invoke_name: &str,
        name: &str,
        arguments: serde_json::Value,
    ) -> Option<serde_json::Value> {
        if self.mcp_manager.has_global_tool(invoke_name) {
            return self
                .mcp_manager
                .call_global_tool_raw(invoke_name, arguments)
                .await
                .ok();
        }
        if name != invoke_name && self.mcp_manager.has_global_tool(name) {
            return self
                .mcp_manager
                .call_global_tool_raw(name, arguments)
                .await
                .ok();
        }

        let session_id = {
            let voice = self.session.lock().await;
            voice
                .as_ref()
                .map(|s| s.state().session_id.clone())
                .filter(|s| !s.is_empty())
        };
        let session_id = session_id.unwrap_or_else(|| {
            self.mcp_session_id
                .try_lock()
                .map(|g| g.clone())
                .unwrap_or_default()
        });
        if session_id.is_empty() {
            return None;
        }

        let (ready, has_tool, request_id, pending) = {
            let mcp = self.device_mcp.lock().await;
            (
                mcp.is_ready(),
                mcp.has_tool(invoke_name) || mcp.has_tool(name),
                mcp.allocate_request_id(),
                mcp.pending_hub(),
            )
        };
        if !ready || !has_tool {
            return None;
        }

        let mgr = Arc::clone(self);
        let send = move |msg: ServerMessage| {
            let mgr = Arc::clone(&mgr);
            async move { mgr.push_messages(&[msg]).await.unwrap_or(false) }
        };
        call_device_tool_raw(
            &session_id,
            invoke_name,
            arguments,
            request_id,
            &pending,
            send,
        )
        .await
        .ok()
    }

    async fn try_play_tool_result_media(
        self: &Arc<Self>,
        tool_name: &str,
        result: &serde_json::Value,
    ) -> Option<bool> {
        let items = result.get("content")?.as_array()?;
        for item in items {
            if let Some(audio) = crate::mcp_tool_media::parse_audio_content(item, tool_name) {
                let played = self
                    .media_player
                    .play_inline_audio(
                        Arc::clone(self),
                        audio.title,
                        audio.data,
                        &audio.audio_format,
                    )
                    .await
                    .is_ok();
                return Some(played);
            }
            if let Some(link) = crate::mcp_tool_media::parse_resource_link(item) {
                let played = if crate::mcp_tool_media::is_direct_audio_url(&link.description) {
                    self.media_player
                        .play_url(Arc::clone(self), link.title, link.description)
                        .await
                        .is_ok()
                } else if !link.uri.is_empty() && self.mcp_manager.has_global_tool(tool_name) {
                    let mut read_args = serde_json::json!({});
                    if crate::mcp_tool_media::is_direct_audio_url(&link.description) {
                        read_args["url"] = serde_json::json!(link.description);
                    }
                    self.media_player
                        .play_mcp_resource(
                            Arc::clone(self),
                            link.title,
                            tool_name.to_string(),
                            link.uri,
                            read_args,
                        )
                        .await
                        .is_ok()
                } else {
                    false
                };
                return Some(played);
            }
        }
        None
    }

    pub fn mcp_manager(&self) -> &Arc<McpManager> {
        &self.mcp_manager
    }

    pub async fn update_session_media_params(&self, params: xiaozhi_protocol::audio::AudioParams) {
        if let Some(media) = self.session_media.lock().await.as_mut() {
            media.audio_params = params.clone();
        }
        if let Some(tts) = self.tts_manager().await {
            tts.set_frame_duration_ms(params.frame_duration.max(1));
        }
    }

    pub async fn update_session_media_tts(&self, tts: Arc<dyn xiaozhi_tts::TtsProvider>) {
        if let Some(media) = self.session_media.lock().await.as_mut() {
            media.tts = tts;
        }
    }

    pub(crate) fn session_id(&self) -> Option<String> {
        if let Ok(guard) = self.session.try_lock() {
            if let Some(session) = guard.as_ref() {
                let sid = session.state().session_id.clone();
                if !sid.is_empty() {
                    return Some(sid);
                }
            }
        }
        self.hello_session_id
            .try_lock()
            .ok()
            .and_then(|g| g.clone())
            .filter(|s| !s.is_empty())
    }

    pub async fn push_messages(&self, messages: &[ServerMessage]) -> Result<bool> {
        self.push_delivery(&SpeakDelivery {
            messages: messages.to_vec(),
            audio_frames: Vec::new(),
        })
        .await
    }

    pub async fn inject_message(
        self: &Arc<Self>,
        text: &str,
        skip_llm: bool,
        auto_listen: bool,
    ) -> Result<(bool, usize)> {
        self.inject_message_with_route(text, skip_llm, auto_listen, None)
            .await
    }

    pub async fn inject_message_with_route(
        self: &Arc<Self>,
        text: &str,
        skip_llm: bool,
        auto_listen: bool,
        audio_route: Option<TtsAudioRoute>,
    ) -> Result<(bool, usize)> {
        let prev_route = self.tts_audio_route();
        if let Some(route) = audio_route {
            self.set_tts_audio_route(route);
        }
        let result = self
            .inject_message_inner(text, skip_llm, auto_listen)
            .await;
        if audio_route.is_some() {
            self.set_tts_audio_route(prev_route);
        }
        result
    }

    pub async fn speak(
        self: &Arc<Self>,
        text: &str,
        audio_route: TtsAudioRoute,
        auto_listen: bool,
    ) -> Result<(bool, usize)> {
        self.inject_message_with_route(text, true, auto_listen, Some(audio_route))
            .await
    }

    async fn inject_message_inner(
        self: &Arc<Self>,
        text: &str,
        skip_llm: bool,
        auto_listen: bool,
    ) -> Result<(bool, usize)> {
        self.cancel_retained_session_cleanup("inject_message").await;
        let is_mqtt = self.is_mqtt_transport();
        let turn_end_policy = injected_speech_tts_turn_end_policy(auto_listen);
        let guard_injected_speech = is_mqtt && !auto_listen;
        let prepare_result = self
            .prepare_speak_path_for_injected_speech(text, auto_listen)
            .await;
        if let Err(e) = prepare_result {
            return Err(e);
        }
        if guard_injected_speech {
            self.begin_injected_speech_guard();
        }
        if let Some(tts) = self.tts_manager().await {
            tts.set_turn_end_policy(turn_end_policy);
        }
        if skip_llm {
            let on_start = self.injected_speech_playback_hook();
            let llm = self
                .llm_manager()
                .await
                .ok_or_else(|| xiaozhi_core::Error::Session("LLM 管理器未初始化".into()))?;
            if let Err(e) = llm.add_text_to_tts_queue_with_on_start(text, on_start).await {
                if guard_injected_speech {
                    self.clear_injected_speech_guard();
                    self.try_resume_pending_listen_start().await;
                }
                return Err(e);
            }
            if auto_listen && !is_mqtt {
                self.on_listen_start(Some("auto")).await;
            }
            return Ok((true, 1));
        }

        self.enqueue_chat_text(text.to_string(), false).await;
        if auto_listen && !is_mqtt {
            self.on_listen_start(Some("auto")).await;
        }
        Ok((true, 0))
    }
    async fn wait_session_closing_done(&self) -> Result<()> {
        const MAX_WAIT_MS: u64 = 3000;
        let mut waited_ms = 0u64;
        while self.session_closing.load(Ordering::SeqCst) {
            if waited_ms >= MAX_WAIT_MS {
                return Err(xiaozhi_core::Error::Session(
                    "会话正在关闭，请稍后重试".into(),
                ));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
            waited_ms += 50;
        }
        Ok(())
    }

    pub async fn init_session(self: Arc<Self>, session_id: String) -> Result<()> {
        self.wait_session_closing_done().await?;
        if self.session.lock().await.is_some() {
            self.clone().spawn_audio_idle_watchdog().await;
            self.media_player.attach_session(Arc::clone(&self)).await;
            return Ok(());
        }
        let mcp_session_id = session_id.clone();
        let uconfig = self.config_provider.get_user_config(&self.device_id).await?;
        if uconfig.llm.config.get("api_key").and_then(|v| v.as_str()).unwrap_or("").is_empty()
            && uconfig.asr.config.get("api_key").and_then(|v| v.as_str()).unwrap_or("").is_empty()
        {
            tracing::warn!(
                device_id = %self.device_id,
                "设备配置缺少 ASR/LLM api_key，可能无法完成对话"
            );
        }
        let state = ClientState::new(self.device_id.clone(), session_id, uconfig.clone());

        let vad_config = serde_json::to_value(&uconfig.vad.config).unwrap_or_default();
        let asr_config = serde_json::to_value(&uconfig.asr.config).unwrap_or_default();
        let llm_config = serde_json::to_value(&uconfig.llm.config).unwrap_or_default();
        let tts_config = serde_json::to_value(&uconfig.tts.config).unwrap_or_default();

        let (vad, asr, llm, tts, pool_handles) = match &self.resource_pools {
            Some(pools) => {
                let (vad, asr, llm, tts, handles) = pools.acquire_session_resources(
                    &uconfig.vad.provider,
                    &vad_config,
                    &uconfig.asr.provider,
                    &asr_config,
                    &uconfig.llm.provider,
                    &llm_config,
                    &uconfig.tts.provider,
                    &tts_config,
                )?;
                (vad, asr, llm, tts, Some(handles))
            }
            None => (
                create_vad(&uconfig.vad.provider, &vad_config)?,
                create_asr(&uconfig.asr.provider, &asr_config)?,
                create_llm(&uconfig.llm.provider, &llm_config)?,
                create_tts(&uconfig.tts.provider, &tts_config)?,
                None,
            ),
        };

        let mem_config = serde_json::to_value(&uconfig.memory.config).unwrap_or_default();
        let memory = create_memory(&uconfig.memory.provider, &mem_config)?;

        let speaker = if uconfig.voice_identify.is_empty() {
            None
        } else {
            create_speaker(&self.app_config.voice_identify)?
        };

        let abort_flag = Arc::clone(&state.abort);
        let session = ChatSession::new(
            state,
            self.app_config.clone(),
            vad,
            asr,
            llm,
            tts,
            memory,
            self.history.clone(),
            self.openclaw.clone(),
            self.mcp_manager.clone(),
            self.knowledge_client.clone(),
            speaker,
            Arc::downgrade(&self),
            pool_handles,
        );

        *self.session.lock().await = Some(session);
        *self.session_abort.lock().await = Some(abort_flag);

        let tts = TtsManager::new(self.device_id.clone(), Arc::downgrade(&self));
        let llm = LlmManager::new(self.device_id.clone(), Arc::downgrade(&self), Arc::clone(&tts));
        *self.tts_manager.lock().await = Some(Arc::clone(&tts));
        *self.llm_manager.lock().await = Some(Arc::clone(&llm));

        {
            let guard = self.session.lock().await;
            if let Some(session) = guard.as_ref() {
                *self.session_media.lock().await = Some(SessionMedia::from_session(
                    session.tts_provider(),
                    session.llm_provider(),
                    session.state(),
                    session.audio_params(),
                ));
            }
        }

        *self.mcp_session_id.lock().await = mcp_session_id;

        self.mark_hello_ready();
        self.init_chat_queue();
        self.clone().spawn_audio_idle_watchdog().await;
        self.media_player.attach_session(Arc::clone(&self)).await;
        Ok(())
    }

    pub async fn handle_audio(self: &Arc<Self>, data: &[u8]) -> Result<()> {
        if self.requires_fresh_hello() {
            return Ok(());
        }
        if self.has_pending_speak_request().await {
            return Ok(());
        }
        if self.session.lock().await.is_none() {
            self.clone().ensure_session().await?;
        }
        let opus = xiaozhi_protocol::unpack_device_audio(data, self.binary_protocol_version());
        let audio_channel = self.inbound_audio_channel();
        let audio_bytes = opus.len();
        self.signal_log
            .record_audio("in", audio_channel, audio_bytes)
            .await;
        let mut session_guard = self.session.lock().await;
        if let Some(session) = session_guard.as_mut() {
            match session.process_audio(opus).await {
                Ok(Some(UplinkPcm::PreListen(pcm))) => {
                    drop(session_guard);
                    let mut pre = self.pre_listen_pcm.lock().await;
                    pre.push(pcm);
                    if pre.len() > MAX_PRE_LISTEN_FRAMES {
                        let drop = pre.len() - MAX_PRE_LISTEN_FRAMES;
                        pre.drain(0..drop);
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::debug!(
                        device_id = %self.device_id,
                        frame_bytes = opus.len(),
                        "跳过无效音频帧: {e}"
                    );
                }
            }
        }
        Ok(())
    }
    pub fn reset_openclaw_mode_on_hello(self: &Arc<Self>) {
        let agent_id = self
            .session
            .try_lock()
            .ok()
            .and_then(|g| g.as_ref().map(|s| s.state().agent_id.clone()))
            .unwrap_or_default();
        self.openclaw.exit_mode(&agent_id, &self.device_id);
        let mgr = Arc::clone(self);
        tokio::spawn(async move {
            mgr.finish_openclaw_warmup("", false).await;
        });
    }

    pub async fn start_openclaw_warmup(self: &Arc<Self>, correlation_id: &str, user_text: &str) {
        let Some(tts) = self.tts_manager().await else {
            return;
        };
        let Some(media) = self.session_media().await else {
            return;
        };
        self.openclaw_warmup
            .lock()
            .await
            .start(
                Arc::clone(self),
                correlation_id.to_string(),
                user_text.to_string(),
                media.session_id,
                media.llm,
                tts,
            )
            .await;
    }

    pub async fn finish_openclaw_warmup(&self, correlation_id: &str, interrupt: bool) {
        if let Some(tts) = self.tts_manager().await {
            self.openclaw_warmup
                .lock()
                .await
                .finish(correlation_id, interrupt, self, &tts)
                .await;
        }
    }

    pub async fn cancel_openclaw_warmup(&self, correlation_id: &str, interrupt: bool) {
        if let Some(tts) = self.tts_manager().await {
            self.openclaw_warmup
                .lock()
                .await
                .cancel(correlation_id, interrupt, self, &tts)
                .await;
        }
    }

    pub async fn has_openclaw_warmup(&self, correlation_id: &str) -> bool {
        self.openclaw_warmup
            .lock()
            .await
            .has_task(correlation_id)
            .await
    }

    pub async fn begin_openclaw_speech_after_warmup(&self, correlation_id: &str) {
        if let Some(tts) = self.tts_manager().await {
            self.openclaw_warmup
                .lock()
                .await
                .begin_openclaw_speech(correlation_id, &tts)
                .await;
        }
    }

    pub async fn openclaw_warmup_speech_started(&self, correlation_id: &str) -> bool {
        self.openclaw_warmup
            .lock()
            .await
            .openclaw_speech_started(correlation_id)
            .await
    }

    /// 对齐 Go `InjectOpenClawResponse`
    pub async fn inject_openclaw_response(
        self: &Arc<Self>,
        event: xiaozhi_openclaw::ResponseDelivery,
    ) -> Result<()> {
        self.cancel_retained_session_cleanup("openclaw_response").await;
        self.clone().ensure_session().await?;
        let mut guard = self.session.lock().await;
        let session = guard
            .as_mut()
            .ok_or_else(|| xiaozhi_core::Error::Session("会话未初始化".into()))?;
        session.inject_openclaw_response(event).await
    }

    /// 对齐 Go `replayOpenClawOfflineMessages`
    pub fn spawn_replay_openclaw_offline_messages(self: &Arc<Self>) {
        let mgr = Arc::clone(self);
        tokio::spawn(async move {
            const MAX_RETRY: usize = 10;
            for _ in 0..MAX_RETRY {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let (delivered, remaining) = mgr
                    .openclaw
                    .replay_offline_messages(&mgr.device_id, |text| {
                        let mgr = Arc::clone(&mgr);
                        async move {
                            if text.trim().is_empty() {
                                return Ok(());
                            }
                            mgr.inject_message(&text, true, false)
                                .await
                                .map(|_| ())
                        }
                    })
                    .await;
                if delivered > 0 {
                    tracing::info!(
                        device_id = %mgr.device_id,
                        delivered,
                        remaining,
                        "OpenClaw 离线消息补发"
                    );
                }
                if remaining == 0 {
                    break;
                }
            }
        });
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// STT 已下发但 LLM/TTS 未启动时，避免设备一直停在聆听界面
    pub async fn ensure_voice_response_after_chat_turn(self: &Arc<Self>) {
        let tts_active = self
            .tts_manager()
            .await
            .map(|t| t.is_tts_active())
            .unwrap_or(false);
        if tts_active {
            return;
        }
        let phase = {
            let guard = self.session.lock().await;
            guard.as_ref().map(|s| s.state().listen_phase)
        };
        if !matches!(
            phase,
            Some(ListenPhase::Processing) | Some(ListenPhase::Listening)
        ) {
            return;
        }
        tracing::warn!(
            device_id = %self.device_id(),
            "对话处理完成但未开始 TTS，播报告错并退出聆听"
        );
        let delivery = {
            let mut guard = self.session.lock().await;
            if let Some(session) = guard.as_mut() {
                session
                    .empty_listen_recovery_session("抱歉，我刚刚没想好怎么说，请再说一遍")
                    .await
                    .ok()
            } else {
                None
            }
        };
        if let Some(delivery) = delivery {
            let _ = self.push_delivery(&delivery).await;
        }
    }

    pub async fn try_resume_pending_listen_start(self: &Arc<Self>) {
        if self.is_injected_speech_guard_active() {
            return;
        }
        if self.is_welcome_playing().await {
            return;
        }
        let pending = {
            let mut guard = self.session.lock().await;
            let Some(session) = guard.as_mut() else {
                return;
            };
            session.take_pending_listen_start_mode()
        };
        if let Some(mode) = pending {
            tracing::info!(
                device_id = %self.device_id(),
                mode = %mode,
                "恢复延后处理的 listen start"
            );
            if self.is_injected_speech_guard_active() {
                tracing::debug!(
                    device_id = %self.device_id(),
                    "主动播报收尾中，跳过恢复 listen start"
                );
                return;
            }
            if self.session.lock().await.is_none() {
                tracing::debug!(
                    device_id = %self.device_id(),
                    "会话已关闭，跳过恢复 listen start"
                );
                return;
            }
            if self.requires_fresh_hello() {
                return;
            }
            self.on_listen_start(Some(&mode)).await;
        }
    }

    pub async fn on_listen_start(self: &Arc<Self>, mode: Option<&str>) {
        if self.requires_fresh_hello() {
            tracing::warn!(device_id = %self.device_id, "会话需重新 hello，忽略 listen start");
            return;
        }
        if !self.check_device_activated().await.unwrap_or(false) {
            return;
        }
        self.cancel_pending_detect_llm().await;
        if let Err(e) = self.clone().ensure_session().await {
            tracing::error!(
                device_id = %self.device_id,
                "listen start 前会话初始化失败: {e:#}"
            );
            return;
        }
        let mode_str = mode.unwrap_or("auto");
        let pre_frames: Vec<Vec<f32>> = self.pre_listen_pcm.lock().await.drain(..).collect();
        tracing::info!(
            device_id = %self.device_id,
            mode = mode_str,
            pre_frames = pre_frames.len(),
            "listen start"
        );
        let mut guard = self.session.lock().await;
        if let Some(session) = guard.as_mut() {
            if let Err(e) = session
                .handle_listen_start(mode, !pre_frames.is_empty())
                .await
            {
                tracing::error!(
                    device_id = %self.device_id,
                    "listen start 启动 ASR 失败: {e:#}"
                );
                return;
            }
            session.flush_prelisten_pcm(pre_frames);
        }
    }

    pub async fn on_listen_stop(self: &Arc<Self>) -> Result<SpeakDelivery> {
        if self.requires_fresh_hello() {
            return Ok(SpeakDelivery::default());
        }
        if self.session.lock().await.is_none() {
            self.clone().ensure_session().await?;
        }
        tracing::info!(device_id = %self.device_id, "listen stop");
        let is_realtime = {
            let guard = self.session.lock().await;
            guard
                .as_ref()
                .map(|s| s.state().is_realtime())
                .unwrap_or(false)
        };
        if is_realtime {
            self.invalidate_listen_start();
        }
        let mut guard = self.session.lock().await;
        if let Some(session) = guard.as_mut() {
            session.handle_listen_stop().await;
        }
        Ok(SpeakDelivery::default())
    }    pub async fn on_listen_stop_and_deliver(self: &Arc<Self>) -> Result<bool> {
        let delivery = self.on_listen_stop().await?;
        self.push_delivery(&delivery).await
    }

    pub async fn on_abort(self: &Arc<Self>, origin: crate::detect::AbortOrigin) {
        self.cancel_pending_detect_llm().await;
        let delivery = {
            let mut guard = self.session.lock().await;
            if let Some(session) = guard.as_mut() {
                session.handle_abort(origin).await
            } else {
                SpeakDelivery::default()
            }
        };
        let _ = self.push_delivery(&delivery).await;
    }
}

#[path = "session_lifecycle.rs"]
mod session_lifecycle;

pub use session_lifecycle::SessionCloseReason;

pub struct ChatManagerRegistry {
    managers: DashMap<String, Arc<ChatManager>>,
}

impl ChatManagerRegistry {
    pub fn new() -> Self {
        Self {
            managers: DashMap::new(),
        }
    }

    pub fn get_or_create<F>(&self, device_id: &str, create_fn: F) -> Result<Arc<ChatManager>>
    where
        F: FnOnce() -> Result<Arc<ChatManager>>,
    {
        if let Some(mgr) = self.managers.get(device_id) {
            return Ok(mgr.clone());
        }
        let mgr = create_fn()?;
        self.managers.insert(device_id.to_string(), mgr.clone());
        Ok(mgr)
    }

    pub fn remove(&self, device_id: &str) {
        self.managers.remove(device_id);
    }

    pub async fn remove_and_shutdown(&self, device_id: &str) {
        if let Some((_, mgr)) = self.managers.remove(device_id) {
            mgr.close_session_with_reason(SessionCloseReason::ManagerShutdown)
                .await;
        }
    }

    pub fn get(&self, device_id: &str) -> Option<Arc<ChatManager>> {
        self.managers.get(device_id).map(|m| m.clone())
    }

    pub fn online_count(&self) -> usize {
        self.managers.len()
    }
}

impl ChatManagerRegistry {
    pub fn insert_manager(&self, device_id: String, mgr: Arc<ChatManager>) {
        self.managers.insert(device_id, mgr);
    }
}

impl Default for ChatManagerRegistry {
    fn default() -> Self {
        Self::new()
    }
}
