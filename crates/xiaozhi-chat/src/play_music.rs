//! 音乐搜索与 HTTP 流（对齐 Go `getMusicURL` + `PlayMusicStream`）

use std::time::Duration;

use reqwest::Client;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use xiaozhi_core::{Error, Result};

const MUSIC_SEARCH_URL: &str = "https://music.txqq.pro/";
/// 按优先级尝试的音源（migu 对部分热门曲目已不可用，netease 更稳定）
const MUSIC_SOURCE_TYPES: &[&str] = &["netease", "migu"];

fn music_http_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| Client::new())
}

fn normalize_music_search_name(name: &str) -> String {
    let name = name.trim();
    if let Some(start) = name.find('《') {
        if let Some(rel_end) = name[start..].find('》') {
            let inner = name[start + '《'.len_utf8()..start + rel_end].trim();
            if !inner.is_empty() {
                return inner.to_string();
            }
        }
    }
    name.to_string()
}

fn pick_music_item<'a>(items: &'a [Value], music_name: &str) -> Option<&'a Value> {
    let needle = music_name.trim();
    let mut fallback: Option<&'a Value> = None;
    for item in items {
        let url = item.get("url").and_then(|v| v.as_str()).unwrap_or("").trim();
        if url.is_empty() {
            continue;
        }
        if fallback.is_none() {
            fallback = Some(item);
        }
        if item
            .get("title")
            .and_then(|v| v.as_str())
            .is_some_and(|title| title.contains(needle))
        {
            return Some(item);
        }
    }
    fallback
}

async fn search_music_from_source(
    client: &Client,
    music_name: &str,
    source_type: &str,
) -> Result<(String, String)> {
    let body = format!(
        "input={}&filter=name&type={}&page=1",
        urlencoding::encode(music_name),
        urlencoding::encode(source_type)
    );

    let resp = client
        .post(MUSIC_SEARCH_URL)
        .header("Accept", "application/json, text/javascript, */*; q=0.01")
        .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
        .header("Content-Type", "application/x-www-form-urlencoded; charset=UTF-8")
        .header("Origin", "https://music.txqq.pro")
        .header("Referer", "https://music.txqq.pro/")
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36",
        )
        .header("X-Requested-With", "XMLHttpRequest")
        .body(body)
        .send()
        .await
        .map_err(|e| Error::Session(format!("音乐搜索请求失败: {e}")))?;

    if !resp.status().is_success() {
        return Err(Error::Session(format!(
            "音乐搜索失败，状态码: {}",
            resp.status()
        )));
    }

    let payload: Value = resp
        .json()
        .await
        .map_err(|e| Error::Session(format!("解析音乐搜索响应失败: {e}")))?;

    let code = payload.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
    if code != 200 {
        let msg = payload
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if msg.is_empty() {
            return Err(Error::Session(format!("音乐搜索 API 错误: code={code}")));
        }
        return Err(Error::Session(msg.to_string()));
    }

    let items = payload
        .get("data")
        .and_then(|v| v.as_array())
        .map(|v| v.as_slice())
        .unwrap_or(&[]);
    let item = pick_music_item(items, music_name)
        .ok_or_else(|| Error::Session(format!("未找到音乐: {music_name}")))?;

    let url = item
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let title = item
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or(music_name)
        .trim()
        .to_string();

    Ok((url, title))
}

pub async fn search_music(music_name: &str) -> Result<(String, String)> {
    let music_name = normalize_music_search_name(music_name);
    if music_name.is_empty() {
        return Err(Error::Session("音乐名称不能为空".into()));
    }

    let client = music_http_client();
    let mut last_err: Option<Error> = None;
    for source_type in MUSIC_SOURCE_TYPES {
        match search_music_from_source(&client, &music_name, source_type).await {
            Ok(found) => return Ok(found),
            Err(err) => {
                tracing::debug!(
                    music_name = %music_name,
                    source = source_type,
                    error = %err,
                    "音乐搜索音源未命中，尝试下一个"
                );
                last_err = Some(err);
            }
        }
    }

    Err(last_err.unwrap_or_else(|| Error::Session(format!("未找到音乐: {music_name}"))))
}

