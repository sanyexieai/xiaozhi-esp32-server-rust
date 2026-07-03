//! 会话级 TTS 音频发送队列（对齐 Go `TTSManager.runSenderLoop`）

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Weak;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::sync::{mpsc, oneshot};
use xiaozhi_core::message;
use xiaozhi_protocol::messages::ServerMessage;

use crate::llm_types::LlmResponseChunk;
use crate::manager::ChatManager;
use crate::outbound::OutboundFrame;
use crate::state::ListenPhase;
use crate::tts_turn_policy::TtsTurnEndPolicy;

const SESSION_AUDIO_QUEUE_CAP: usize = 150;
pub(crate) const TTS_PLAYBACK_COMPLETION_GRACE_MS: u64 = 150;
const STOP_SPEAKING_INTERRUPT_TIMEOUT_MS: u64 = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AudioQueueKind {
    Frame,
    SentenceStart,
    SentenceEnd,
    TtsStart,
    TtsStop,
}

type OnStartCb = Arc<dyn Fn() + Send + Sync>;
type OnEndCb = Arc<dyn Fn(Option<String>) + Send + Sync>;

pub type TtsPlaybackStartHook = OnStartCb;

struct AudioQueueElem {
    kind: AudioQueueKind,
    data: Vec<u8>,
    text: String,
    generation: u64,
    debug_reason: String,
    on_start: Option<OnStartCb>,
    on_end: Option<OnEndCb>,
}

struct DelayedSentenceTask {
    elem: AudioQueueElem,
    execute_at: Instant,
}

struct InterruptRequest {
    done: Option<oneshot::Sender<()>>,
    reason: String,
}

struct PendingInterruptStop {
    send_tts_stop: bool,
    reason: String,
}

enum SenderWaitResult {
    Reached,
    Interrupted(InterruptRequest),
}

const TTS_QUEUE_CAP: usize = 10;

struct TtsQueueItem {
    text: String,
    generation: u64,
    on_start: Option<OnStartCb>,
    on_end: Option<OnEndCb>,
}

pub struct TtsManager {
    device_id: String,
    manager: Weak<ChatManager>,
    session_audio_tx: mpsc::Sender<AudioQueueElem>,
    delayed_sentence_tx: mpsc::Sender<DelayedSentenceTask>,
    interrupt_tx: mpsc::Sender<InterruptRequest>,
    tts_queue_tx: mpsc::Sender<TtsQueueItem>,
    audio_generation: AtomicU64,
    tts_queue_seq: AtomicU64,
    dropped_tts_seq: AtomicU64,
    frame_duration_ms: AtomicU32,
    sender_loop_active: AtomicBool,
    tts_active: AtomicBool,
    pending_tts_jobs: AtomicU64,
    pending_interrupt: Mutex<Option<PendingInterruptStop>>,
    turn_end_policy: Mutex<TtsTurnEndPolicy>,
    media_playback_active: AtomicBool,
}

impl TtsManager {
    pub fn new(device_id: String, manager: Weak<ChatManager>) -> Arc<Self> {
        let (session_audio_tx, session_audio_rx) = mpsc::channel(SESSION_AUDIO_QUEUE_CAP);
        let (delayed_sentence_tx, delayed_sentence_rx) = mpsc::channel(SESSION_AUDIO_QUEUE_CAP);
        let (delayed_ready_tx, delayed_ready_rx) = mpsc::channel(SESSION_AUDIO_QUEUE_CAP);
        let (interrupt_tx, interrupt_rx) = mpsc::channel(1);
        let (tts_queue_tx, tts_queue_rx) = mpsc::channel(TTS_QUEUE_CAP);

        let mgr = Arc::new(Self {
            device_id,
            manager,
            session_audio_tx,
            delayed_sentence_tx,
            interrupt_tx,
            tts_queue_tx,
            audio_generation: AtomicU64::new(1),
            tts_queue_seq: AtomicU64::new(0),
            dropped_tts_seq: AtomicU64::new(0),
            frame_duration_ms: AtomicU32::new(60),
            sender_loop_active: AtomicBool::new(false),
            tts_active: AtomicBool::new(false),
            pending_tts_jobs: AtomicU64::new(0),
            pending_interrupt: Mutex::new(None),
            turn_end_policy: Mutex::new(TtsTurnEndPolicy::None),
            media_playback_active: AtomicBool::new(false),
        });

        let sender_mgr = Arc::clone(&mgr);
        tokio::spawn(async move {
            sender_mgr
                .run_delayed_sentence_loop(delayed_sentence_rx, delayed_ready_tx)
                .await;
        });

        let sender_mgr = Arc::clone(&mgr);
        tokio::spawn(async move {
            sender_mgr
                .run_sender_loop(session_audio_rx, delayed_ready_rx, interrupt_rx)
                .await;
        });

        let queue_mgr = Arc::clone(&mgr);
        tokio::spawn(async move {
            queue_mgr.process_tts_queue(tts_queue_rx).await;
        });

        mgr
    }

