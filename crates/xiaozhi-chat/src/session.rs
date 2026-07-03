use std::collections::HashMap;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use tokio::sync::{oneshot, Mutex as AsyncMutex};

use xiaozhi_asr::AsrProvider;
use xiaozhi_config::AppConfig;
use xiaozhi_core::Result;
use xiaozhi_history::HistoryClient;
use xiaozhi_llm::{ChatMessage, LlmProvider, ToolInfo};
use xiaozhi_memory::MemoryProvider;
use xiaozhi_openclaw::{OpenClawManager, ResponseDelivery};
use xiaozhi_rag::KnowledgeClient;
use crate::knowledge::{collect_searchable_kb_ids, default_knowledge_search_threshold};
use xiaozhi_config::user::ProviderConfig;
use xiaozhi_protocol::audio::AudioParams;
use xiaozhi_protocol::messages::ServerMessage;
use xiaozhi_speaker::{IdentifyResult, SpeakerProvider};
use xiaozhi_tts::{create_tts, TtsProvider};
use xiaozhi_vad::VadProvider;

use crate::asr_manager::AsrPipeline;
use crate::detect::{
    is_auto_listen_active, random_greeting, remove_punctuation, resolve_detect_action,
    should_ignore_detect_during_injected_speech,
    should_ignore_listen_start_during_injected_speech, should_ignore_listen_start_during_speak,
    should_ignore_listen_start_during_welcome, should_interrupt_output_on_listen_start,
    AbortOrigin, DetectAction,
};
use crate::tts_turn_policy::TtsTurnEndPolicy;
use crate::llm_types::LlmResponseChunk;
use crate::manager::ChatManager;
use crate::resource_pools::SessionPoolHandles;
use crate::outbound::SpeakDelivery;
use crate::pipeline;
use crate::state::{ClientState, ListenPhase, RealtimeMode};

/// 设备进入聆听 UI 后，麦克风/UDP 真正就绪前的保护窗口（对齐固件 WaitForPlaybackQueueEmpty）
const LISTEN_WARMUP: Duration = Duration::from_millis(1500);

/// 上行 PCM 处理结果
pub enum UplinkPcm {
    /// listen start 前的预缓冲（唤醒词尾音等）
    PreListen(Vec<f32>),
}

/// 对话准备结果：Complete 表示无需 LLM；RunLlm 需在释放 session 锁后调用 LLM
pub enum ChatTurnOutcome {
    Complete(SpeakDelivery),
    RunLlm {
        dialogue: Vec<ChatMessage>,
        tools: Vec<ToolInfo>,
        delivery: SpeakDelivery,
    },
}

pub struct ChatSession {
    state: ClientState,
    app_config: AppConfig,
    vad: Box<dyn VadProvider>,
    asr: Arc<dyn AsrProvider>,
    llm: Arc<dyn LlmProvider>,
    tts: Arc<dyn TtsProvider>,
    memory: Arc<dyn MemoryProvider>,
    history: Arc<HistoryClient>,
    openclaw: Arc<OpenClawManager>,
    mcp_manager: Arc<xiaozhi_mcp::McpManager>,
    knowledge_client: Arc<KnowledgeClient>,
    manager: Weak<ChatManager>,
    _pool_handles: Option<SessionPoolHandles>,
    speaker: Option<Arc<dyn SpeakerProvider>>,
    speaker_streaming: bool,
    default_tts: ProviderConfig,
    active_tts_config_id: Option<String>,
    audio_params: AudioParams,
    opus_decoder: Option<crate::opus_codec::OpusStreamDecoder>,
    asr_pipeline: AsrPipeline,
    openclaw_stream_started: HashMap<String, bool>,
    /// 对齐 Go `welcomePlaybackDoneCh`：realtime listen 可等待欢迎语自然结束
    welcome_playback_done: AsyncMutex<Option<oneshot::Sender<bool>>>,
    /// 欢迎语/注入播报期间被忽略的 listen start，播完后再补开 ASR
    pending_listen_start_mode: Option<String>,
    /// 设备进入聆听 UI 后，麦克风/UDP 真正就绪前的保护窗口
    listen_warmup_until: Option<Instant>,
}

impl ChatSession {
    pub fn new(
        state: ClientState,
        app_config: AppConfig,
        vad: Box<dyn VadProvider>,
        asr: Arc<dyn AsrProvider>,
        llm: Arc<dyn LlmProvider>,
        tts: Arc<dyn TtsProvider>,
        memory: Arc<dyn MemoryProvider>,
        history: Arc<HistoryClient>,
        openclaw: Arc<OpenClawManager>,
        mcp_manager: Arc<xiaozhi_mcp::McpManager>,
        knowledge_client: Arc<KnowledgeClient>,
        speaker: Option<Arc<dyn SpeakerProvider>>,
        manager: Weak<ChatManager>,
        pool_handles: Option<SessionPoolHandles>,
    ) -> Self {
        let default_tts = state.device_config.tts.clone();
        let silence_ms = app_config.chat.chat_max_silence_duration;
        let asr_auto_end = state
            .device_config
            .asr
            .config
            .get("auto_end")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        Self {
            state,
            app_config,
            vad,
            asr,
            llm,
            tts,
            memory,
            history,
            openclaw,
            mcp_manager,
            knowledge_client,
            manager,
            _pool_handles: pool_handles,
            speaker,
            speaker_streaming: false,
            default_tts,
            active_tts_config_id: None,
            audio_params: AudioParams::default(),
            opus_decoder: None,
            asr_pipeline: AsrPipeline::new(silence_ms, asr_auto_end),
            openclaw_stream_started: HashMap::new(),
            welcome_playback_done: AsyncMutex::new(None),
            pending_listen_start_mode: None,
            listen_warmup_until: None,
        }
    }

    fn defer_listen_start(&mut self, mode: &str) {
        self.pending_listen_start_mode = Some(mode.to_string());
    }