pub async fn fetch_music_bytes(url: &str) -> Result<Vec<u8>> {
    let url = url.trim();
    if url.is_empty() {
        return Err(Error::Session("音乐 URL 为空".into()));
    }
    let client = music_http_client();
    let resp = client
        .get(url)
        .header("Accept", "audio/*")
        .header("User-Agent", "MusicPlayer/1.0")
        .send()
        .await
        .map_err(|e| Error::Session(format!("下载音乐失败: {e}")))?;

    if !resp.status().is_success() {
        return Err(Error::Session(format!(
            "下载音乐失败，状态码: {}",
            resp.status()
        )));
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| Error::Session(format!("读取音乐数据失败: {e}")))?;

    if bytes.is_empty() {
        return Err(Error::Session("音乐数据为空".into()));
    }

    Ok(bytes.to_vec())
}

pub async fn resolve_and_fetch(music_name: &str) -> Result<(Vec<u8>, String)> {
    let (url, title) = search_music(music_name).await?;
    let bytes = fetch_music_bytes(&url).await?;
    Ok((bytes, title))
}

/// HTTP 流式拉取 MP3，边下边播（对齐 Go `PlayMusicStream`）
pub async fn stream_http_mp3(
    url: &str,
    cancel: CancellationToken,
) -> Result<mpsc::Receiver<Vec<u8>>> {
    let url = url.trim().to_string();
    if url.is_empty() {
        return Err(Error::Session("音乐 URL 为空".into()));
    }

    let (tx, rx) = mpsc::channel(64);
    tokio::spawn(async move {
        let client = music_http_client();
        let resp = match client
            .get(&url)
            .header("Accept", "audio/*")
            .header("User-Agent", "MusicPlayer/1.0")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("音乐流请求失败: {e}");
                return;
            }
        };
        if !resp.status().is_success() {
            tracing::warn!("音乐流 HTTP 状态: {}", resp.status());
            return;
        }
        let mut stream = resp.bytes_stream();
        use futures_util::StreamExt;
        loop {
            if cancel.is_cancelled() {
                break;
            }
            let chunk = tokio::select! {
                _ = cancel.cancelled() => break,
                item = stream.next() => item,
            };
            let Some(item) = chunk else { break };
            match item {
                Ok(bytes) if !bytes.is_empty() => {
                    if tx.send(bytes.to_vec()).await.is_err() {
                        break;
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!("音乐流读取失败: {e}");
                    break;
                }
            }
        }
    });
    Ok(rx)
}

pub fn normalize_music_playback_action(raw: &str) -> Option<&'static str> {
    match raw.trim().to_lowercase().as_str() {
        "resume" | "play" | "continue" | "unpause" => Some("resume"),
        "pause" => Some("pause"),
        "stop" => Some("stop"),
        "prev" | "previous" => Some("prev"),
        "next" => Some("next"),
        "play_playlist" | "play_agent_playlist" | "play_playlist_songs" | "playlist" => {
            Some("play_playlist")
        }
        "enqueue_current" | "append_current" | "add_current_to_playlist" => {
            Some("enqueue_current")
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_playback_actions() {
        assert_eq!(normalize_music_playback_action("Resume"), Some("resume"));
        assert_eq!(normalize_music_playback_action("PAUSE"), Some("pause"));
        assert_eq!(normalize_music_playback_action("next"), Some("next"));
        assert_eq!(normalize_music_playback_action("unknown"), None);
    }

    #[test]
    fn normalizes_music_search_name() {
        assert_eq!(normalize_music_search_name("晴天"), "晴天");
        assert_eq!(
            normalize_music_search_name("周杰伦的《晴天》"),
            "晴天"
        );
        assert_eq!(normalize_music_search_name("  《七里香》 "), "七里香");
    }
}
