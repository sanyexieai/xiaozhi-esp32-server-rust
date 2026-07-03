//! 会话媒体播放器（对齐 Go `SessionMediaPlayer` / `deviceMediaRuntime`）

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use dashmap::DashMap;

use serde_json::json;
use tokio::sync::{Mutex, Notify};
use tokio_util::sync::CancellationToken;
use xiaozhi_core::{message, Error, Result};
use xiaozhi_protocol::audio::AudioParams;
use xiaozhi_protocol::messages::ServerMessage;
use xiaozhi_tts::wrap_tts_audio_stream;

use crate::manager::ChatManager;
use crate::media_coordinator::shared_media_coordinator;
use crate::outbound::OutboundFrame;
use crate::mcp_tool_media;
use crate::play_music;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaSourceType {
    HttpUrl,
    McpResource,
    InlineAudio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackStatus {
    Idle,
    Playing,
    Paused,
    Stopped,
    Error,
}

impl PlaybackStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Playing => "playing",
            Self::Paused => "paused",
            Self::Stopped => "stopped",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MediaPlayerState {
    pub status: PlaybackStatus,
    pub current_title: String,
    pub position_ms: u64,
    pub playlist_length: usize,
    pub current_index: i32,
}

impl Default for MediaPlayerState {
    fn default() -> Self {
        Self {
            status: PlaybackStatus::Idle,
            current_title: String::new(),
            position_ms: 0,
            playlist_length: 0,
            current_index: -1,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum MediaTrack {
    Bytes {
        title: String,
        data: Vec<u8>,
        audio_format: String,
        source_type: MediaSourceType,
    },
    Url {
        title: String,
        url: String,
    },
    McpResource {
        title: String,
        tool_name: String,
        resource_uri: String,
        read_args: serde_json::Value,
    },
}

impl MediaTrack {
    pub(crate) fn title(&self) -> &str {
        match self {
            Self::Bytes { title, .. }
            | Self::Url { title, .. }
            | Self::McpResource { title, .. } => title,
        }
    }

    pub(crate) fn source_type(&self) -> MediaSourceType {
        match self {
            Self::Bytes { source_type, .. } => *source_type,
            Self::Url { .. } => MediaSourceType::HttpUrl,
            Self::McpResource { .. } => MediaSourceType::McpResource,
        }
    }

    pub(crate) fn can_enqueue_to_agent_playlist(&self) -> bool {
        !matches!(self.source_type(), MediaSourceType::InlineAudio)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlaybackMode {
    Standalone,
    AgentPlaylist,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MediaPauseReason {
    None,
    User,
    Interrupt,
}

pub(crate) struct PlaybackRuntime {
    status: PlaybackStatus,
    mode: PlaybackMode,
    agent_id: String,
    playlist: Vec<MediaTrack>,
    current_index: i32,
    current_title: String,
    current_source_type: MediaSourceType,
    position_ms: Arc<AtomicU64>,
    cancel: CancellationToken,
    paused: Arc<AtomicBool>,
    resume_notify: Arc<Notify>,
    task: Option<tokio::task::JoinHandle<()>>,
    resume_on_attach: bool,
    pause_reason: MediaPauseReason,
    session_attached: Arc<AtomicBool>,
    attachment_notify: Arc<Notify>,
}

impl Default for PlaybackRuntime {
    fn default() -> Self {
        Self {
            status: PlaybackStatus::Idle,
            mode: PlaybackMode::Standalone,
            agent_id: String::new(),
            playlist: Vec::new(),
            current_index: -1,
            current_title: String::new(),
            current_source_type: MediaSourceType::HttpUrl,
            position_ms: Arc::new(AtomicU64::new(0)),
            cancel: CancellationToken::new(),
            paused: Arc::new(AtomicBool::new(false)),
            resume_notify: Arc::new(Notify::new()),
            task: None,
            resume_on_attach: false,
            pause_reason: MediaPauseReason::None,
            session_attached: Arc::new(AtomicBool::new(true)),
            attachment_notify: Arc::new(Notify::new()),
        }
    }
}

static DEVICE_RUNTIMES: OnceLock<DashMap<String, Arc<Mutex<PlaybackRuntime>>>> = OnceLock::new();

fn device_runtime_registry() -> &'static DashMap<String, Arc<Mutex<PlaybackRuntime>>> {
    DEVICE_RUNTIMES.get_or_init(DashMap::new)
}

/// 对齐 Go `mediaPlaybackCoordinator.getOrCreateRuntime`
pub(crate) fn get_or_create_device_runtime(device_id: &str) -> Arc<Mutex<PlaybackRuntime>> {
    let device_id = device_id.trim();
    let key = if device_id.is_empty() {
        "_unknown_device".to_string()
    } else {
        device_id.to_string()
    };
    device_runtime_registry()
        .entry(key)
        .or_insert_with(|| Arc::new(Mutex::new(PlaybackRuntime::default())))
        .clone()
}

#[derive(Clone)]
pub struct SessionMediaPlayer {
    device_id: String,
    runtime: Arc<Mutex<PlaybackRuntime>>,
}

impl SessionMediaPlayer {
    pub fn new(device_id: String) -> Self {
        let runtime = get_or_create_device_runtime(&device_id);
        Self { device_id, runtime }
    }

    pub async fn get_state(&self) -> MediaPlayerState {
        let inner = self.runtime.lock().await;
        let (playlist_length, current_index) = match inner.mode {
            PlaybackMode::AgentPlaylist if !inner.agent_id.is_empty() => {
                let len = shared_media_coordinator()
                    .agent_playlist_len(&inner.agent_id)
                    .await;
                (len, inner.current_index)
            }
            _ => (inner.playlist.len(), inner.current_index),
        };
        MediaPlayerState {
            status: inner.status,
            current_title: inner.current_title.clone(),
            position_ms: inner.position_ms.load(Ordering::Relaxed),
            playlist_length,
            current_index,
        }
    }

    pub async fn is_playing(&self) -> bool {
        self.runtime.lock().await.status == PlaybackStatus::Playing
    }

    pub async fn has_realtime_control_context(&self) -> bool {
        let inner = self.runtime.lock().await;
        realtime_mcp_audio_gate_status(&inner).0
    }

    pub async fn should_gate_realtime_asr(&self) -> bool {
        let inner = self.runtime.lock().await;
        realtime_mcp_audio_gate_status(&inner).1
    }

    pub async fn play_mp3(
        &self,
        manager: Arc<ChatManager>,
        title: String,
        mp3_data: Vec<u8>,
    ) -> Result<()> {
        self.play_inline_audio(manager, title, mp3_data, "mp3").await
    }

    pub async fn play_inline_audio(
        &self,
        manager: Arc<ChatManager>,
        title: String,
        data: Vec<u8>,
        audio_format: &str,
    ) -> Result<()> {
        self.replace_playlist_and_play(
            manager,
            vec![MediaTrack::Bytes {
                title,
                data,
                audio_format: audio_format.to_string(),
                source_type: MediaSourceType::InlineAudio,
            }],
            0,
            PlaybackMode::Standalone,
            String::new(),
        )
        .await
    }

    pub async fn play_mcp_resource(
        &self,
        manager: Arc<ChatManager>,
        title: String,
        tool_name: String,
        resource_uri: String,
        read_args: serde_json::Value,
    ) -> Result<()> {
        self.replace_playlist_and_play(
            manager,
            vec![MediaTrack::McpResource {
                title,
                tool_name,
                resource_uri,
                read_args,
            }],
            0,
            PlaybackMode::Standalone,
            String::new(),
        )
        .await
    }

    pub async fn play_url(
        &self,
        manager: Arc<ChatManager>,
        title: String,
        url: String,
    ) -> Result<()> {
        self.replace_playlist_and_play(
            manager,
            vec![MediaTrack::Url { title, url }],
            0,
            PlaybackMode::Standalone,
            String::new(),
        )
        .await
    }

    async fn replace_playlist_and_play(
        &self,
        manager: Arc<ChatManager>,
        playlist: Vec<MediaTrack>,
        start_index: usize,
        mode: PlaybackMode,
        agent_id: String,
    ) -> Result<()> {
        if playlist.is_empty() {
            return Err(Error::Session("播放列表为空".into()));
        }
        if start_index >= playlist.len() {
            return Err(Error::Session(format!("无效的播放索引: {start_index}")));
        }

        self.stop_internal().await;

        if let Some(tts) = manager.tts_manager().await {
            let _ = tts.begin_exclusive_media_playback().await;
        }

        let mut inner = self.runtime.lock().await;
        inner.mode = mode;
        inner.agent_id = agent_id;
        inner.playlist = playlist;
        inner.current_index = start_index as i32;
        inner.current_title = track_title(&inner.playlist[start_index]);
        inner.current_source_type = track_source_type(&inner.playlist[start_index]);
        inner.position_ms.store(0, Ordering::Relaxed);
        inner.status = PlaybackStatus::Playing;
        inner.cancel = CancellationToken::new();
        inner.paused.store(false, Ordering::SeqCst);
        inner.session_attached.store(true, Ordering::SeqCst);

        let player = self.clone();
        let cancel = inner.cancel.clone();
        let paused = Arc::clone(&inner.paused);
        let resume_notify = Arc::clone(&inner.resume_notify);
        let position_ms = Arc::clone(&inner.position_ms);
        let session_attached = Arc::clone(&inner.session_attached);
        let attachment_notify = Arc::clone(&inner.attachment_notify);

        let handle = tokio::spawn(async move {
            if let Err(e) = player
                .run_playlist_from(
                    manager,
                    start_index,
                    cancel,
                    paused,
                    resume_notify,
                    position_ms,
                    session_attached,
                    attachment_notify,
                )
                .await
            {
                tracing::warn!("播放列表结束: {e}");
            }
        });
        inner.task = Some(handle);
        Ok(())
    }

    async fn run_playlist_from(
        &self,
        manager: Arc<ChatManager>,
        mut index: usize,
        cancel: CancellationToken,
        paused: Arc<AtomicBool>,
        resume_notify: Arc<Notify>,
        position_ms: Arc<AtomicU64>,
        session_attached: Arc<AtomicBool>,
        attachment_notify: Arc<Notify>,
    ) -> Result<()> {
        loop {
            if cancel.is_cancelled() {
                break;
            }

            let track = {
                let mut inner = self.runtime.lock().await;
                if inner.mode == PlaybackMode::AgentPlaylist && !inner.agent_id.is_empty() {
                    let snapshot = shared_media_coordinator()
                        .snapshot_agent_playlist(&inner.agent_id)
                        .await;
                    if index >= snapshot.len() {
                        break;
                    }
                    inner.playlist = snapshot.iter().map(|i| i.track.clone()).collect();
                    snapshot[index].track.clone()
                } else {
                    if index >= inner.playlist.len() {
                        break;
                    }
                    inner.playlist[index].clone()
                }
            };

            {
                let mut inner = self.runtime.lock().await;
                inner.current_index = index as i32;
                inner.current_title = track_title(&track);
                inner.current_source_type = track_source_type(&track);
                inner.status = PlaybackStatus::Playing;
            }

            let finished = match track {
                MediaTrack::Bytes {
                    title,
                    data,
                    audio_format,
                    ..
                } => run_bytes_playback(
                    &manager,
                    &title,
                    data,
                    &audio_format,
                    &cancel,
                    &paused,
                    &resume_notify,
                    &position_ms,
                    &session_attached,
                    &attachment_notify,
                )
                .await
                .unwrap_or(false),
                MediaTrack::Url { title, url } => run_url_playback(
                    &manager,
                    &title,
                    &url,
                    &cancel,
                    &paused,
                    &resume_notify,
                    &position_ms,
                    &session_attached,
                    &attachment_notify,
                )
                .await
                .unwrap_or(false),
                MediaTrack::McpResource {
                    title,
                    tool_name,
                    resource_uri,
                    read_args,
                } => run_mcp_resource_playback(
                    &manager,
                    &title,
                    &tool_name,
                    &resource_uri,
                    read_args,
                    &cancel,
                    &paused,
                    &resume_notify,
                    &position_ms,
                    &session_attached,
                    &attachment_notify,
                )
                .await
                .unwrap_or(false),
            };

            if cancel.is_cancelled() || !finished {
                break;
            }

            let has_next = {
                let inner = self.runtime.lock().await;
                match inner.mode {
                    PlaybackMode::AgentPlaylist if !inner.agent_id.is_empty() => {
                        let len = shared_media_coordinator()
                            .agent_playlist_len(&inner.agent_id)
                            .await;
                        index + 1 < len
                    }
                    _ => index + 1 < inner.playlist.len(),
                }
            };
            if !has_next {
                break;
            }
            index += 1;
        }

        let mut inner = self.runtime.lock().await;
        if !cancel.is_cancelled() {
            inner.status = PlaybackStatus::Idle;
        }
        Ok(())
    }

    pub async fn attach_session(&self, manager: Arc<ChatManager>) {
        let device_id = manager.device_id().to_string();
        {
            let inner = self.runtime.lock().await;
            inner.session_attached.store(true, Ordering::SeqCst);
            inner.attachment_notify.notify_waiters();
        }
        let should_resume = {
            let inner = self.runtime.lock().await;
            inner.resume_on_attach
        };
        if !should_resume {
            tracing::debug!(
                device_id = %device_id,
                "媒体播放 attachment 已绑定"
            );
            return;
        }
        tracing::info!(
            device_id = %device_id,
            "媒体播放 attachment 已绑定，尝试恢复播放"
        );
        if let Err(e) = self.recover_playback(manager).await {
            tracing::warn!(device_id = %device_id, "恢复媒体播放失败: {e}");
        }
    }

    /// 对齐 Go `DetachSession`：preserve=true 时暂停并标记 resume_on_attach
    pub async fn detach_session(&self, preserve: bool) {
        if preserve {
            let mut inner = self.runtime.lock().await;
            inner.session_attached.store(false, Ordering::SeqCst);
            inner.attachment_notify.notify_waiters();
            if inner.status == PlaybackStatus::Playing {
                inner.paused.store(true, Ordering::SeqCst);
                inner.status = PlaybackStatus::Paused;
                inner.resume_on_attach = true;
                tracing::debug!("媒体播放 attachment 已解绑(preserve)，保留恢复标记");
            }
            return;
        }
        self.stop_internal().await;
    }

    async fn recover_playback(&self, manager: Arc<ChatManager>) -> Result<()> {
        let (paused, playlist_empty, start, playlist, mode, agent_id) = {
            let inner = self.runtime.lock().await;
            (
                inner.status == PlaybackStatus::Paused,
                inner.playlist.is_empty(),
                inner.current_index.max(0) as usize,
                inner.playlist.clone(),
                inner.mode,
                inner.agent_id.clone(),
            )
        };
        if playlist_empty {
            let mut inner = self.runtime.lock().await;
            inner.resume_on_attach = false;
            return Ok(());
        }
        if paused {
            let mut inner = self.runtime.lock().await;
            inner.resume_on_attach = false;
            inner.paused.store(false, Ordering::SeqCst);
            inner.status = PlaybackStatus::Playing;
            inner.pause_reason = MediaPauseReason::None;
            inner.resume_notify.notify_waiters();
            return Ok(());
        }
        {
            let mut inner = self.runtime.lock().await;
            inner.resume_on_attach = false;
        }
        self.replace_playlist_and_play(manager, playlist, start, mode, agent_id)
            .await
    }

    pub async fn stop(&self) {
        self.stop_internal().await;
    }

    async fn stop_internal(&self) {
        let mut inner = self.runtime.lock().await;
        inner.cancel.cancel();
        inner.paused.store(false, Ordering::SeqCst);
        inner.resume_notify.notify_waiters();
        inner.resume_on_attach = false;
        inner.pause_reason = MediaPauseReason::None;
        if let Some(handle) = inner.task.take() {
            handle.abort();
        }
        inner.status = PlaybackStatus::Stopped;
        inner.position_ms.store(0, Ordering::Relaxed);
    }

    pub async fn pause(&self) {
        let mut inner = self.runtime.lock().await;
        if inner.status != PlaybackStatus::Playing {
            return;
        }
        inner.paused.store(true, Ordering::SeqCst);
        inner.status = PlaybackStatus::Paused;
        inner.pause_reason = MediaPauseReason::User;
        inner.resume_on_attach = false;
    }

    /// 对齐 Go `Play` / `RecoverPlayback(trigger=user)`
    pub async fn recover_playback_for_user(
        &self,
        manager: Arc<ChatManager>,
    ) -> Result<()> {
        self.recover_playback(manager).await
    }

    /// 对齐 Go `ResumeIfInterruptedPause`：仅恢复被中断（非用户主动）暂停的播放
    pub async fn resume_if_interrupted_pause(&self) -> Result<bool> {
        let should = {
            let inner = self.runtime.lock().await;
            inner.status == PlaybackStatus::Paused
                && inner.pause_reason == MediaPauseReason::Interrupt
                && inner.task.is_some()
        };
        if !should {
            return Ok(false);
        }
        let mut inner = self.runtime.lock().await;
        inner.paused.store(false, Ordering::SeqCst);
        inner.status = PlaybackStatus::Playing;
        inner.pause_reason = MediaPauseReason::None;
        inner.resume_on_attach = false;
        inner.resume_notify.notify_waiters();
        Ok(true)
    }

    pub async fn resume(&self, manager: Arc<ChatManager>) -> Result<()> {
        self.recover_playback_for_user(manager).await
    }

    pub async fn suspend(&self) {
        let mut inner = self.runtime.lock().await;
        if inner.status != PlaybackStatus::Playing {
            return;
        }
        inner.paused.store(true, Ordering::SeqCst);
        inner.status = PlaybackStatus::Paused;
        inner.pause_reason = MediaPauseReason::Interrupt;
    }

    pub async fn next(&self, manager: Arc<ChatManager>) -> Result<()> {
        self.jump_agent_relative(manager, 1).await
    }

    pub async fn prev(&self, manager: Arc<ChatManager>) -> Result<()> {
        self.jump_agent_relative(manager, -1).await
    }

    pub async fn play_agent_playlist(
        &self,
        manager: Arc<ChatManager>,
        agent_id: &str,
    ) -> Result<()> {
        let agent_id = agent_id.trim();
        if agent_id.is_empty() {
            return Err(Error::Session("agentID 不能为空".into()));
        }
        let snapshot = shared_media_coordinator()
            .snapshot_agent_playlist(agent_id)
            .await;
        if snapshot.is_empty() {
            return Err(Error::Session("播放列表为空".into()));
        }
        let start_index = {
            let inner = self.runtime.lock().await;
            if inner.mode == PlaybackMode::AgentPlaylist
                && inner.agent_id == agent_id
                && inner.current_index >= 0
            {
                let idx = inner.current_index as usize;
                if idx < snapshot.len() {
                    idx
                } else {
                    0
                }
            } else {
                0
            }
        };
        let tracks: Vec<MediaTrack> = snapshot.into_iter().map(|i| i.track).collect();
        self.replace_playlist_and_play(
            manager,
            tracks,
            start_index,
            PlaybackMode::AgentPlaylist,
            agent_id.to_string(),
        )
        .await
    }

    pub async fn enqueue_current_to_agent(&self, agent_id: &str) -> Result<String> {
        let agent_id = agent_id.trim();
        if agent_id.is_empty() {
            return Err(Error::Session("agentID 不能为空".into()));
        }
        let track = {
            let inner = self.runtime.lock().await;
            let idx = inner.current_index.max(0) as usize;
            if idx >= inner.playlist.len() {
                return Err(Error::Session("当前没有可加入歌单的媒体".into()));
            }
            let track = inner.playlist[idx].clone();
            if !track.can_enqueue_to_agent_playlist() {
                return Err(Error::Session("当前音频来源不支持加入歌单".into()));
            }
            track
        };
        let title = track.title().to_string();
        let (index, _) = shared_media_coordinator()
            .append_agent_track(agent_id, track)
            .await
            .map_err(Error::Session)?;
        let mut inner = self.runtime.lock().await;
        inner.mode = PlaybackMode::AgentPlaylist;
        inner.agent_id = agent_id.to_string();
        inner.current_index = index as i32;
        Ok(title)
    }

    async fn jump_agent_relative(
        &self,
        manager: Arc<ChatManager>,
        delta: i32,
    ) -> Result<()> {
        let agent_id = {
            let inner = self.runtime.lock().await;
            if inner.mode != PlaybackMode::AgentPlaylist {
                return Err(Error::Session(
                    "当前播放未加入智能体播放列表，请先执行 enqueue_current".into(),
                ));
            }
            inner.agent_id.clone()
        };
        let snapshot = shared_media_coordinator()
            .snapshot_agent_playlist(&agent_id)
            .await;
        if snapshot.is_empty() {
            return Err(Error::Session("播放列表为空".into()));
        }
        let current_index = {
            let inner = self.runtime.lock().await;
            inner.current_index
        };
        let len = snapshot.len();
        let mut current = if current_index >= 0 {
            current_index as usize
        } else if delta >= 0 {
            0
        } else {
            len - 1
        };
        if current >= len {
            current = if delta >= 0 { 0 } else { len - 1 };
        }
        let next = (current as i32 + delta).rem_euclid(len as i32) as usize;
        let tracks: Vec<MediaTrack> = snapshot.into_iter().map(|i| i.track).collect();
        self.replace_playlist_and_play(
            manager,
            tracks,
            next,
            PlaybackMode::AgentPlaylist,
            agent_id,
        )
        .await
    }
}

fn track_title(track: &MediaTrack) -> String {
    track.title().to_string()
}

fn track_source_type(track: &MediaTrack) -> MediaSourceType {
    track.source_type()
}

/// 对齐 Go `deviceMediaRuntime.realtimeMcpAudioGateStatus`
pub(crate) fn realtime_mcp_audio_gate_status(inner: &PlaybackRuntime) -> (bool, bool) {
    let source_ok = matches!(
        inner.current_source_type,
        MediaSourceType::McpResource | MediaSourceType::InlineAudio
    );
    if !source_ok {
        return (false, false);
    }
    let has_active = inner.task.is_some();
    let attached = inner.session_attached.load(Ordering::SeqCst);
    let can_control = has_active
        || inner.resume_on_attach
        || inner.status == PlaybackStatus::Paused;
    if !has_active || !attached || inner.status != PlaybackStatus::Playing {
        return (can_control, false);
    }
    (true, !inner.paused.load(Ordering::SeqCst))
}

async fn wait_for_playback_attachment(
    manager: &ChatManager,
    session_attached: &AtomicBool,
    attachment_notify: &Notify,
    cancel: &CancellationToken,
) -> bool {
    loop {
        if cancel.is_cancelled() {
            return false;
        }
        if session_attached.load(Ordering::SeqCst)
            && manager.outbound_tx().is_some()
            && manager
                .session_id()
                .is_some_and(|id| !id.trim().is_empty())
        {
            return true;
        }
        tokio::select! {
            _ = cancel.cancelled() => return false,
            _ = attachment_notify.notified() => {}
        }
    }
}

async fn run_bytes_playback(
    manager: &ChatManager,
    title: &str,
    mp3_data: Vec<u8>,
    audio_format: &str,
    cancel: &CancellationToken,
    paused: &Arc<AtomicBool>,
    resume_notify: &Arc<Notify>,
    position_ms: &Arc<AtomicU64>,
    session_attached: &Arc<AtomicBool>,
    attachment_notify: &Arc<Notify>,
) -> Result<bool> {
    let (raw_tx, raw_rx) = tokio::sync::mpsc::channel(32);
    let feed = tokio::spawn(async move {
        const CHUNK: usize = 16 * 1024;
        for chunk in mp3_data.chunks(CHUNK) {
            if raw_tx.send(chunk.to_vec()).await.is_err() {
                break;
            }
        }
    });
    let ok = run_opus_playback_loop(
        manager,
        title,
        raw_rx,
        audio_format,
        cancel,
        paused,
        resume_notify,
        position_ms,
        session_attached,
        attachment_notify,
    )
    .await?;
    let _ = feed.await;
    Ok(ok)
}

async fn run_url_playback(
    manager: &ChatManager,
    title: &str,
    url: &str,
    cancel: &CancellationToken,
    paused: &Arc<AtomicBool>,
    resume_notify: &Arc<Notify>,
    position_ms: &Arc<AtomicU64>,
    session_attached: &Arc<AtomicBool>,
    attachment_notify: &Arc<Notify>,
) -> Result<bool> {
    let raw_rx = play_music::stream_http_mp3(url, cancel.clone()).await?;
    run_opus_playback_loop(
        manager,
        title,
        raw_rx,
        "mp3",
        cancel,
        paused,
        resume_notify,
        position_ms,
        session_attached,
        attachment_notify,
    )
    .await
}

async fn run_mcp_resource_playback(
    manager: &ChatManager,
    title: &str,
    tool_name: &str,
    resource_uri: &str,
    read_args: serde_json::Value,
    cancel: &CancellationToken,
    paused: &Arc<AtomicBool>,
    resume_notify: &Arc<Notify>,
    position_ms: &Arc<AtomicU64>,
    session_attached: &Arc<AtomicBool>,
    attachment_notify: &Arc<Notify>,
) -> Result<bool> {
    let raw_rx = mcp_tool_media::stream_mcp_resource(
        Arc::clone(manager.mcp_manager()),
        tool_name,
        resource_uri,
        read_args,
        cancel.clone(),
    )
    .await?;
    run_opus_playback_loop(
        manager,
        title,
        raw_rx,
        "mp3",
        cancel,
        paused,
        resume_notify,
        position_ms,
        session_attached,
        attachment_notify,
    )
    .await
}

async fn run_opus_playback_loop(
    manager: &ChatManager,
    title: &str,
    raw_rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
    audio_format: &str,
    cancel: &CancellationToken,
    paused: &Arc<AtomicBool>,
    resume_notify: &Arc<Notify>,
    position_ms: &Arc<AtomicU64>,
    session_attached: &Arc<AtomicBool>,
    attachment_notify: &Arc<Notify>,
) -> Result<bool> {
    let audio_params = session_audio_params(manager);
    let session_id = manager.session_id();
    let play_text = if title.trim().is_empty() {
        "音乐播放".to_string()
    } else {
        title.to_string()
    };

    if !wait_for_playback_attachment(
        manager,
        session_attached,
        attachment_notify,
        cancel,
    )
    .await
    {
        return Ok(false);
    }

    send_command(manager, ServerMessage::tts(message::START, session_id.clone()));
    send_command(
        manager,
        ServerMessage::tts_sentence(
            &play_text,
            message::SENTENCE_START,
            session_id.clone(),
        ),
    );

    let mut opus_rx = wrap_tts_audio_stream(
        raw_rx,
        audio_format,
        audio_params.sample_rate,
        audio_params.channels,
        audio_params.frame_duration,
    );

    let started = Instant::now();
    let mut completed = true;

    while let Some(frame) = opus_rx.recv().await {
        if cancel.is_cancelled() {
            completed = false;
            break;
        }
        while paused.load(Ordering::SeqCst) && !cancel.is_cancelled() {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = resume_notify.notified() => break,
            }
        }
        if cancel.is_cancelled() {
            completed = false;
            break;
        }
        if !wait_for_playback_attachment(
            manager,
            session_attached,
            attachment_notify,
            cancel,
        )
        .await
        {
            completed = false;
            break;
        }
        if !send_audio(manager, &frame) {
            completed = false;
            break;
        }
        position_ms.store(started.elapsed().as_millis() as u64, Ordering::Relaxed);
    }

    if !cancel.is_cancelled() {
        send_command(
            manager,
            ServerMessage::tts_sentence(
                &play_text,
                message::SENTENCE_END,
                session_id.clone(),
            ),
        );
        send_command(manager, ServerMessage::tts(message::STOP, session_id));
    }

    if let Some(tts) = manager.tts_manager().await {
        tts.end_exclusive_media_playback();
    }

    Ok(completed)
}

fn session_audio_params(manager: &ChatManager) -> AudioParams {
    manager
        .session
        .try_lock()
        .ok()
        .and_then(|guard| guard.as_ref().map(|s| s.audio_params().clone()))
        .unwrap_or_default()
}

fn send_command(manager: &ChatManager, msg: ServerMessage) -> bool {
    let Ok(data) = serde_json::to_vec(&msg) else {
        return false;
    };
    manager.send_outbound_command(data)
}

fn send_audio(manager: &ChatManager, frame: &[u8]) -> bool {
    let proto = manager.binary_protocol_version();
    let packed = xiaozhi_protocol::pack_device_audio(frame, proto);
    manager.send_outbound_audio(packed)
}

pub fn control_result(state: &MediaPlayerState, action: &str) -> serde_json::Value {
    json!({
        "action": action,
        "status": state.status.as_str(),
        "current_title": state.current_title,
        "current_index": state.current_index,
        "playlist_length": state.playlist_length,
        "position_ms": state.position_ms,
        "silence_response": true,
    })
}

pub fn control_result_with_added(
    state: &MediaPlayerState,
    action: &str,
    added_title: &str,
) -> serde_json::Value {
    let mut v = control_result(state, action);
    if let Some(obj) = v.as_object_mut() {
        obj.insert("added_title".to_string(), json!(added_title));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn realtime_gate_status_matches_go_cases() {
        let mut rt = PlaybackRuntime::default();
        rt.current_source_type = MediaSourceType::McpResource;
        rt.status = PlaybackStatus::Playing;
        rt.task = Some(tokio::spawn(async {}));
        rt.session_attached
            .store(true, Ordering::SeqCst);
        let (can, gate) = realtime_mcp_audio_gate_status(&rt);
        assert!(can && gate, "MCP 播放中应可控制且门控 ASR");

        rt.status = PlaybackStatus::Paused;
        let (can, gate) = realtime_mcp_audio_gate_status(&rt);
        assert!(can && !gate, "暂停时应可控制但不门控普通 ASR");

        rt.status = PlaybackStatus::Playing;
        rt.session_attached
            .store(false, Ordering::SeqCst);
        let (can, gate) = realtime_mcp_audio_gate_status(&rt);
        assert!(can && !gate, "无 attachment 时不应门控 ASR");

        rt.session_attached.store(true, Ordering::SeqCst);
        rt.current_source_type = MediaSourceType::HttpUrl;
        let (can, gate) = realtime_mcp_audio_gate_status(&rt);
        assert!(!can && !gate, "HTTP 音乐不应进入 realtime MCP 门控");

        rt.current_source_type = MediaSourceType::McpResource;
        rt.status = PlaybackStatus::Stopped;
        rt.task = None;
        rt.resume_on_attach = true;
        let (can, gate) = realtime_mcp_audio_gate_status(&rt);
        assert!(can && !gate, "resume_on_attach 应有控制上下文但不门控");
    }

    #[tokio::test]
    async fn device_runtime_shared_across_session_players() {
        let rt1 = get_or_create_device_runtime("device-shared");
        let rt2 = get_or_create_device_runtime("device-shared");
        let rt3 = get_or_create_device_runtime("device-other");
        assert!(Arc::ptr_eq(&rt1, &rt2));
        assert!(!Arc::ptr_eq(&rt1, &rt3));

        {
            let mut inner = rt1.lock().await;
            inner.current_title = "shared-title".into();
        }
        assert_eq!(rt2.lock().await.current_title, "shared-title");
    }
}