    pub fn take_pending_listen_start_mode(&mut self) -> Option<String> {
        self.pending_listen_start_mode.take()
    }

    pub fn begin_listen_warmup(&mut self) {
        self.listen_warmup_until = Some(Instant::now() + LISTEN_WARMUP);
    }

    pub fn extend_listen_warmup(&mut self, extra: Duration) {
        let until = Instant::now() + extra;
        match self.listen_warmup_until {
            Some(current) if current > until => {}
            _ => self.listen_warmup_until = Some(until),
        }
    }

    pub fn is_in_listen_warmup(&self) -> bool {
        self.listen_warmup_until
            .is_some_and(|until| Instant::now() < until)
    }

    pub fn set_audio_params(&mut self, params: AudioParams) {
        self.asr_pipeline.frame_duration_ms = params.frame_duration.max(1);
        self.audio_params = params.clone();
        if let Some(manager) = self.manager.upgrade() {
            let manager = manager.clone();
            let params = params.clone();
            tokio::spawn(async move {
                manager.update_session_media_params(params).await;
            });
        }
    }

    pub fn audio_params(&self) -> AudioParams {
        self.audio_params.clone()
    }

    pub fn tts_provider(&self) -> Arc<dyn TtsProvider> {
        Arc::clone(&self.tts)
    }

    pub fn llm_provider(&self) -> Arc<dyn LlmProvider> {
        Arc::clone(&self.llm)
    }

    pub fn memory_client(&self) -> Arc<dyn MemoryProvider> {
        Arc::clone(&self.memory)
    }

    pub fn history_client(&self) -> Arc<HistoryClient> {
        Arc::clone(&self.history)
    }

    pub async fn execute_mcp_tool_public(&self, name: &str, arguments: serde_json::Value) -> String {
        self.execute_mcp_tool(name, arguments).await
    }

    fn ensure_opus_decoder(&mut self) -> Result<()> {
        if self.opus_decoder.is_none() {
            self.opus_decoder = Some(crate::opus_codec::OpusStreamDecoder::new(
                &self.audio_params,
            )?);
        }
        Ok(())
    }

    pub fn state_mut(&mut self) -> &mut ClientState {
        &mut self.state
    }

    pub async fn handle_listen_start(
        &mut self,
        mode: Option<&str>,
        has_prelisten_audio: bool,
    ) -> Result<()> {
        let manager = self
            .manager
            .upgrade()
            .ok_or_else(|| xiaozhi_core::Error::Session("ChatManager 已释放".into()))?;
        let mode_str = mode.unwrap_or("auto");

        if should_ignore_listen_start_during_welcome(mode_str, self.state.welcome_playing) {
            tracing::info!(
                device_id = %self.state.device_id,
                mode = mode_str,
                "欢迎语播放中，忽略 listen start"
            );
            if let Some(tts) = manager.tts_manager().await {
                tts.nudge_tts_speaking_signal();
            }
            return Ok(());
        }

        if should_ignore_listen_start_during_injected_speech(
            mode_str,
            manager.is_injected_speech_guard_active(),
        ) {
            tracing::info!(
                device_id = %self.state.device_id,
                mode = mode_str,
                "主动注入播报中，忽略 listen start（已记录，播完后自动恢复）"
            );
            self.defer_listen_start(mode_str);
            return Ok(());
        }

        let tts_active = manager
            .tts_manager()
            .await
            .map(|t| t.is_tts_active())
            .unwrap_or(false);
        let speaking = self.state.listen_phase == ListenPhase::Speaking;
        if should_ignore_listen_start_during_speak(
            mode_str,
            tts_active,
            speaking,
            has_prelisten_audio,
        ) {
            tracing::info!(
                device_id = %self.state.device_id,
                mode = mode_str,
                tts_active,
                speaking,
                "LLM 播报中，忽略无音频 listen start（已记录，播完后自动恢复）"
            );
            self.defer_listen_start(mode_str);
            return Ok(());
        }

        if mode_str == "realtime" && manager.is_realtime_listen_active() {
            tracing::debug!(
                device_id = %self.state.device_id,
                "realtime listen 会话仍活跃，忽略重复 listen start"
            );
            return Ok(());
        }

        // 对齐 Go HandleListenStart：有输出在播时才 StopSpeaking
        let need_stop = tts_active || speaking;

        if mode_str != "realtime" {
            if need_stop {
                let stop = self
                    .stop_speaking_with_reason(
                        manager.as_ref(),
                        true,
                        true,
                        "HandleListenStart",
                        false,
                    )
                    .await;
                let _ = manager.push_delivery(&stop).await;
            }
        } else if should_interrupt_output_on_listen_start(mode_str, self.state.welcome_playing) && need_stop
        {
            let stop = self
                .stop_speaking_with_reason(
                    manager.as_ref(),
                    true,
                    true,
                    "HandleListenStart",
                    false,
                )
                .await;
            let _ = manager.push_delivery(&stop).await;
        }

        let start_seq = manager.begin_listen_start(mode_str);
        tracing::info!(
            device_id = %self.state.device_id,
            mode = mode_str,
            start_seq,
            "listen start seq 已分配"
        );
        if !manager.is_current_listen_start(start_seq) {
            tracing::debug!(
                device_id = %self.state.device_id,
                start_seq,
                "listen start seq 已失效，跳过"
            );
            return Ok(());
        }

        self.state.listen_mode = mode_str.to_string();
        self.state.clear_abort();
        self.vad.reset();
        self.opus_decoder = None;
        if let Err(e) = self.ensure_opus_decoder() {
            tracing::warn!(
                device_id = %self.state.device_id,
                "创建 Opus 解码器失败: {e}"
            );
        }
        self.speaker_streaming = false;
        if let Some(speaker) = &self.speaker {
            let _ = speaker.reset().await;
        }
        if mode_str == "manual" {
            self.asr_pipeline.voice.have_voice = true;
        }

        if !manager.is_current_listen_start(start_seq) {
            return Ok(());
        }

        if manager.is_mqtt_transport() {
            self.begin_listen_warmup();
        }

        // 对齐 Go OnListenStart：先同步启动 ASR 流式连接，再进入 Listening，避免音频早于 channel 就绪被丢弃
        let first_outcome = match self
            .asr_pipeline
            .restart_asr_recognition(Arc::clone(&manager), Arc::clone(&self.asr))
            .await
        {
            Ok(rx) => rx,
            Err(e) => {
                tracing::error!(
                    device_id = %self.state.device_id,
                    "asr 流式识别失败: {e:#}"
                );
                if manager.is_current_listen_start(start_seq) {
                    self.state.listen_phase = ListenPhase::Idle;
                }
                Arc::clone(&manager)
                    .close_session_with_reason(crate::SessionCloseReason::FatalError)
                    .await;
                return Err(e);
            }
        };

        if !manager.is_current_listen_start(start_seq) {
            self.asr_pipeline.abort_turn();
            return Ok(());
        }

        let should_start_audio_idle = self.state.listen_phase != ListenPhase::Listening;
        self.state.listen_phase = ListenPhase::Listening;
        if should_start_audio_idle {
            self.start_audio_idle_window();
        }
        self.asr_pipeline.start_recognition_loop(
            Arc::clone(&manager),
            Arc::clone(&self.asr),
            start_seq,
            first_outcome,
        );
        Ok(())
    }

