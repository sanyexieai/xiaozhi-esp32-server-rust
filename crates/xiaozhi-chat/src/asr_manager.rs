//! ASR 流式识别（与 Go `internal/app/server/chat/asr.go` 对齐）

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::{mpsc, oneshot};
use xiaozhi_asr::AsrProvider;
use xiaozhi_core::Result;
use xiaozhi_vad::VadProvider;

use crate::manager::ChatManager;
use crate::state::ListenPhase;
use crate::voice_status::{VadTracker, VoiceStatus};

const ASR_AUDIO_CHANNEL_CAP: usize = 100;
const EMPTY_RESULT_PROTECT_WINDOW: Duration = Duration::from_secs(3);
const MAX_EMPTY_RESULT_IN_WINDOW: u32 = 3;
const MAX_ASR_IDLE_RESTART: Duration = Duration::from_secs(60);
const INVALID_STATUS_WAIT_ROUNDS: u32 = 10;
const INVALID_STATUS_WAIT_MS: u64 = 200;
const RECOVERABLE_ASR_ERROR_PROTECT_WINDOW: Duration = Duration::from_secs(10);
const MAX_RECOVERABLE_ASR_ERROR_IN_WINDOW: u32 = 3;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug)]
pub enum AsrTurnOutcome {
    Text(String),
    Empty,
    Error(String),
    Cancelled,
}

struct AsrTurn {
    audio_tx: mpsc::Sender<Vec<f32>>,
    outcome_tx: Option<oneshot::Sender<AsrTurnOutcome>>,
}

pub struct AsrPipeline {
    pub voice: VoiceStatus,
    pub vad: VadTracker,
    pub asr_audio_buffer: Vec<f32>,
    pub asr_auto_end: bool,
    pub frame_duration_ms: u32,
    turn: Option<AsrTurn>,
    loop_handle: Option<tokio::task::JoinHandle<()>>,
    received_text_in_turn: bool,
}

impl AsrPipeline {
    pub fn new(silence_threshold_ms: u64, asr_auto_end: bool) -> Self {
        Self {
            voice: VoiceStatus {
                silence_threshold_ms: silence_threshold_ms as i64,
                ..Default::default()
            },
            vad: VadTracker::default(),
            asr_audio_buffer: Vec::new(),
            asr_auto_end,
            turn: None,
            frame_duration_ms: 20,
            loop_handle: None,
            received_text_in_turn: false,
        }
    }

    pub fn has_open_audio_input(&self) -> bool {
        self.turn.is_some()
    }

    pub fn has_received_text(&self) -> bool {
        self.received_text_in_turn
    }

    pub fn mark_text_received(&mut self) {
        self.received_text_in_turn = true;
    }

    pub fn reset_received_text(&mut self) {
        self.received_text_in_turn = false;
    }

    pub fn on_manual_stop(&mut self) {
        self.on_voice_silence();
    }

    pub fn on_voice_silence(&mut self) {
        self.voice.client_voice_stop = true;
        if let Some(turn) = self.turn.take() {
            drop(turn.audio_tx);
        }
    }

    pub fn abort_turn(&mut self) {
        if let Some(mut turn) = self.turn.take() {
            if let Some(tx) = turn.outcome_tx.take() {
                let _ = tx.send(AsrTurnOutcome::Cancelled);
            }
        }
    }

    pub fn abort_loop(&mut self) {
        if let Some(handle) = self.loop_handle.take() {
            handle.abort();
        }
        self.abort_turn();
    }

    /// 重置语音/VAD 缓冲，允许继续拾音（Go `ResumeAudioIdleWindow` 后重启 ASR）
    pub fn resume_audio_idle(&mut self) {
        self.voice.reset();
        self.vad.reset_all();
        self.asr_audio_buffer.clear();
    }