    /// 对齐 Go `handleTextResponse`（非双流式：每条文本 Push 到 ttsQueue）
    pub async fn handle_text_response(
        &self,
        chunk: LlmResponseChunk,
        on_start: Option<OnStartCb>,
        on_end: Option<OnEndCb>,
    ) -> xiaozhi_core::Result<()> {
        if chunk.text.trim().is_empty() && !chunk.is_end {
            return Ok(());
        }
        if !chunk.text.trim().is_empty() {
            let seq = self.tts_queue_seq.fetch_add(1, Ordering::Relaxed) + 1;
            if seq <= self.dropped_tts_seq.load(Ordering::Relaxed) {
                return Ok(());
            }
            let item = TtsQueueItem {
                text: chunk.text,
                generation: self.current_audio_generation(),
                on_start: if chunk.is_start { on_start } else { None },
                on_end: if chunk.is_end { on_end } else { None },
            };
            self.tts_queue_tx
                .send(item)
                .await
                .map_err(|e| xiaozhi_core::Error::Session(format!("ttsQueue 已满: {e}")))?;
            self.pending_tts_jobs.fetch_add(1, Ordering::Relaxed);
        } else if chunk.is_end {
            if let Some(on_end) = on_end {
                on_end(None);
            }
        }
        Ok(())
    }

    pub fn set_turn_end_policy(&self, policy: TtsTurnEndPolicy) {
        *self.turn_end_policy.lock() = policy;
    }

    fn take_turn_end_policy(&self) -> TtsTurnEndPolicy {
        std::mem::replace(&mut *self.turn_end_policy.lock(), TtsTurnEndPolicy::None)
    }

    /// 对齐 Go `handleTextResponse(..., isSync=true)` 单条播报
    pub async fn handle_text_response_sync(&self, text: &str) -> xiaozhi_core::Result<()> {
        self.handle_text_response_sync_with_on_start(text, None).await
    }

    pub async fn handle_text_response_sync_with_on_start(
        &self,
        text: &str,
        on_start: Option<OnStartCb>,
    ) -> xiaozhi_core::Result<()> {
        let (done_tx, done_rx) = oneshot::channel();
        let on_end: OnEndCb = Arc::new({
            let done_tx = Arc::new(parking_lot::Mutex::new(Some(done_tx)));
            move |_err| {
                if let Some(tx) = done_tx.lock().take() {
                    let _ = tx.send(());
                }
            }
        });
        self.handle_text_response(
            LlmResponseChunk {
                text: text.to_string(),
                is_start: true,
                is_end: true,
            },
            on_start,
            Some(on_end),
        )
        .await?;
        let _ = tokio::time::timeout(Duration::from_secs(30), done_rx).await;
        Ok(())
    }

    pub fn clear_tts_queue(&self) {
        self.dropped_tts_seq
            .store(self.tts_queue_seq.load(Ordering::Relaxed), Ordering::Relaxed);
    }