    /// 对齐 Go `StopSpeakingWithReason`
    pub async fn stop_speaking_with_reason(
        &mut self,
        manager: &ChatManager,
        cancel_session: bool,
        send_tts_stop: bool,
        reason: &str,
        preserve_welcome_playing: bool,
    ) -> SpeakDelivery {
        tracing::info!(
            device_id = %self.state.device_id,
            reason,
            cancel_session,
            send_tts_stop,
            preserve_welcome_playing,
            "stop speaking"
        );
        // 对齐 Go stopSpeaking：仅取消 session/afterAsr ctx，不置 Abort；Abort 仅用于 HandleAbortMessage。
        self.complete_welcome_playback_wait(false).await;
        // 对齐 Go stopSpeaking：仅清 IsWelcomePlaying，保留 IsWelcomeSpeaking
        if !preserve_welcome_playing {
            self.state.welcome_playing = false;
        }

        if cancel_session {
            manager.invalidate_listen_start();
            self.asr_pipeline.abort_loop();
            self.asr_pipeline.on_voice_silence();
            self.state.listen_phase = ListenPhase::Idle;
        }

        if let Some(tts) = manager.tts_manager().await {
            tts.interrupt_and_stop_sync(send_tts_stop, reason).await;
        }
        manager.media_player().suspend().await;

        SpeakDelivery::default()
    }

    /// 对齐 Go `ChatSession.ResetToSilentState`
    pub async fn reset_to_silent_state(&mut self, manager: &ChatManager) {
        manager.cancel_pending_detect_llm().await;
        self.state.clear_abort();
        self.state.welcome_speaking = false;
        let _ = self
            .stop_speaking_with_reason(
                manager,
                true,
                true,
                "ChatSession.ResetToSilentState",
                true,
            )
            .await;
        self.asr_pipeline.abort_loop();
        self.asr_pipeline.on_voice_silence();
        self.vad.reset();
        self.asr_pipeline.voice.reset();
        self.asr_pipeline.vad.reset_all();
        self.asr_pipeline.asr_audio_buffer.clear();
        self.asr_pipeline.reset_received_text();
        self.state.reset_audio_idle_window();
        self.state.clear_audio_idle_timeout_pending();
        self.state.listen_phase = ListenPhase::Idle;
        self.state.welcome_playing = false;
        self.pending_listen_start_mode = None;
        self.listen_warmup_until = None;
        manager.invalidate_listen_start();
        self.state.clear_abort();
    }

    /// 对齐 Go `beginWelcomePlaybackWait`
    pub async fn begin_welcome_playback_wait(&mut self) {
        let mut guard = self.welcome_playback_done.lock().await;
        if let Some(stale) = guard.take() {
            let _ = stale.send(false);
        }
        let (tx, _rx) = oneshot::channel();
        *guard = Some(tx);
    }

    /// 对齐 Go `completeWelcomePlaybackWait`
    pub async fn complete_welcome_playback_wait(&mut self, natural: bool) {
        let mut guard = self.welcome_playback_done.lock().await;
        if let Some(tx) = guard.take() {
            let _ = tx.send(natural);
        }
    }

    pub fn start_audio_idle_window(&mut self) {
        if !self.state.uses_audio_idle_clock() {
            return;
        }
        self.state.start_audio_idle_window();
        self.asr_pipeline.voice.client_voice_stop = false;
        tracing::info!(
            device_id = %self.state.device_id,
            mode = %self.state.listen_mode,
            "音频空闲计时已启动"
        );
    }

