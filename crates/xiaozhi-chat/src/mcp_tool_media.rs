//! MCP 工具返回的音频 / ResourceLink 处理（对齐 Go `tool.go` handleAudioContent / handleResourceLink）

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use xiaozhi_core::Result;
use xiaozhi_mcp::McpManager;

pub const MCP_READ_RESOURCE_PAGE_SIZE: usize = 100 * 1024;
pub const MCP_READ_RESOURCE_STREAM_DONE: &str = "[DONE]";

#[derive(Debug, Clone)]
pub struct ParsedResourceLink {
    pub title: String,
    pub uri: String,
    pub description: String,
    pub mime_type: String,
}

#[derive(Debug, Clone)]
pub struct ParsedAudioContent {
    pub title: String,
    pub data: Vec<u8>,
    pub audio_format: String,
}

/// 从 CallToolResult 提取给 LLM 的文本摘要
pub fn tool_result_display_text(result: &Value) -> String {
    if let Some(items) = result.get("content").and_then(|v| v.as_array()) {
        let texts: Vec<String> = items
            .iter()
            .filter_map(|item| {
                if item.get("type")?.as_str()? == "text" {
                    item.get("text")?.as_str().map(String::from)
                } else {
                    None
                }
            })
            .collect();
        if !texts.is_empty() {
            return texts.join("\n");
        }
    }
    result.to_string()
}

pub fn parse_audio_content(item: &Value, tool_name: &str) -> Option<ParsedAudioContent> {
    if item.get("type")?.as_str()? != "audio" {
        return None;
    }
    let b64 = item.get("data")?.as_str()?;
    let data = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .ok()?;
    if data.is_empty() {
        return None;
    }
    let mime = item
        .get("mimeType")
        .or_else(|| item.get("mime_type"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let mut title = tool_name.trim().to_string();
    if title.is_empty() || title == "执行成功" {
        title = "工具音频".to_string();
    }
    Some(ParsedAudioContent {
        title,
        data,
        audio_format: audio_format_from_mime(mime).to_string(),
    })
}

pub fn parse_resource_link(item: &Value) -> Option<ParsedResourceLink> {
    if item.get("type")?.as_str()? != "resource_link" {
        return None;
    }
    let uri = item.get("uri")?.as_str()?.trim().to_string();
    let description = item
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let mut title = item
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if title.is_empty() {
        title = description.clone();
    }
    if title.is_empty() {
        title = uri.clone();
    }
    let mime_type = item
        .get("mimeType")
        .or_else(|| item.get("mime_type"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Some(ParsedResourceLink {
        title,
        uri,
        description,
        mime_type,
    })
}

pub fn is_direct_audio_url(url: &str) -> bool {
    let url = url.trim();
    url.starts_with("http://") || url.starts_with("https://")
}

pub fn audio_format_from_mime(mime: &str) -> &'static str {
    match mime.trim().to_ascii_lowercase().as_str() {
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
        "audio/ogg" | "audio/opus" => "ogg",
        _ => "mp3",
    }
}

/// 分页读取 MCP Resource 并流式输出原始音频字节
pub async fn stream_mcp_resource(
    mcp_manager: Arc<McpManager>,
    tool_name: &str,
    uri: &str,
    read_args_base: Value,
    cancel: CancellationToken,
) -> Result<mpsc::Receiver<Vec<u8>>> {
    let (tx, rx) = mpsc::channel(64);
    let tool_name = tool_name.to_string();
    let uri = uri.to_string();
    tokio::spawn(async move {
        let mut start = 0usize;
        loop {
            if cancel.is_cancelled() {
                break;
            }
            let mut read_args = match read_args_base.as_object() {
                Some(map) => Value::Object(map.clone()),
                None => json!({}),
            };
            if let Some(obj) = read_args.as_object_mut() {
                obj.insert("start".into(), json!(start));
                obj.insert("end".into(), json!(start + MCP_READ_RESOURCE_PAGE_SIZE));
            }

            let page = match mcp_manager
                .read_global_resource(&tool_name, &uri, read_args)
                .await
            {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("MCP 资源读取失败: {e}");
                    break;
                }
            };

            let Some(items) = page.get("contents").and_then(|v| v.as_array()) else {
                break;
            };
            if items.is_empty() {
                break;
            }

            let mut has_data = false;
            for content in items {
                let blob = content
                    .get("blob")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if blob.is_empty() {
                    continue;
                }
                let raw = match base64::engine::general_purpose::STANDARD.decode(blob) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!("解码 MCP 音频 blob 失败: {e}");
                        break;
                    }
                };
                if raw.is_empty() {
                    continue;
                }
                if raw == MCP_READ_RESOURCE_STREAM_DONE.as_bytes() {
                    return;
                }
                if tx.send(raw.clone()).await.is_err() {
                    return;
                }
                has_data = true;
                if raw.len() < MCP_READ_RESOURCE_PAGE_SIZE {
                    return;
                }
            }

            if !has_data {
                break;
            }
            start += MCP_READ_RESOURCE_PAGE_SIZE;
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    });
    Ok(rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_resource_link() {
        let item = json!({
            "type": "resource_link",
            "uri": "music://track/1",
            "name": "测试歌曲",
            "description": "https://example.com/a.mp3"
        });
        let link = parse_resource_link(&item).unwrap();
        assert_eq!(link.title, "测试歌曲");
        assert!(is_direct_audio_url(&link.description));
    }

    #[test]
    fn picks_audio_format_from_mime() {
        assert_eq!(audio_format_from_mime("audio/wav"), "wav");
        assert_eq!(audio_format_from_mime("audio/mpeg"), "mp3");
    }
}