    /// 启动 ASR 结果循环（每次 listen start 调用一次，对齐 Go `StartAsrRecognitionLoop`）
    pub fn start_recognition_loop(
        &mut self,
        manager: Arc<ChatManager>,
        asr: Arc<dyn AsrProvider>,
        start_seq: u64,
        first_outcome: oneshot::Receiver<AsrTurnOutcome>,
    ) {
        if let Some(handle) = self.loop_handle.take() {
            handle.abort();
        }
        let device_id = manager.device_id().to_string();
        let mgr_log = Arc::clone(&manager);
        self.loop_handle = Some(tokio::spawn(async move {
            if let Err(e) =
                run_asr_recognition_loop(mgr_log, asr, start_seq, Some(first_outcome)).await
            {
                tracing::error!(device_id = %device_id, "ASR 结果循环异常: {e:#}");
            }
        }));
    }

    /// 与 Go `RestartAsrRecognition` 等价：开启新一轮流式识别并等待 outcome
    pub async fn restart_asr_recognition(
        &mut self,
        manager: Arc<ChatManager>,
        asr: Arc<dyn AsrProvider>,
    ) -> Result<oneshot::Receiver<AsrTurnOutcome>> {
        if let Some(mut turn) = self.turn.take() {
            drop(turn.audio_tx);
            if let Some(tx) = turn.outcome_tx.take() {
                let _ = tx.send(AsrTurnOutcome::Cancelled);
            }
        }

        self.reset_received_text();
        self.voice.client_voice_stop = false;

        let (audio_tx, audio_rx) = mpsc::channel(ASR_AUDIO_CHANNEL_CAP);
        let mut result_rx = asr.streaming_recognize(audio_rx).await?;
        let (outcome_tx, outcome_rx) = oneshot::channel();
        let manager_weak = Arc::downgrade(&manager);

        tokio::spawn(async move {
            let mut final_text = String::new();
            let mut marked_first_text = false;
            while let Some(result) = result_rx.recv().await {
                if let Some(err) = result.error {
                    let _ = outcome_tx.send(AsrTurnOutcome::Error(err));
                    return;
                }
                if !result.text.is_empty() {
                    if !marked_first_text {
                        marked_first_text = true;
                        if let Some(mgr) = manager_weak.upgrade() {
                            let mut guard = mgr.session.lock().await;
                            if let Some(session) = guard.as_mut() {
                                session.asr_pipeline_mut().mark_text_received();
                                session.state_mut().clear_audio_idle_timeout_pending();
                                session.state_mut().pause_audio_idle_window();
                            }
                        }
                    }
                    final_text = result.text.clone();
                }
                if result.is_final {
                    break;
                }
            }
            let outcome = if final_text.trim().is_empty() {
                AsrTurnOutcome::Empty
            } else {
                AsrTurnOutcome::Text(final_text)
            };
            let _ = outcome_tx.send(outcome);
        });

        self.turn = Some(AsrTurn {
            audio_tx,
            outcome_tx: None,
        });

        Ok(outcome_rx)
    }

    pub fn add_audio_data(&self, pcm: &[f32]) {
        if let Some(turn) = &self.turn {
            let data = pcm.to_vec();
            if turn.audio_tx.try_send(data).is_err() {
                tracing::warn!("ASR 音频通道已满，跳过本帧（对齐 Go AsrAudioChannel 满时丢弃）");
            }
        }
    }

