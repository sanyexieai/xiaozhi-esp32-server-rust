use std::path::PathBuf;

use reqwest::Client;
use tokio::sync::mpsc;
use xiaozhi_core::Result;

pub struct MusicPlayer {
    music_dir: PathBuf,
    client: Client,
}

impl MusicPlayer {
    pub fn new(music_dir: PathBuf) -> Self {
        Self {
            music_dir,
            client: Client::new(),
        }
    }

    pub async fn search_local(&self, song_name: &str) -> Result<Vec<PathBuf>> {
        let mut results = Vec::new();
        if !self.music_dir.exists() {
            return Ok(results);
        }
        for entry in std::fs::read_dir(&self.music_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.contains(song_name))
                .unwrap_or(false)
            {
                results.push(path);
            }
        }
        Ok(results)
    }

    pub async fn stream_audio(
        &self,
        path: &PathBuf,
        chunk_size: usize,
    ) -> Result<mpsc::Receiver<Vec<u8>>> {
        let (tx, rx) = mpsc::channel(32);
        let data = tokio::fs::read(path).await?;
        tokio::spawn(async move {
            for chunk in data.chunks(chunk_size) {
                if tx.send(chunk.to_vec()).await.is_err() {
                    break;
                }
            }
        });
        Ok(rx)
    }
}
