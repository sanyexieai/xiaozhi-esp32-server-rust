//! 跨设备共享的智能体歌单（对齐 Go `mediaPlaybackCoordinator` / `agentMediaPlaylist`）

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::Mutex;

use crate::media_player::MediaTrack;

#[derive(Debug, Clone)]
pub(crate) struct AgentPlaylistItem {
    pub(crate) track: MediaTrack,
    pub added_at_ms: i64,
}

#[derive(Default)]
struct CoordinatorInner {
    agent_playlists: HashMap<String, Vec<AgentPlaylistItem>>,
}

pub(crate) struct MediaPlaybackCoordinator {
    inner: Mutex<CoordinatorInner>,
}

impl MediaPlaybackCoordinator {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(CoordinatorInner::default()),
        }
    }

    pub async fn snapshot_agent_playlist(&self, agent_id: &str) -> Vec<AgentPlaylistItem> {
        let agent_id = agent_id.trim();
        if agent_id.is_empty() {
            return Vec::new();
        }
        let inner = self.inner.lock().await;
        inner
            .agent_playlists
            .get(agent_id)
            .cloned()
            .unwrap_or_default()
    }

    pub async fn agent_playlist_len(&self, agent_id: &str) -> usize {
        self.snapshot_agent_playlist(agent_id).await.len()
    }

    pub async fn append_agent_track(
        &self,
        agent_id: &str,
        track: MediaTrack,
    ) -> Result<(usize, usize), String> {
        let agent_id = agent_id.trim();
        if agent_id.is_empty() {
            return Err("agentID 不能为空".into());
        }
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let item = AgentPlaylistItem {
            track,
            added_at_ms: now_ms,
        };
        let mut inner = self.inner.lock().await;
        let list = inner.agent_playlists.entry(agent_id.to_string()).or_default();
        list.push(item);
        let index = list.len() - 1;
        Ok((index, list.len()))
    }
}

static SHARED_COORDINATOR: std::sync::OnceLock<Arc<MediaPlaybackCoordinator>> =
    std::sync::OnceLock::new();

pub(crate) fn shared_media_coordinator() -> Arc<MediaPlaybackCoordinator> {
    SHARED_COORDINATOR
        .get_or_init(|| Arc::new(MediaPlaybackCoordinator::new()))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_player::MediaTrack;

    #[tokio::test]
    async fn appends_and_snapshots_agent_playlist() {
        let coord = MediaPlaybackCoordinator::new();
        let track = MediaTrack::Url {
            title: "曲1".into(),
            url: "https://example.com/1.mp3".into(),
        };
        let (idx, len) = coord.append_agent_track("agent-1", track).await.unwrap();
        assert_eq!(idx, 0);
        assert_eq!(len, 1);
        let snap = coord.snapshot_agent_playlist("agent-1").await;
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].track.title(), "曲1");
    }
}