    pub fn process_pcm_frame(
        &mut self,
        pcm_f32: &[f32],
        pcm_i16: &[i16],
        listen_mode: &str,
        listen_phase: ListenPhase,
        vad: &mut Box<dyn VadProvider>,
        suppress_end_detection: bool,
    ) -> bool {
        if self.voice.client_voice_stop {
            return false;
        }
        if listen_phase != ListenPhase::Listening {
            return false;
        }

        let frame_ms = self.frame_duration_ms.max(1) as i64;
        let mut skip_vad = false;
        let mut have_voice = false;
        let mut client_have_voice = self.voice.have_voice;

        if listen_mode == "manual" {
            skip_vad = true;
            client_have_voice = true;
            have_voice = true;
        } else if self.asr_auto_end {
            skip_vad = true;
            have_voice = true;
        }

        if !skip_vad {
            self.asr_audio_buffer.extend_from_slice(pcm_f32);
            if self.asr_audio_buffer.len() >= pcm_f32.len() {
                have_voice = vad.is_vad(pcm_i16).unwrap_or(false);
                if have_voice && !client_have_voice {
                    let keep_samples = (200 * 16) as usize;
                    let all = std::mem::take(&mut self.asr_audio_buffer);
                    if all.len() > keep_samples {
                        self.asr_audio_buffer = all[all.len() - keep_samples..].to_vec();
                    } else {
                        self.asr_audio_buffer = all;
                    }
                }
            }
        }

        if have_voice {
            self.voice.have_voice = true;
            self.voice.have_voice_last_time_ms = now_ms();
            self.vad.reset_idle();
            self.vad.add_voice(frame_ms);
        } else {
            self.vad.add_idle(frame_ms);
            if !client_have_voice {
                return false;
            }
        }

        if client_have_voice || have_voice {
            let to_send = if !skip_vad && have_voice && !client_have_voice {
                std::mem::take(&mut self.asr_audio_buffer)
            } else {
                pcm_f32.to_vec()
            };
            if !to_send.is_empty() {
                self.add_audio_data(&to_send);
            }
        }

        if client_have_voice
            && self.voice.have_voice_last_time_ms > 0
            && !have_voice
            && !suppress_end_detection
        {
            let voice_duration = self.vad.voice_duration_in_session_ms;
            if voice_duration < 100 {
                self.voice.have_voice = false;
                self.vad.reset_voice_in_session();
                return false;
            }
            if self.voice.is_silence(self.vad.idle_duration_ms) {
                tracing::info!(
                    idle_ms = self.vad.idle_duration_ms,
                    "判定语音结束，停止 ASR"
                );
                self.on_voice_silence();
                return true;
            }
        }
        false
    }
}