    pub async fn handle_listen_detect(&mut self, text: Option<&str>) -> Result<SpeakDelivery> {
        let manager = self
            .manager
            .upgrade()
            .ok_or_else(|| xiaozhi_core::Error::Session("ChatManager 已释放".into()))?;
        manager.cancel_pending_detect_llm().await;

        let raw = text.unwrap_or("").trim();
        if raw.is_empty() {
            return Ok(SpeakDelivery::default());
        }

        let normalized = remove_punctuation(raw);
        let auto_listen_active = is_auto_listen_active(&self.state);
        let action = resolve_detect_action(
            &normalized,
            &self.app_config,
            self.state.welcome_speaking,
            auto_listen_active,
            self.state.welcome_playing,
        );

        tracing::debug!(
            device_id = %self.state.device_id,
            text = %normalized,
            ?action,
            auto_listen_active,
            welcome_speaking = self.state.welcome_speaking,
            welcome_playing = self.state.welcome_playing,
            "detect recv"
        );

        if action == DetectAction::Silent {
            return Ok(SpeakDelivery::default());
        }

        if should_ignore_detect_during_injected_speech(manager.is_injected_speech_guard_active()) {
            tracing::info!(
                device_id = %self.state.device_id,
                text = %normalized,
                ?action,
                "主动注入播报中，忽略 detect"
            );
            return Ok(SpeakDelivery::default());
        }

        // 仅在有残留输出时 StopSpeaking；空闲唤醒时勿 interrupt sender / 下发 tts_stop，避免与紧随其后的欢迎语 tts start 竞态
        let tts_active = manager
            .tts_manager()
            .await
            .map(|t| t.is_tts_active())
            .unwrap_or(false);
        let speaking = self.state.listen_phase == ListenPhase::Speaking;
        if tts_active || speaking {
            let stop_reason = format!("HandleListenDetect action={action:?} text={normalized}");
            let stop = self
                .stop_speaking_with_reason(manager.as_ref(), true, true, &stop_reason, false)
                .await;
            let _ = manager.push_delivery(&stop).await;
        }

        match action {
            DetectAction::Welcome => self.handle_welcome().await,
            DetectAction::Llm => {
                manager.schedule_detect_llm(normalized).await;
                Ok(SpeakDelivery::default())
            }
            DetectAction::Silent => Ok(SpeakDelivery::default()),
        }
    }

    pub async fn handle_welcome(&mut self) -> Result<SpeakDelivery> {
        let greeting = random_greeting(&self.app_config);
        // 对齐 Go HandleWelcome：stopSpeaking 会取消 afterAsr ctx，随后 Get 新 ctx；
        // Rust 等价于清除 stop_speaking 留下的 abort，否则 handleTts 立即退出 frames=0。
        self.state.clear_abort();
        self.state.welcome_speaking = true;
        self.state.welcome_playing = true;
        self.begin_welcome_playback_wait().await;

        let manager = self
            .manager
            .upgrade()
            .ok_or_else(|| xiaozhi_core::Error::Session("ChatManager 已释放".into()))?;
        if let Some(tts) = manager.tts_manager().await {
            tts.set_turn_end_policy(TtsTurnEndPolicy::None);
        }
        let llm = manager
            .llm_manager()
            .await
            .ok_or_else(|| xiaozhi_core::Error::Session("LLM 管理器未初始化".into()))?;
        let device_id = self.state.device_id.clone();
        tokio::spawn(async move {
            if let Err(e) = llm.handle_welcome_tts(&greeting).await {
                tracing::error!(device_id = %device_id, "HandleWelcome TTS 失败: {e:#}");
                manager.complete_welcome_playback_wait(false).await;
            }
        });
        Ok(SpeakDelivery::default())
    }

    pub async fn handle_not_activated(&mut self) -> Result<SpeakDelivery> {
        let manager = self
            .manager
            .upgrade()
            .ok_or_else(|| xiaozhi_core::Error::Session("ChatManager 已释放".into()))?;
        let (code, _challenge, _message, _timeout) = manager
            .config_provider()
            .get_activation_info(&self.state.device_id, "client_id")
            .await?;
        let text = format!("请在后台添加设备，激活码: {code}");
        let mut delivery = SpeakDelivery::default();
        self.speak_text(&text, &mut delivery.messages).await?;
        Ok(delivery)
    }

