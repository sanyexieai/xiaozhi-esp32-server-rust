use std::collections::HashSet;
use std::path::Path;

use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::json;

pub const KNOWLEDGE_UPLOAD_CONTENT_PREFIX: &str = "__KB_FILE_UPLOAD_V1__:";
pub const KNOWLEDGE_DOCUMENT_UPLOAD_MAX_BYTES: usize = 2 * 1024 * 1024;

pub fn sanitize_upload_file_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return "document.txt".to_string();
    }
    Path::new(trimmed)
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("document.txt")
        .to_string()
}

pub fn build_upload_document_name(input_name: &str, file_name: &str) -> String {
    let name = input_name.trim();
    if !name.is_empty() {
        return truncate_runes(name, 200);
    }
    let from_file = file_name.trim();
    if !from_file.is_empty() {
        return truncate_runes(from_file, 200);
    }
    "上传文档".to_string()
}

fn truncate_runes(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

fn dify_ext() -> &'static HashSet<&'static str> {
    static SET: std::sync::OnceLock<HashSet<&'static str>> = std::sync::OnceLock::new();
    SET.get_or_init(|| {
        [
            ".txt", ".md", ".markdown", ".pdf", ".html", ".htm", ".xlsx", ".xls", ".docx",
            ".csv", ".eml", ".msg", ".pptx", ".ppt", ".xml", ".epub",
        ]
        .into_iter()
        .collect()
    })
}

fn ragflow_ext() -> &'static HashSet<&'static str> {
    static SET: std::sync::OnceLock<HashSet<&'static str>> = std::sync::OnceLock::new();
    SET.get_or_init(|| {
        [
            ".txt", ".text", ".md", ".markdown", ".pdf", ".doc", ".docx", ".ppt", ".pptx",
            ".xls", ".xlsx", ".wps", ".json", ".csv", ".log", ".xml", ".html", ".htm", ".yml",
            ".yaml", ".rtf", ".sql", ".ini", ".jpg", ".jpeg", ".png", ".gif", ".bmp", ".webp",
            ".tif", ".tiff", ".eml", ".msg",
        ]
        .into_iter()
        .collect()
    })
}

pub fn allowed_extensions_for(provider: &str) -> (&'static HashSet<&'static str>, &'static str) {
    match provider.trim().to_lowercase().as_str() {
        "dify" => (
            dify_ext(),
            "txt, md, markdown, pdf, html, htm, xlsx, xls, docx, csv, eml, msg, pptx, ppt, xml, epub",
        ),
        "weknora" => (
            ragflow_ext(),
            "txt, text, md, markdown, pdf, doc, docx, ppt, pptx, xls, xlsx, wps, json, csv, log, xml, html, htm, yml, yaml, rtf, sql, ini, jpg, jpeg, png, gif, bmp, webp, tif, tiff, eml, msg",
        ),
        _ => (
            ragflow_ext(),
            "txt, text, md, markdown, pdf, doc, docx, ppt, pptx, xls, xlsx, wps, json, csv, log, xml, html, htm, yml, yaml, rtf, sql, ini, jpg, jpeg, png, gif, bmp, webp, tif, tiff, eml, msg",
        ),
    }
}

pub fn validate_upload_file(provider: &str, file_name: &str, data: &[u8]) -> Result<(), String> {
    if data.is_empty() {
        return Err("上传文件为空".to_string());
    }
    if data.len() > KNOWLEDGE_DOCUMENT_UPLOAD_MAX_BYTES {
        return Err(format!(
            "文件过大，最大支持 {}MB",
            KNOWLEDGE_DOCUMENT_UPLOAD_MAX_BYTES / (1024 * 1024)
        ));
    }
    let safe_name = sanitize_upload_file_name(file_name);
    let ext = Path::new(&safe_name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e.to_lowercase()))
        .unwrap_or_default();
    if ext.is_empty() {
        let (_, supported) = allowed_extensions_for(provider);
        return Err(format!(
            "文件类型不支持，缺少扩展名，{}支持格式: {}",
            provider.to_uppercase(),
            supported
        ));
    }
    let (allowed, supported) = allowed_extensions_for(provider);
    if !allowed.contains(ext.as_str()) {
        return Err(format!(
            "文件类型不支持，{}支持格式: {}",
            provider.to_uppercase(),
            supported
        ));
    }
    Ok(())
}

pub fn decode_upload_content(content: &str) -> Result<(String, Vec<u8>, bool), String> {
    let raw = content.trim();
    if !raw.starts_with(KNOWLEDGE_UPLOAD_CONTENT_PREFIX) {
        return Ok((String::new(), Vec::new(), false));
    }
    let json_part = raw
        .trim_start_matches(KNOWLEDGE_UPLOAD_CONTENT_PREFIX)
        .trim();
    if json_part.is_empty() {
        return Err("上传文件元数据为空".to_string());
    }
    let payload: serde_json::Value =
        serde_json::from_str(json_part).map_err(|e| format!("解析上传文件元数据失败: {e}"))?;
    let file_name = sanitize_upload_file_name(
        payload
            .get("file_name")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
    );
    let b64 = payload
        .get("content_base64")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if b64.is_empty() {
        return Err("上传文件内容为空".to_string());
    }
    let file_data = STANDARD
        .decode(b64)
        .map_err(|e| format!("解析上传文件内容失败: {e}"))?;
    if file_data.is_empty() {
        return Err("上传文件内容为空".to_string());
    }
    Ok((file_name, file_data, true))
}

pub fn encode_upload_content(file_name: &str, file_data: &[u8]) -> Result<String, String> {
    let payload = json!({
        "file_name": sanitize_upload_file_name(file_name),
        "content_base64": STANDARD.encode(file_data),
    });
    serde_json::to_string(&payload)
        .map(|s| format!("{KNOWLEDGE_UPLOAD_CONTENT_PREFIX}{s}"))
        .map_err(|e| format!("编码上传文件失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_and_validates_txt_upload() {
        let data = b"hello";
        validate_upload_file("dify", "note.txt", data).unwrap();
        let encoded = encode_upload_content("note.txt", data).unwrap();
        assert!(encoded.starts_with(KNOWLEDGE_UPLOAD_CONTENT_PREFIX));
    }
}