async fn run_asr_recognition_loop(
    manager: Arc<ChatManager>,
    asr: Arc<dyn AsrProvider>,
    start_seq: u64,
    mut pending_outcome: Option<oneshot::Receiver<AsrTurnOutcome>>,
) -> Result<()> {
    let mut empty_window_start = Instant::now();
    let mut empty_result_count = 0u32;
    let mut start_idle = Instant::now();
    let mut invalid_status_waits = 0u32;
    let mut recoverable_window_start = Instant::now();
    let mut recoverable_error_count = 0u32;

    loop {
        if !manager.is_current_listen_start(start_seq) {
            tracing::debug!(device_id = %manager.device_id(), "ASR 循环 listen start 已失效，退出");
            return Ok(());
        }

        let outcome_rx = match pending_outcome.take() {
            Some(rx) => rx,
            None => {
                let mut guard = manager.session.lock().await;
                let session = guard
                    .as_mut()
                    .ok_or_else(|| xiaozhi_core::Error::Session("会话未初始化".into()))?;
                session
                    .asr_pipeline_mut()
                    .restart_asr_recognition(Arc::clone(&manager), Arc::clone(&asr))
                    .await?
            }
        };

        let outcome = match outcome_rx.await {
            Ok(o) => o,
            Err(_) => AsrTurnOutcome::Cancelled,
        };

        if !manager.is_current_listen_start(start_seq) {
            return Ok(());
        }

        match outcome {
            AsrTurnOutcome::Text(text) => {
                empty_result_count = 0;
                empty_window_start = Instant::now();
                invalid_status_waits = 0;

                if manager
                    .session
                    .lock()
                    .await
                    .as_ref()
                    .map(|s| s.state().audio_idle_timeout_pending())
                    .unwrap_or(false)
                {
                    manager
                        .clone()
                        .close_audio_idle_timeout_if_pending()
                        .await;
                    return Ok(());
                }

                manager.enqueue_chat_text(text, true).await;

                let is_realtime = manager.is_session_realtime().await;
                if !is_realtime {
                    tracing::debug!(device_id = %manager.device_id(), "非 realtime ASR 完成，退出循环");
                    return Ok(());
                }
                start_idle = Instant::now();
                continue;
            }
            AsrTurnOutcome::Empty => {
                if manager
                    .session
                    .lock()
                    .await
                    .as_ref()
                    .map(|s| s.state().audio_idle_timeout_pending())
                    .unwrap_or(false)
                {
                    manager
                        .clone()
                        .close_audio_idle_timeout_if_pending()
                        .await;
                    return Ok(());
                }
                if manager.is_welcome_playing().await {
                    tracing::debug!(
                        device_id = %manager.device_id(),
                        "欢迎语播放中，忽略 ASR 空结果"
                    );
                    invalid_status_waits = 0;
                    tokio::time::sleep(Duration::from_millis(INVALID_STATUS_WAIT_MS)).await;
                    continue;
                }
                tracing::warn!(device_id = %manager.device_id(), "ASR 空结果");
                if empty_window_start.elapsed() <= EMPTY_RESULT_PROTECT_WINDOW {
                    empty_result_count += 1;
                } else {
                    empty_window_start = Instant::now();
                    empty_result_count = 1;
                }
                if empty_result_count >= MAX_EMPTY_RESULT_IN_WINDOW {
                    tracing::warn!(
                        device_id = %manager.device_id(),
                        count = empty_result_count,
                        "ASR 空结果过多，触发 recovery"
                    );
                    let _ = manager.dispatch_asr_empty_result().await;
                    return Ok(());
                }
                if start_idle.elapsed() > MAX_ASR_IDLE_RESTART {
                    tracing::warn!(
                        device_id = %manager.device_id(),
                        "ASR 空闲超时，触发 recovery"
                    );
                    let _ = manager.dispatch_asr_empty_result().await;
                    return Ok(());
                }
                if !manager.is_allowed_asr_restart(start_seq).await {
                    invalid_status_waits += 1;
                    if invalid_status_waits >= INVALID_STATUS_WAIT_ROUNDS {
                        tracing::debug!(
                            device_id = %manager.device_id(),
                            "状态不允许 ASR 重启，退出循环"
                        );
                        let _ = manager.dispatch_asr_empty_result().await;
                        return Ok(());
                    }
                    tokio::time::sleep(Duration::from_millis(INVALID_STATUS_WAIT_MS)).await;
                    continue;
                }
                invalid_status_waits = 0;
                {
                    let mut guard = manager.session.lock().await;
                    if let Some(session) = guard.as_mut() {
                        let pipeline = session.asr_pipeline_mut();
                        pipeline.voice.reset();
                        pipeline.vad.reset_all();
                        pipeline.asr_audio_buffer.clear();
                        session.state_mut().resume_audio_idle_window();
                    }
                }
                continue;
            }
            AsrTurnOutcome::Error(err) => {
                if manager
                    .session
                    .lock()
                    .await
                    .as_ref()
                    .map(|s| s.state().audio_idle_timeout_pending())
                    .unwrap_or(false)
                {
                    manager
                        .clone()
                        .close_audio_idle_timeout_if_pending()
                        .await;
                    return Ok(());
                }
                if manager.is_welcome_playing().await {
                    tracing::debug!(
                        device_id = %manager.device_id(),
                        "欢迎语播放中，忽略 ASR 错误: {err}"
                    );
                    invalid_status_waits = 0;
                    tokio::time::sleep(Duration::from_millis(INVALID_STATUS_WAIT_MS)).await;
                    continue;
                }
                if is_recoverable_asr_error(&err) {
                    if recoverable_window_start.elapsed() > RECOVERABLE_ASR_ERROR_PROTECT_WINDOW {
                        recoverable_window_start = Instant::now();
                        recoverable_error_count = 0;
                    }
                    recoverable_error_count += 1;
                    tracing::warn!(
                        device_id = %manager.device_id(),
                        count = recoverable_error_count,
                        "ASR 可恢复错误，尝试重启: {err}"
                    );
                    if recoverable_error_count >= MAX_RECOVERABLE_ASR_ERROR_IN_WINDOW {
                        tracing::error!(
                            device_id = %manager.device_id(),
                            "ASR 可恢复错误过多，触发 recovery"
                        );
                        let _ = manager.dispatch_asr_empty_result().await;
                        return Ok(());
                    }
                    if !manager.is_allowed_asr_restart(start_seq).await {
                        invalid_status_waits += 1;
                        if invalid_status_waits >= INVALID_STATUS_WAIT_ROUNDS {
                            let _ = manager.dispatch_asr_empty_result().await;
                            return Ok(());
                        }
                        tokio::time::sleep(Duration::from_millis(INVALID_STATUS_WAIT_MS)).await;
                        continue;
                    }
                    invalid_status_waits = 0;
                    if err.to_ascii_lowercase().contains("no_valid_audio") {
                        let mut guard = manager.session.lock().await;
                        if let Some(session) = guard.as_mut() {
                            session.extend_listen_warmup(Duration::from_millis(800));
                        }
                    }
                    {
                        let mut guard = manager.session.lock().await;
                        if let Some(session) = guard.as_mut() {
                            let pipeline = session.asr_pipeline_mut();
                            pipeline.voice.reset();
                            pipeline.vad.reset_all();
                            pipeline.asr_audio_buffer.clear();
                            session.state_mut().resume_audio_idle_window();
                        }
                    }
                    continue;
                }
                tracing::error!(device_id = %manager.device_id(), "ASR 流式识别错误: {err}");
                manager
                    .clone()
                    .close_session_with_reason(crate::SessionCloseReason::FatalError)
                    .await;
                return Ok(());
            }
            AsrTurnOutcome::Cancelled => {
                if manager
                    .session
                    .lock()
                    .await
                    .as_ref()
                    .map(|s| s.state().audio_idle_timeout_pending())
                    .unwrap_or(false)
                {
                    manager
                        .clone()
                        .close_audio_idle_timeout_if_pending()
                        .await;
                }
                tracing::debug!(device_id = %manager.device_id(), "ASR turn 已取消");
                return Ok(());
            }
        }
    }
}

