use std::sync::Arc;

use axum::{
    extract::{Multipart, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use xiaozhi_config::AppConfig;
use xiaozhi_llm::create_llm;

use crate::shared_config::SharedAppConfig;

pub struct VisionState {
    pub config: SharedAppConfig,
}

pub async fn vision_handler(
    State(state): State<Arc<VisionState>>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Response {
    let cfg = state.config.read().await;

    let device_id = headers
        .get("Device-Id")
        .or(headers.get("device-id"))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if device_id.is_empty() {
        return text_err(StatusCode::BAD_REQUEST, "缺少Device-Id");
    }

    if cfg.vision.enable_auth {
        let auth = headers
            .get("Authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let token = auth.strip_prefix("Bearer ").unwrap_or(auth).trim();
        if token.is_empty() {
            return text_err(StatusCode::BAD_REQUEST, "缺少Authorization");
        }
    }

    let mut question = String::new();
    let mut file_bytes: Option<Vec<u8>> = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "question" => {
                if let Ok(text) = field.text().await {
                    question = text;
                }
            }
            "file" => {
                if let Ok(bytes) = field.bytes().await {
                    file_bytes = Some(bytes.to_vec());
                }
            }
            _ => {}
        }
    }

    if question.trim().is_empty() {
        return text_err(StatusCode::BAD_REQUEST, "缺少question参数");
    }
    let file = match file_bytes {
        Some(b) if !b.is_empty() => b,
        _ => return text_err(StatusCode::BAD_REQUEST, "缺少file参数或文件读取失败"),
    };

    let mime = infer_image_mime(&file);
    tracing::info!(
        "图片识别 device={device_id} size={} question={}",
        file.len(),
        question
    );

    match run_vision(&cfg, &file, &question, mime).await {
        Ok(text) => {
            tracing::info!("图片识别成功 device={device_id} len={}", text.len());
            (
                StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                text,
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("图片识别失败 device={device_id}: {e}");
            text_err(StatusCode::INTERNAL_SERVER_ERROR, "图片识别失败")
        }
    }
}

async fn run_vision(
    config: &AppConfig,
    file: &[u8],
    question: &str,
    mime_type: &str,
) -> Result<String, String> {
    let section = &config.vision.vllm;
    let provider = section.provider.trim();
    if provider.is_empty() {
        return Err("未配置 vision.vllm.provider".into());
    }
    let provider_cfg = section
        .active_config()
        .ok_or_else(|| format!("未找到视觉模型配置: {provider}"))?;

    let llm = create_llm(provider, provider_cfg).map_err(|e| e.to_string())?;
    llm.response_with_vllm(file, question, mime_type)
        .await
        .map_err(|e| e.to_string())
}

fn infer_image_mime(data: &[u8]) -> &'static str {
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        "image/png"
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        "image/gif"
    } else if data.starts_with(b"RIFF") && data.len() > 12 && &data[8..12] == b"WEBP" {
        "image/webp"
    } else {
        "image/jpeg"
    }
}

fn text_err(status: StatusCode, msg: &str) -> Response {
    (status, msg.to_string()).into_response()
}