    pub fn flush_prelisten_pcm(&mut self, frames: Vec<Vec<f32>>) {
        let listen_mode = self.state.listen_mode.clone();
        let phase = self.state.listen_phase;
        let suppress_end = self.is_in_listen_warmup();
        for pcm_f32 in frames {
            if pcm_f32.is_empty() {
                continue;
            }
            let pcm_i16: Vec<i16> = pcm_f32
                .iter()
                .map(|&s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
                .collect();
            self.asr_pipeline.process_pcm_frame(
                &pcm_f32,
                &pcm_i16,
                &listen_mode,
                phase,
                &mut self.vad,
                suppress_end,
            );
        }
    }

    async fn process_listening_frame(&mut self, pcm_f32: &[f32], pcm_i16: &[i16]) {
        if self.asr_pipeline.voice.client_voice_stop {
            return;
        }
        let listen_mode = self.state.listen_mode.clone();
        let listen_phase = self.state.listen_phase;
        let suppress_end = self.is_in_listen_warmup();
        self.asr_pipeline.process_pcm_frame(
            pcm_f32,
            pcm_i16,
            &listen_mode,
            listen_phase,
            &mut self.vad,
            suppress_end,
        );
        if self.asr_pipeline.voice.have_voice {
            if let Some(speaker) = &self.speaker {
                if !self.speaker_streaming {
                    if speaker
                        .start_streaming(self.audio_params.sample_rate, &self.state.agent_id)
                        .await
                        .is_ok()
                    {
                        self.speaker_streaming = true;
                    }
                }
                if self.speaker_streaming {
                    let _ = speaker.send_audio_chunk(pcm_f32).await;
                }
            }
        }
    }

    pub async fn handle_hello(&mut self) -> ServerMessage {
        ServerMessage::hello(self.state.session_id.clone(), self.audio_params.clone())
    }

    pub fn state(&self) -> &ClientState {
        &self.state
    }

    pub async fn handle_listen_stop(&mut self) {
        self.asr_pipeline.on_manual_stop();
    }

    /// 处理上行音频；返回 PCM 供预卷缓冲
    pub async fn process_audio(&mut self, opus_data: &[u8]) -> Result<Option<UplinkPcm>> {
        let realtime_mode = RealtimeMode::from(self.app_config.chat.realtime_mode);
        let phase = self.state.listen_phase;

        self.ensure_opus_decoder()?;
        let pcm = {
            let dec = self
                .opus_decoder
                .as_mut()
                .ok_or_else(|| xiaozhi_core::Error::Audio("Opus 解码器未就绪".into()))?;
            dec.decode_to_pcm_i16_le(opus_data)?
        };
        if pcm.is_empty() {
            return Ok(None);
        }
        let pcm_i16: Vec<i16> = pcm
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect();
        let pcm_f32 = pipeline::pcm_i16_to_f32(&pcm_i16);

        if phase == ListenPhase::Idle {
            return Ok(Some(UplinkPcm::PreListen(pcm_f32)));
        }

        if phase != ListenPhase::Listening && phase != ListenPhase::Speaking {
            return Ok(None);
        }

        if self.state.listen_phase == ListenPhase::Speaking {
            if self.vad.is_vad(&pcm_i16)? {
                match realtime_mode {
                    RealtimeMode::VadInterrupt | RealtimeMode::AsrInterrupt => {
                        self.state.trigger_abort();
                    }
                    RealtimeMode::SpeakerInterrupt if self.speaker.is_some() => {
                        self.state.trigger_abort();
                    }
                    _ => {}
                }
            }
            return Ok(None);
        }

        if self.state.listen_phase != ListenPhase::Listening {
            return Ok(None);
        }

        self.process_listening_frame(&pcm_f32, &pcm_i16).await;
        Ok(None)
    }

    /// 流式 ASR 最终结果 → LLM/TTS（对齐 Go `AddAsrResultToQueue` 主路径）
    pub async fn handle_asr_result(&mut self, text: String) -> Result<SpeakDelivery> {
        match self.prepare_chat_turn(text, true).await? {
            ChatTurnOutcome::Complete(delivery) => Ok(delivery),
            ChatTurnOutcome::RunLlm {
                dialogue,
                tools,
                mut delivery,
            } => {
                let manager = self
                    .manager
                    .upgrade()
                    .ok_or_else(|| xiaozhi_core::Error::Session("ChatManager 已释放".into()))?;
                let turn = manager.run_llm_turn(dialogue, tools).await?;
                delivery.messages.extend(turn.delivery.messages);
                Ok(delivery)
            }
        }
    }

    /// 准备对话轮次；若需 LLM，调用方必须在释放 session 锁后再 `run_llm_turn`
    pub async fn prepare_chat_turn(
        &mut self,
        text: String,
        send_stt: bool,
    ) -> Result<ChatTurnOutcome> {
        let trimmed = text.trim().to_string();
        if !trimmed.is_empty() {
            if let Some(manager) = self.manager.upgrade() {
                let agent_id = self
                    .state
                    .agent_id
                    .parse::<i64>()
                    .ok()
                    .filter(|id| *id > 0);
                manager
                    .persist_chat_message_fields(
                        "user",
                        &trimmed,
                        &self.state.device_id,
                        &self.state.session_id,
                        agent_id,
                    )
                    .await;
            }
        }

        let mut delivery = SpeakDelivery::default();

        let speaker_result = if let Some(speaker) = &self.speaker {
            self.speaker_streaming = false;
            speaker.finish_and_identify().await.ok().flatten()
        } else {
            None
        };

        if self.should_block_unidentified_speaker(&speaker_result) {
            let hint = "未能识别您的声纹，请重试或联系管理员注册声纹样本";
            delivery.messages.push(ServerMessage::text(hint));
            self.speak_text_for_delivery(hint, &mut delivery).await?;
            return Ok(ChatTurnOutcome::Complete(delivery));
        }

        self.apply_speaker_tts(&speaker_result).await;

        if send_stt {
            delivery.messages.push(ServerMessage::stt(
                text.clone(),
                Some(self.state.session_id.clone()),
            ));
        }

        if self
            .try_route_openclaw(&text, &mut delivery)
            .await?
        {
            return Ok(ChatTurnOutcome::Complete(delivery));
        }

        let (dialogue, tools) = self
            .build_llm_request(&text, speaker_result.as_ref())
            .await?;
        Ok(ChatTurnOutcome::RunLlm {
            dialogue,
            tools,
            delivery,
        })
    }

    pub async fn process_chat_text(
        &mut self,
        text: String,
        send_stt: bool,
    ) -> Result<SpeakDelivery> {
        match self.prepare_chat_turn(text, send_stt).await? {
            ChatTurnOutcome::Complete(delivery) => Ok(delivery),
            ChatTurnOutcome::RunLlm {
                dialogue,
                tools,
                mut delivery,
            } => {
                let manager = self
                    .manager
                    .upgrade()
                    .ok_or_else(|| xiaozhi_core::Error::Session("ChatManager 已释放".into()))?;
                let turn = manager.run_llm_turn(dialogue, tools).await?;
                delivery.messages.extend(turn.delivery.messages);
                Ok(delivery)
            }
        }
    }

    #[allow(dead_code)]
    pub async fn finalize_speech(&mut self, pcm_chunks: Vec<Vec<f32>>) -> Result<SpeakDelivery> {
        let mut delivery = SpeakDelivery::default();

        let pcm: Vec<f32> = pcm_chunks.into_iter().flatten().collect();
        if pcm.is_empty() {
            tracing::warn!(
                device_id = %self.state.device_id,
                "listen stop 无有效音频数据"
            );
            self.state.listen_phase = ListenPhase::Idle;
            return Ok(Self::empty_listen_recovery(
                &self.state.session_id,
                "没听清楚，请再说一遍",
            ));
        }

        let speaker_result = if let Some(speaker) = &self.speaker {
            self.speaker_streaming = false;
            speaker.finish_and_identify().await.ok().flatten()
        } else {
            None
        };

        if self.should_block_unidentified_speaker(&speaker_result) {
            let hint = "未能识别您的声纹，请重试或联系管理员注册声纹样本";
            delivery.messages.push(ServerMessage::text(hint));
            self.speak_text_for_delivery(hint, &mut delivery).await?;
            return Ok(delivery);
        }

        self.apply_speaker_tts(&speaker_result).await;

        let text = match self.asr.process(&pcm).await {
            Ok(t) => t,
            Err(e) => {
                tracing::error!(
                    device_id = %self.state.device_id,
                    "ASR 识别失败: {e}"
                );
                self.state.listen_phase = ListenPhase::Idle;
                return Ok(Self::empty_listen_recovery(
                    &self.state.session_id,
                    "语音识别失败，请重试",
                ));
            }
        };
        if text.trim().is_empty() {
            tracing::warn!(
                device_id = %self.state.device_id,
                pcm_samples = pcm.len(),
                "ASR 结果为空"
            );
            self.state.listen_phase = ListenPhase::Idle;
            return Ok(Self::empty_listen_recovery(
                &self.state.session_id,
                "没听清楚，请再说一遍",
            ));
        }

        delivery.messages.push(ServerMessage::stt(
            text.clone(),
            Some(self.state.session_id.clone()),
        ));

        if self.try_route_openclaw(&text, &mut delivery).await? {
            return Ok(delivery);
        }

        let (dialogue, tools) = self
            .build_llm_request(&text, speaker_result.as_ref())
            .await?;
        let manager = self
            .manager
            .upgrade()
            .ok_or_else(|| xiaozhi_core::Error::Session("ChatManager 已释放".into()))?;
        let turn = manager.run_llm_turn(dialogue, tools).await?;
        delivery.messages.extend(turn.delivery.messages);
        Ok(delivery)
    }

    pub async fn inject_openclaw_response(&mut self, event: ResponseDelivery) -> Result<()> {
        let correlation_id = event.correlation_id.trim().to_string();
        if correlation_id.is_empty() {
            if event.text.trim().is_empty() {
                return Ok(());
            }
            return self
                .speak_text_for_delivery(&event.text, &mut SpeakDelivery::default())
                .await;
        }

        if event.text.is_empty() && !event.is_end {
            return Ok(());
        }

        let manager = self
            .manager
            .upgrade()
            .ok_or_else(|| xiaozhi_core::Error::Session("ChatManager 已释放".into()))?;
        let tts = manager
            .tts_manager()
            .await
            .ok_or_else(|| xiaozhi_core::Error::Session("TTS 管理器未初始化".into()))?;

        let mut is_start = event.is_start;
        if is_start {
            if manager.has_openclaw_warmup(&correlation_id).await {
                if !event.text.trim().is_empty() {
                    manager
                        .cancel_openclaw_warmup(&correlation_id, false)
                        .await;
                    manager
                        .begin_openclaw_speech_after_warmup(&correlation_id)
                        .await;
                } else {
                    is_start = false;
                }
            }
        } else if event.is_end {
            manager
                .cancel_openclaw_warmup(&correlation_id, false)
                .await;
        }

        let stream = self
            .openclaw_stream_started
            .entry(correlation_id.clone())
            .or_insert(false);
        if !*stream && !is_start {
            is_start = true;
        }
        if is_start && !*stream {
            let warmup_already_started = manager
                .openclaw_warmup_speech_started(&correlation_id)
                .await;
            if !warmup_already_started {
                tts.enqueue_tts_start("InjectOpenClawResponse").await;
            }
            *stream = true;
        } else if is_start {
            is_start = false;
        }

        if !event.text.is_empty() {
            tts.handle_text_response(
                LlmResponseChunk {
                    text: event.text,
                    is_start,
                    is_end: event.is_end,
                },
                None,
                None,
            )
            .await?;
        }

        if event.is_end {
            tts.finish_tts_turn("InjectOpenClawResponse").await;
            self.openclaw_stream_started.remove(&correlation_id);
            manager
                .finish_openclaw_warmup(&correlation_id, false)
                .await;
        }
        Ok(())
    }

    pub async fn inject_message(&mut self, text: &str, skip_llm: bool) -> Result<SpeakDelivery> {
        let mut delivery = SpeakDelivery::default();
        if text.trim().is_empty() {
            return Ok(delivery);
        }

        if skip_llm {
            delivery.messages.push(ServerMessage::llm(
                text,
                Some(self.state.session_id.clone()),
            ));
            let audio = self.speak_text(text, &mut delivery.messages).await?;
            delivery.audio_frames.extend(audio);
        } else {
            match self.prepare_chat_turn(text.to_string(), false).await? {
                ChatTurnOutcome::Complete(reply) => {
                    delivery.messages.extend(reply.messages);
                    delivery.audio_frames.extend(reply.audio_frames);
                }
                ChatTurnOutcome::RunLlm {
                    dialogue,
                    tools,
                    delivery: mut llm_delivery,
                } => {
                    let manager = self
                        .manager
                        .upgrade()
                        .ok_or_else(|| xiaozhi_core::Error::Session("ChatManager 已释放".into()))?;
                    let turn = manager.run_llm_turn(dialogue, tools).await?;
                    llm_delivery.messages.extend(turn.delivery.messages);
                    delivery.messages.extend(llm_delivery.messages);
                    delivery.audio_frames.extend(llm_delivery.audio_frames);
                }
            }
        }
        Ok(delivery)
    }

    async fn build_llm_request(
        &mut self,
        text: &str,
        speaker_result: Option<&IdentifyResult>,
    ) -> Result<(Vec<ChatMessage>, Vec<ToolInfo>)> {
        self.state.dialogue.push(ChatMessage::user(text));
        let _ = self
            .memory
            .add_message(&self.state.agent_id, ChatMessage::user(text))
            .await;

        let mut system_prompt = if self.state.device_config.system_prompt.is_empty() {
            self.app_config.system_prompt.clone()
        } else {
            self.state.device_config.system_prompt.clone()
        };

        if let Some(result) = speaker_result {
            if result.identified {
                if let Some(group) = self
                    .state
                    .device_config
                    .voice_identify
                    .get(&result.speaker_name)
                {
                    if !group.prompt.is_empty() {
                        system_prompt
                            .push_str(&format!("\n基于声纹识别到对话人信息:\n{}", group.prompt));
                    }
                }
            }
        }

        let mut dialogue = vec![ChatMessage::system(&system_prompt)];
        dialogue.extend(self.state.dialogue.clone());

        let memory_context = self
            .memory
            .get_context(&self.state.agent_id, 2000)
            .await
            .unwrap_or_default();
        if !memory_context.is_empty() {
            dialogue.insert(1, ChatMessage::system(format!("记忆上下文:\n{memory_context}")));
        }

        let kb_ids = collect_searchable_kb_ids(&self.state.device_config.knowledge_bases, &[]);
        if !kb_ids.is_empty() {
            let threshold =
                default_knowledge_search_threshold(&self.state.device_config.knowledge_bases, &kb_ids);
            if let Ok(hits) = self
                .knowledge_client
                .search(&kb_ids, text, 3, threshold)
                .await
            {
                if !hits.is_empty() {
                    let ctx = hits
                        .iter()
                        .map(|h| format!("[{}] {}", h.title, h.content))
                        .collect::<Vec<_>>()
                        .join("\n\n");
                    dialogue.insert(
                        1,
                        ChatMessage::system(format!("相关知识库内容:\n{ctx}")),
                    );
                }
            }
        }

        let knowledge_bases = self.state.device_config.knowledge_bases.clone();
        let tools: Vec<ToolInfo> = if let Some(mgr) = self.manager.upgrade() {
            mgr.collect_llm_tools_for_bases(&knowledge_bases).await
        } else {
            self.mcp_manager
                .list_local_tools()
                .into_iter()
                .map(|t| ToolInfo {
                    name: t.name,
                    description: t.description,
                    parameters: t.input_schema,
                })
                .collect()
        };

        Ok((dialogue, tools))
    }

    fn should_block_unidentified_speaker(&self, speaker_result: &Option<IdentifyResult>) -> bool {
        if self.state.device_config.speaker_chat_mode != "identified_only" {
            return false;
        }
        if self.speaker.is_none() {
            return false;
        }
        if self.state.device_config.voice_identify.is_empty() {
            return false;
        }
        !speaker_result
            .as_ref()
            .map(|r| r.identified)
            .unwrap_or(false)
    }

    async fn apply_speaker_tts(&mut self, speaker_result: &Option<IdentifyResult>) {
        let Some(result) = speaker_result else {
            return;
        };
        if !result.identified {
            return;
        }
        let Some(group) = self
            .state
            .device_config
            .voice_identify
            .get(&result.speaker_name)
        else {
            return;
        };

        if let Some(ref config_id) = group.tts_config_id {
            if !config_id.is_empty() {
                if self.active_tts_config_id.as_deref() != Some(config_id.as_str()) {
                    if let Some(cfg) = self.state.device_config.tts_configs.get(config_id) {
                        let mut provider_cfg = cfg.clone();
                        if let Some(voice) = &group.voice {
                            if !voice.is_empty() {
                                provider_cfg
                                    .config
                                    .insert("voice".into(), serde_json::json!(voice));
                            }
                        }
                        match build_tts_provider(&provider_cfg) {
                            Ok(tts) => {
                                self.tts = tts;
                                self.active_tts_config_id = Some(config_id.clone());
                                if let Some(mgr) = self.manager.upgrade() {
                                    mgr.update_session_media_tts(Arc::clone(&self.tts)).await;
                                }
                                tracing::info!(
                                    "声纹 {} 切换 TTS 配置: {}",
                                    result.speaker_name,
                                    config_id
                                );
                            }
                            Err(e) => {
                                tracing::warn!("声纹 TTS 配置 {config_id} 加载失败: {e}");
                            }
                        }
                    } else {
                        tracing::warn!("声纹 TTS 配置 {config_id} 未在 uconfig 中找到");
                    }
                } else if let Some(voice) = &group.voice {
                    if !voice.is_empty() {
                        let _ = self
                            .tts
                            .set_voice(&serde_json::json!({ "voice": voice }))
                            .await;
                    }
                }
                return;
            }
        }

        if self.active_tts_config_id.is_some() {
            if let Ok(tts) = build_tts_provider(&self.default_tts) {
                self.tts = tts;
                self.active_tts_config_id = None;
                if let Some(mgr) = self.manager.upgrade() {
                    mgr.update_session_media_tts(Arc::clone(&self.tts)).await;
                }
            }
        }

        if let Some(voice) = &group.voice {
            if !voice.is_empty() {
                let _ = self
                    .tts
                    .set_voice(&serde_json::json!({ "voice": voice }))
                    .await;
            }
        }
    }

    async fn try_route_openclaw(
        &mut self,
        text: &str,
        delivery: &mut SpeakDelivery,
    ) -> Result<bool> {
        let cfg = &self.state.device_config.openclaw;
        let agent_id = self.state.agent_id.as_str();
        let device_id = self.state.device_id.as_str();
        let session_id = self.state.session_id.as_str();

        if !cfg.allowed {
            if let Some(mgr) = self.manager.upgrade() {
                mgr.finish_openclaw_warmup("", false).await;
            }
            self.openclaw.exit_mode(agent_id, device_id);
            return Ok(false);
        }

        if self.openclaw.is_mode_enabled(agent_id, device_id) {
            if self.openclaw.should_exit(text, cfg) {
                if let Some(mgr) = self.manager.upgrade() {
                    mgr.finish_openclaw_warmup("", true).await;
                }
                self.openclaw.exit_mode(agent_id, device_id);
                let hint = "已退出 OpenClaw 模式";
                delivery.messages.push(ServerMessage::text(hint));
                self.speak_text_for_delivery(hint, delivery).await?;
                return Ok(true);
            }
            if let Some(mgr) = self.manager.upgrade() {
                mgr.finish_openclaw_warmup("", true).await;
            }
            match self
                .openclaw
                .send_message(agent_id, device_id, text, session_id)
            {
                Ok(message_id) => {
                    tracing::info!(
                        agent_id,
                        device_id,
                        message_id = %message_id,
                        "OpenClaw 消息已发送"
                    );
                    if let Some(mgr) = self.manager.upgrade() {
                        mgr.start_openclaw_warmup(&message_id, text).await;
                    }
                }
                Err(e) => {
                    tracing::warn!(agent_id, device_id, "OpenClaw 发送失败: {e}");
                    self.openclaw.exit_mode(agent_id, device_id);
                    let hint = "OpenClaw 当前不可用，已退出 OpenClaw 模式";
                    delivery.messages.push(ServerMessage::text(hint));
                    self.speak_text_for_delivery(hint, delivery).await?;
                }
            }
            return Ok(true);
        }

        if self.openclaw.should_enter(text, cfg) {
            if self.openclaw.enter_mode(agent_id, device_id) {
                let hint = "已进入 OpenClaw 模式，请继续说";
                delivery.messages.push(ServerMessage::text(hint));
                self.speak_text_for_delivery(hint, delivery).await?;
            } else {
                let hint = "OpenClaw 当前不可用，请稍后再试";
                delivery.messages.push(ServerMessage::text(hint));
                self.speak_text_for_delivery(hint, delivery).await?;
            }
            return Ok(true);
        }

        Ok(false)
    }

    async fn speak_text(
        &mut self,
        text: &str,
        _messages: &mut Vec<ServerMessage>,
    ) -> Result<Vec<Vec<u8>>> {
        self.speak_text_for_delivery(text, &mut SpeakDelivery::default())
            .await?;
        Ok(Vec::new())
    }

    pub(crate) async fn speak_text_for_delivery(
        &mut self,
        text: &str,
        _delivery: &mut SpeakDelivery,
    ) -> Result<()> {
        self.state.clear_abort();
        let manager = self
            .manager
            .upgrade()
            .ok_or_else(|| xiaozhi_core::Error::Session("ChatManager 已释放".into()))?;
        let llm = manager
            .llm_manager()
            .await
            .ok_or_else(|| xiaozhi_core::Error::Session("LLM 管理器未初始化".into()))?;
        llm.add_text_to_tts_queue(text).await?;
        Ok(())
    }

    pub async fn handle_abort(&mut self, origin: AbortOrigin) -> SpeakDelivery {
        let manager = match self.manager.upgrade() {
            Some(m) => m,
            None => {
                self.state.trigger_abort();
                self.state.listen_phase = ListenPhase::Idle;
                return SpeakDelivery::default();
            }
        };
        if origin == AbortOrigin::Device && self.state.welcome_playing {
            tracing::info!(
                device_id = %self.state.device_id,
                "欢迎语播放中，忽略设备自发 abort"
            );
            if let Some(tts) = manager.tts_manager().await {
                tts.nudge_tts_speaking_signal();
            }
            return SpeakDelivery::default();
        }
        self.state.trigger_abort();
        let cancel_session = !self.state.is_realtime();
        let reason = if self.state.is_realtime() {
            "HandleAbortMessage realtime"
        } else {
            "HandleAbortMessage auto"
        };
        let delivery = self
            .stop_speaking_with_reason(manager.as_ref(), cancel_session, true, reason, false)
            .await;
        if self.state.is_realtime() && self.state.listen_phase == ListenPhase::Listening {
            // realtime：保留 listen 会话（对齐 Go）
        } else if !cancel_session {
            // assistant-only stop，listen 仍可能有效
        } else {
            self.state.listen_phase = ListenPhase::Idle;
        }
        delivery
    }

    pub fn asr_pipeline(&self) -> &AsrPipeline {
        &self.asr_pipeline
    }

    pub fn asr_pipeline_mut(&mut self) -> &mut AsrPipeline {
        &mut self.asr_pipeline
    }

    pub fn asr_provider(&self) -> Arc<dyn AsrProvider> {
        Arc::clone(&self.asr)
    }

    pub(crate) fn empty_listen_recovery_public(session_id: &str, hint: &str) -> SpeakDelivery {
        Self::empty_listen_recovery(session_id, hint)
    }

    /// 空 ASR 结果 recovery：TTS 播报提示并退出聆听（对齐 Go 设备状态机）
    pub async fn empty_listen_recovery_session(&mut self, hint: &str) -> Result<SpeakDelivery> {
        self.state.listen_phase = ListenPhase::Idle;
        self.asr_pipeline.abort_loop();
        let mut delivery = SpeakDelivery::default();
        self.speak_text(hint, &mut delivery.messages).await?;
        Ok(delivery)
    }

    /// 无有效识别结果时下发短 TTS JSON（无音频，仅用于同步路径 fallback）
    fn empty_listen_recovery(session_id: &str, hint: &str) -> SpeakDelivery {
        let mut delivery = SpeakDelivery::default();
        delivery.messages.push(ServerMessage::tts(
            xiaozhi_core::message::START,
            Some(session_id.to_string()),
        ));
        delivery.messages.push(ServerMessage::tts_sentence(
            hint,
            xiaozhi_core::message::SENTENCE_START,
            Some(session_id.to_string()),
        ));
        delivery.messages.push(ServerMessage::tts(
            xiaozhi_core::message::STOP,
            Some(session_id.to_string()),
        ));
        delivery
    }

    async fn execute_mcp_tool(&self, name: &str, arguments: serde_json::Value) -> String {
        let Some(mgr) = self.manager.upgrade() else {
            return "会话管理器不可用".to_string();
        };
        mgr.execute_tool(name, arguments).await
    }
}

fn build_tts_provider(cfg: &ProviderConfig) -> Result<Arc<dyn TtsProvider>> {
    let mut map = cfg.config.clone();
    map.insert(
        "provider".into(),
        serde_json::Value::String(cfg.provider.clone()),
    );
    let value = serde_json::Value::Object(map.into_iter().collect());
    create_tts(&cfg.provider, &value)
}