fn is_recoverable_asr_error(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    e.contains("timeout")
        || e.contains("task-failed")
        || e.contains("request timeout")
        || e.contains("wait task-started")
}

/// 对齐 Go `runAudioIdleTimeoutWatchdog`
pub async fn run_audio_idle_timeout_watchdog(manager: Arc<ChatManager>) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut pending_stuck_ticks = 0u32;
    const PENDING_FORCE_CLOSE_TICKS: u32 = 5;

    loop {
        interval.tick().await;

        let tts_active = manager
            .tts_manager()
            .await
            .map(|t| t.is_tts_active())
            .unwrap_or(false);

        let (should_trigger, timeout_pending, elapsed_ms, threshold_ms) = {
            let mut guard = manager.session.lock().await;
            let Some(session) = guard.as_mut() else {
                tracing::debug!(
                    device_id = %manager.device_id(),
                    "会话已关闭，音频空闲 watchdog 退出"
                );
                return;
            };
            let state = session.state();
            let threshold = manager.max_idle_duration_ms();
            let elapsed = state.audio_idle_elapsed_ms();
            let pending = state.audio_idle_timeout_pending();

            if pending {
                (false, true, elapsed, threshold)
            } else if !state.uses_audio_idle_clock()
                || !state.audio_idle_started()
                || state.audio_idle_paused()
            {
                (false, false, elapsed, threshold)
            } else if !manager.should_count_audio_idle_timeout_for(session, tts_active) {
                (false, false, elapsed, threshold)
            } else if session.asr_pipeline().has_received_text() {
                (false, false, elapsed, threshold)
            } else if session.asr_pipeline().voice.client_voice_stop {
                (false, false, elapsed, threshold)
            } else {
                let trigger = threshold != u64::MAX
                    && elapsed >= threshold
                    && session.state_mut().mark_audio_idle_timeout_pending();
                (trigger, false, elapsed, threshold)
            }
        };

        if timeout_pending {
            pending_stuck_ticks += 1;
            if pending_stuck_ticks >= PENDING_FORCE_CLOSE_TICKS {
                tracing::warn!(
                    device_id = %manager.device_id(),
                    elapsed_ms,
                    threshold_ms,
                    "音频空闲超时收口等待过久，强制关闭会话"
                );
                manager
                    .clone()
                    .close_audio_idle_timeout_if_pending()
                    .await;
                pending_stuck_ticks = 0;
            }
            continue;
        }
        pending_stuck_ticks = 0;

        if should_trigger {
            tracing::info!(
                device_id = %manager.device_id(),
                elapsed_ms,
                threshold_ms,
                "音频空闲超时阈值已到"
            );
            manager.clone().trigger_audio_idle_timeout().await;
        }
    }
}