    /// 对齐 Go `BeginExclusiveMediaPlayback`：打断 TTS 但不立即发 tts_stop
    pub async fn begin_exclusive_media_playback(&self) -> xiaozhi_core::Result<()> {
        if self
            .media_playback_active
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(xiaozhi_core::Error::Session(
                "媒体播放已处于独占状态".into(),
            ));
        }
        self.clear_tts_queue();
        self.interrupt_and_stop_sync(false, "BeginExclusiveMediaPlayback")
            .await;
        Ok(())
    }

    /// 对齐 Go `EndExclusiveMediaPlayback`
    pub fn end_exclusive_media_playback(&self) {
        self.media_playback_active.store(false, Ordering::SeqCst);
    }

    pub async fn finish_tts_turn(&self, reason: &str) {
        self.wait_tts_queue_drain().await;
        self.enqueue_tts_stop(reason).await;
    }

    pub async fn wait_tts_queue_drain(&self) {
        for _ in 0..600 {
            if self.pending_tts_jobs.load(Ordering::Relaxed) == 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    pub fn set_frame_duration_ms(&self, ms: u32) {
        self.frame_duration_ms
            .store(ms.max(1), Ordering::Relaxed);
    }

    pub fn is_tts_active(&self) -> bool {
        self.tts_active.load(Ordering::Relaxed)
    }

    pub async fn enqueue_tts_start(&self, reason: &str) {
        self.enqueue_elem(
            AudioQueueElem {
                kind: AudioQueueKind::TtsStart,
                debug_reason: reason.to_string(),
                data: Vec::new(),
                text: String::new(),
                generation: 0,
                on_start: None,
                on_end: None,
            },
        )
        .await;
    }

    pub async fn enqueue_tts_stop(&self, reason: &str) {
        self.enqueue_elem(
            AudioQueueElem {
                kind: AudioQueueKind::TtsStop,
                debug_reason: reason.to_string(),
                data: Vec::new(),
                text: String::new(),
                generation: 0,
                on_start: None,
                on_end: None,
            },
        )
        .await;
    }

    pub async fn enqueue_sentence_start(&self, text: &str) {
        self.enqueue_elem(
            AudioQueueElem {
                kind: AudioQueueKind::SentenceStart,
                text: text.to_string(),
                data: Vec::new(),
                debug_reason: String::new(),
                generation: 0,
                on_start: None,
                on_end: None,
            },
        )
        .await;
    }

    pub async fn enqueue_sentence_end(&self, text: &str) {
        self.enqueue_elem(
            AudioQueueElem {
                kind: AudioQueueKind::SentenceEnd,
                text: text.to_string(),
                data: Vec::new(),
                debug_reason: String::new(),
                generation: 0,
                on_start: None,
                on_end: None,
            },
        )
        .await;
    }

    pub async fn enqueue_frame(&self, frame: Vec<u8>) {
        self.enqueue_elem(
            AudioQueueElem {
                kind: AudioQueueKind::Frame,
                data: frame,
                text: String::new(),
                debug_reason: String::new(),
                generation: 0,
                on_start: None,
                on_end: None,
            },
        )
        .await;
    }

    pub async fn interrupt_and_stop_sync(&self, send_tts_stop: bool, reason: &str) {
        self.clear_tts_queue();
        self.record_pending_interrupt_stop(send_tts_stop, reason);
        self.next_audio_generation();
        if !self.sender_loop_active.load(Ordering::Relaxed) {
            self.finish_pending_interrupt_stop();
            return;
        }
        let (done_tx, done_rx) = oneshot::channel();
        let req = InterruptRequest {
            done: Some(done_tx),
            reason: reason.to_string(),
        };
        if self.interrupt_tx.try_send(req).is_err() {
            self.finish_pending_interrupt_stop();
            return;
        }
        let _ = tokio::time::timeout(
            Duration::from_millis(STOP_SPEAKING_INTERRUPT_TIMEOUT_MS),
            done_rx,
        )
        .await;
        self.finish_pending_interrupt_stop();
    }

    async fn enqueue_elem(&self, mut elem: AudioQueueElem) {
        elem.generation = self.current_audio_generation();
        let _ = self.session_audio_tx.send(elem).await;
    }

    fn current_audio_generation(&self) -> u64 {
        self.audio_generation.load(Ordering::Relaxed)
    }

    fn next_audio_generation(&self) -> u64 {
        self.audio_generation.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn sentence_control_delay(&self) -> Duration {
        let frame_ms = self.frame_duration_ms.load(Ordering::Relaxed).max(1);
        let cache_frame_count = 120 / frame_ms;
        Duration::from_millis(u64::from(cache_frame_count * frame_ms))
    }

    fn record_pending_interrupt_stop(&self, send_tts_stop: bool, reason: &str) {
        let mut pending = self.pending_interrupt.lock();
        if let Some(existing) = pending.as_mut() {
            existing.send_tts_stop = existing.send_tts_stop || send_tts_stop;
        } else {
            *pending = Some(PendingInterruptStop {
                send_tts_stop,
                reason: reason.to_string(),
            });
        }
    }

    fn consume_pending_interrupt_stop(&self) -> Option<PendingInterruptStop> {
        self.pending_interrupt.lock().take()
    }

    fn finish_pending_interrupt_stop(&self) {
        if let Some(pending) = self.consume_pending_interrupt_stop() {
            self.finish_tts_stop(pending.send_tts_stop, &pending.reason);
        }
    }

    async fn run_delayed_sentence_loop(
        &self,
        mut delayed_rx: mpsc::Receiver<DelayedSentenceTask>,
        ready_tx: mpsc::Sender<AudioQueueElem>,
    ) {
        let mut pending: Vec<DelayedSentenceTask> = Vec::new();
        loop {
            let wait = pending.first().map(|t| {
                t.execute_at
                    .saturating_duration_since(Instant::now())
            });
            tokio::select! {
                maybe_task = delayed_rx.recv() => {
                    let Some(task) = maybe_task else { break };
                    insert_delayed_task(&mut pending, task);
                }
                _ = async {
                    if let Some(d) = wait {
                        tokio::time::sleep(d).await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                }, if wait.is_some() => {
                    let task = pending.remove(0);
                    if task.elem.generation == self.current_audio_generation() {
                        let _ = ready_tx.send(task.elem).await;
                    } else if let Some(on_end) = task.elem.on_end {
                        on_end(Some("canceled".into()));
                    }
                }
            }
        }
    }

    fn dispatch_delayed_sentence(&self, elem: AudioQueueElem) {
        match elem.kind {
            AudioQueueKind::SentenceStart => {
                if let Some(on_start) = &elem.on_start {
                    on_start();
                }
                if !elem.text.is_empty() {
                    self.send_sentence_start(&elem.text);
                }
            }
            AudioQueueKind::SentenceEnd => {
                if !elem.text.is_empty() {
                    self.send_sentence_end(&elem.text);
                }
                if let Some(on_end) = &elem.on_end {
                    on_end(None);
                }
            }
            _ => {}
        }
    }

    fn drain_on_interrupt(
        &self,
        reason: &str,
        session_rx: &mut mpsc::Receiver<AudioQueueElem>,
        delayed_ready_rx: &mut mpsc::Receiver<AudioQueueElem>,
    ) {
        self.drain_session_queue(session_rx);
        self.drain_delayed_queue(delayed_ready_rx);
        if let Some(pending) = self.consume_pending_interrupt_stop() {
            self.finish_tts_stop(pending.send_tts_stop, &pending.reason);
        }
        tracing::info!(
            device_id = %self.device_id,
            reason,
            "runSenderLoop interrupt, drained queue and continue"
        );
    }

    async fn run_sender_loop(
        &self,
        mut session_rx: mpsc::Receiver<AudioQueueElem>,
        mut delayed_ready_rx: mpsc::Receiver<AudioQueueElem>,
        mut interrupt_rx: mpsc::Receiver<InterruptRequest>,
    ) {
        self.sender_loop_active.store(true, Ordering::Relaxed);
        struct LoopGuard<'a>(&'a AtomicBool);
        impl Drop for LoopGuard<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Relaxed);
            }
        }
        let _guard = LoopGuard(&self.sender_loop_active);

        let frame_duration =
            Duration::from_millis(u64::from(self.frame_duration_ms.load(Ordering::Relaxed).max(1)));
        let cache_frame_count = 120 / self.frame_duration_ms.load(Ordering::Relaxed).max(1);
        let allowed_ahead = Duration::from_millis(
            u64::from(cache_frame_count)
                * u64::from(self.frame_duration_ms.load(Ordering::Relaxed).max(1)),
        );
        let mut playback_tail: Option<Instant> = None;

        loop {
            while let Ok(elem) = delayed_ready_rx.try_recv() {
                self.dispatch_delayed_sentence(elem);
            }

            tokio::select! {
                maybe_req = interrupt_rx.recv() => {
                    let Some(req) = maybe_req else { break };
                    self.drain_on_interrupt(&req.reason, &mut session_rx, &mut delayed_ready_rx);
                    playback_tail = None;
                    if let Some(done) = req.done {
                        let _ = done.send(());
                    }
                }
                maybe_elem = delayed_ready_rx.recv() => {
                    let Some(elem) = maybe_elem else { break };
                    self.dispatch_delayed_sentence(elem);
                }
                maybe_elem = session_rx.recv() => {
                    let Some(mut elem) = maybe_elem else { break };
                    if elem.generation != self.current_audio_generation() {
                        if let Some(on_end) = elem.on_end.take() {
                            on_end(Some("canceled".into()));
                        }
                        continue;
                    }
                    match elem.kind {
                        AudioQueueKind::SentenceStart | AudioQueueKind::SentenceEnd => {
                            let task = DelayedSentenceTask {
                                elem,
                                execute_at: Instant::now() + self.sentence_control_delay(),
                            };
                            let _ = self.delayed_sentence_tx.send(task).await;
                        }
                        AudioQueueKind::Frame => {
                            let now = Instant::now();
                            if playback_tail.is_none() || now > playback_tail.unwrap() {
                                playback_tail = Some(now);
                            }
                            let tail = playback_tail.unwrap();
                            let send_at = tail.checked_sub(allowed_ahead).unwrap_or(now);
                            if now < send_at {
                                match self
                                    .wait_until_deadline(send_at, &mut interrupt_rx, &mut delayed_ready_rx)
                                    .await
                                {
                                    SenderWaitResult::Interrupted(req) => {
                                        self.drain_on_interrupt(&req.reason, &mut session_rx, &mut delayed_ready_rx);
                                        playback_tail = None;
                                        if let Some(done) = req.done {
                                            let _ = done.send(());
                                        }
                                        continue;
                                    }
                                    SenderWaitResult::Reached => {}
                                }
                            }
                            if !self.send_audio_frame(&elem.data) {
                                continue;
                            }
                            playback_tail = Some(playback_tail.unwrap() + frame_duration);
                        }
                        AudioQueueKind::TtsStart => {
                            tracing::debug!(
                                device_id = %self.device_id,
                                reason = %elem.debug_reason,
                                "enqueue tts start -> send"
                            );
                            self.tts_active.store(true, Ordering::Relaxed);
                            self.on_tts_started();
                            self.send_tts_start();
                            playback_tail = None;
                        }
                        AudioQueueKind::TtsStop => {
                            tracing::info!(
                                device_id = %self.device_id,
                                reason = %elem.debug_reason,
                                "runSenderLoop processing tts stop"
                            );
                            if let Some(tail) = playback_tail {
                                match self
                                    .wait_until_deadline(tail, &mut interrupt_rx, &mut delayed_ready_rx)
                                    .await
                                {
                                    SenderWaitResult::Interrupted(req) => {
                                        self.drain_on_interrupt(&req.reason, &mut session_rx, &mut delayed_ready_rx);
                                        playback_tail = None;
                                        if let Some(done) = req.done {
                                            let _ = done.send(());
                                        }
                                        continue;
                                    }
                                    SenderWaitResult::Reached => {}
                                }
                            }
                            let grace_deadline =
                                Instant::now() + Duration::from_millis(TTS_PLAYBACK_COMPLETION_GRACE_MS);
                            match self
                                .wait_until_deadline(grace_deadline, &mut interrupt_rx, &mut delayed_ready_rx)
                                .await
                            {
                                SenderWaitResult::Interrupted(req) => {
                                    self.drain_on_interrupt(&req.reason, &mut session_rx, &mut delayed_ready_rx);
                                    playback_tail = None;
                                    if let Some(done) = req.done {
                                        let _ = done.send(());
                                    }
                                    continue;
                                }
                                SenderWaitResult::Reached => {}
                            }
                            self.finish_tts_stop(true, &elem.debug_reason);
                            playback_tail = None;
                        }
                    }
                }
            }
        }
    }

    async fn wait_until_deadline(
        &self,
        deadline: Instant,
        interrupt_rx: &mut mpsc::Receiver<InterruptRequest>,
        delayed_ready_rx: &mut mpsc::Receiver<AudioQueueElem>,
    ) -> SenderWaitResult {
        loop {
            let now = Instant::now();
            if now >= deadline {
                return SenderWaitResult::Reached;
            }
            let wait = deadline - now;
            tokio::select! {
                maybe_req = interrupt_rx.recv() => {
                    if let Some(req) = maybe_req {
                        return SenderWaitResult::Interrupted(req);
                    }
                }
                maybe_elem = delayed_ready_rx.recv() => {
                    if let Some(elem) = maybe_elem {
                        self.dispatch_delayed_sentence(elem);
                    }
                }
                _ = tokio::time::sleep(wait) => {
                    return SenderWaitResult::Reached;
                }
            }
        }
    }

    fn drain_session_queue(&self, rx: &mut mpsc::Receiver<AudioQueueElem>) {
        while let Ok(elem) = rx.try_recv() {
            if let Some(on_end) = elem.on_end {
                on_end(Some("canceled".into()));
            }
        }
    }

    fn drain_delayed_queue(&self, rx: &mut mpsc::Receiver<AudioQueueElem>) {
        while let Ok(elem) = rx.try_recv() {
            if let Some(on_end) = elem.on_end {
                on_end(Some("canceled".into()));
            }
        }
    }

    fn should_dispatch_turn_end_policy(send_tts_stop: bool, reason: &str) -> bool {
        send_tts_stop
            && !reason.contains("canceled")
            && !reason.contains("Abort")
            && !reason.contains("interrupt")
            && !reason.contains("HandleListenStart")
            && !reason.contains("ResetToSilentState")
            && !reason.contains("HandleGoodByeMessage")
    }

    fn dispatch_turn_end_policy_if_needed(&self, policy: TtsTurnEndPolicy, natural: bool) {
        if policy == TtsTurnEndPolicy::None || !natural {
            return;
        }
        if let Some(manager) = self.manager.upgrade() {
            let mgr = manager.clone();
            tokio::spawn(async move {
                mgr.handle_tts_turn_end_policy(policy).await;
            });
        }
    }

    fn finish_tts_stop(&self, send_tts_stop: bool, reason: &str) {
        tracing::info!(
            device_id = %self.device_id,
            reason,
            send_tts_stop,
            "finish tts stop"
        );
        let natural = Self::should_dispatch_turn_end_policy(send_tts_stop, reason);
        let was_active = self.tts_active.swap(false, Ordering::Relaxed);
        if !was_active {
            self.try_start_audio_idle_window();
            self.signal_welcome_playback_complete(natural);
            let policy = self.take_turn_end_policy();
            self.dispatch_turn_end_policy_if_needed(policy, natural);
            return;
        }
        if send_tts_stop {
            self.send_tts_stop();
        }
        self.try_start_audio_idle_window();
        self.signal_welcome_playback_complete(natural);
        let policy = self.take_turn_end_policy();
        self.dispatch_turn_end_policy_if_needed(policy, natural);
    }

    fn signal_welcome_playback_complete(&self, natural: bool) {
        let Some(manager) = self.manager.upgrade() else {
            return;
        };
        let manager = manager.clone();
        tokio::spawn(async move {
            manager.complete_welcome_playback_wait(natural).await;
        });
    }

    fn try_start_audio_idle_window(&self) {
        let Some(manager) = self.manager.upgrade() else {
            return;
        };
        let manager = manager.clone();
        tokio::spawn(async move {
            let mut guard = manager.session.lock().await;
            if let Some(session) = guard.as_mut() {
                session.start_audio_idle_window();
            }
        });
    }

    fn on_tts_started(&self) {
        let Some(manager) = self.manager.upgrade() else {
            return;
        };
        let manager = manager.clone();
        tokio::spawn(async move {
            let mut guard = manager.session.lock().await;
            if let Some(session) = guard.as_mut() {
                session.state_mut().listen_phase = ListenPhase::Speaking;
                session.state_mut().pause_audio_idle_window();
            }
        });
    }

    pub(crate) fn send_tts_stop_signal(&self) {
        self.send_tts_stop();
    }

    /// 欢迎语期间设备可能自发 listen start/abort 并本地停播；重发 tts start 拉回 Speaking，不打断 UDP 队列。
    pub(crate) fn nudge_tts_speaking_signal(&self) {
        if !self.tts_active.load(Ordering::Relaxed) {
            return;
        }
        tracing::debug!(
            device_id = %self.device_id,
            "nudge tts start to keep device in Speaking"
        );
        self.send_tts_start();
    }

    fn send_command(&self, msg: ServerMessage) -> bool {
        let Some(manager) = self.manager.upgrade() else {
            tracing::warn!(device_id = %self.device_id, msg_type = %msg.msg_type, "TTS 信令发送失败: ChatManager 已释放");
            return false;
        };
        let Ok(data) = serde_json::to_vec(&msg) else {
            tracing::warn!(device_id = %self.device_id, msg_type = %msg.msg_type, "TTS 信令序列化失败");
            return false;
        };
        if manager.endpoint_count() == 0 {
            tracing::warn!(
                device_id = %self.device_id,
                msg_type = %msg.msg_type,
                state = ?msg.state,
                "TTS 信令发送失败: outbound 未就绪"
            );
            return false;
        }
        manager.send_outbound_command(data)
    }

    fn send_tts_start(&self) {
        let session_id = self.session_id();
        self.send_command(ServerMessage::tts(message::START, session_id));
    }

    fn send_tts_stop(&self) {
        let session_id = self.session_id();
        self.send_command(ServerMessage::tts(message::STOP, session_id));
        // 对齐 Go `ServerTransport.SendTtsStop`：下发 stop 时清除 welcome_playing
        let Some(manager) = self.manager.upgrade() else {
            return;
        };
        let manager = manager.clone();
        tokio::spawn(async move {
            let mut guard = manager.session.lock().await;
            if let Some(session) = guard.as_mut() {
                session.state_mut().welcome_playing = false;
                // 对齐 Go SendTtsStop：仅结束 Speaking，勿覆盖设备已发起的 Listening。
                if session.state().listen_phase == ListenPhase::Speaking {
                    session.state_mut().listen_phase = ListenPhase::Idle;
                }
            }
            drop(guard);
            manager.try_resume_pending_listen_start().await;
        });
    }

    fn send_sentence_start(&self, text: &str) {
        let session_id = self.session_id();
        self.send_command(ServerMessage::tts_sentence(
            text,
            message::SENTENCE_START,
            session_id,
        ));
    }

    fn send_sentence_end(&self, text: &str) {
        let session_id = self.session_id();
        self.send_command(ServerMessage::tts_sentence(
            text,
            message::SENTENCE_END,
            session_id,
        ));
    }

    fn send_audio_frame(&self, frame: &[u8]) -> bool {
        let Some(manager) = self.manager.upgrade() else {
            return false;
        };
        let proto = manager.binary_protocol_version();
        let packed = xiaozhi_protocol::pack_device_audio(frame, proto);
        if manager.endpoint_count() == 0 {
            return false;
        }
        manager.send_outbound_audio(packed)
    }

    fn session_id(&self) -> Option<String> {
        let manager = self.manager.upgrade()?;
        manager.session_id()
    }

    async fn process_tts_queue(&self, mut rx: mpsc::Receiver<TtsQueueItem>) {
        while let Some(item) = rx.recv().await {
            if item.generation != self.current_audio_generation() {
                if let Some(on_end) = item.on_end {
                    on_end(Some("canceled".into()));
                }
                self.pending_tts_jobs.fetch_sub(1, Ordering::Relaxed);
                continue;
            }
            if let Err(e) = self.handle_tts(item).await {
                tracing::warn!(device_id = %self.device_id, "handleTts 失败: {e:#}");
            }
            self.pending_tts_jobs.fetch_sub(1, Ordering::Relaxed);
        }
    }

    /// 对齐 Go `handleTts`：SentenceStart → Frame… → SentenceEnd
    async fn handle_tts(&self, mut item: TtsQueueItem) -> xiaozhi_core::Result<()> {
        let text = item.text.trim();
        if text.is_empty() {
            if let Some(on_end) = item.on_end {
                on_end(None);
            }
            return Ok(());
        }

        let manager = self
            .manager
            .upgrade()
            .ok_or_else(|| xiaozhi_core::Error::Session("ChatManager 已释放".into()))?;
        let media = manager
            .session_media()
            .await
            .ok_or_else(|| xiaozhi_core::Error::Session("SessionMedia 未初始化".into()))?;

        self.set_frame_duration_ms(media.audio_params.frame_duration.max(1));

        let mut tts_rx = media
            .tts
            .text_to_speech_stream(
                text,
                media.audio_params.sample_rate,
                media.audio_params.channels,
                media.audio_params.frame_duration,
            )
            .await?;

        let mut started = false;
        let mut frame_count = 0u32;
        while let Some(frame) = tts_rx.recv().await {
            if manager.is_session_aborted().await {
                break;
            }
            if frame.is_empty() {
                continue;
            }
            if !started {
                // ESP32 MQTT 仅在 kDeviceStateSpeaking 时接收 UDP 音频；tts start 须紧挨首帧，
                // 避免唤醒后欢迎语在 Listening 态下发导致无声。
                if !self.is_tts_active() {
                    self.enqueue_tts_start("handleTts").await;
                }
                self.enqueue_elem(AudioQueueElem {
                    kind: AudioQueueKind::SentenceStart,
                    text: text.to_string(),
                    data: Vec::new(),
                    debug_reason: String::new(),
                    generation: item.generation,
                    on_start: item.on_start.take(),
                    on_end: None,
                })
                .await;
                started = true;
            }
            self.enqueue_elem(AudioQueueElem {
                kind: AudioQueueKind::Frame,
                data: frame,
                text: String::new(),
                debug_reason: String::new(),
                generation: item.generation,
                on_start: None,
                on_end: None,
            })
            .await;
            frame_count += 1;
        }

        tracing::info!(
            device_id = %self.device_id,
            text_len = text.len(),
            frames = frame_count,
            "TTS 合成完成"
        );

        if !started {
            if !self.is_tts_active() {
                self.enqueue_tts_start("handleTts").await;
            }
            self.enqueue_elem(AudioQueueElem {
                kind: AudioQueueKind::SentenceStart,
                text: text.to_string(),
                data: Vec::new(),
                debug_reason: String::new(),
                generation: item.generation,
                on_start: item.on_start,
                on_end: None,
            })
            .await;
        } else {
            // on_start 已在首帧前消费
            let _ = item.on_start;
        }

        self.enqueue_elem(AudioQueueElem {
            kind: AudioQueueKind::SentenceEnd,
            text: text.to_string(),
            data: Vec::new(),
            debug_reason: String::new(),
            generation: item.generation,
            on_start: None,
            on_end: item.on_end,
        })
        .await;

        Ok(())
    }
}

fn insert_delayed_task(tasks: &mut Vec<DelayedSentenceTask>, task: DelayedSentenceTask) {
    let mut insert_at = tasks.len();
    while insert_at > 0 && task.execute_at < tasks[insert_at - 1].execute_at {
        insert_at -= 1;
    }
    tasks.insert(insert_at, task);
}
