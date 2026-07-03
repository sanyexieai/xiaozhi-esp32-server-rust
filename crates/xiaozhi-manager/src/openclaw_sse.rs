use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::body::Body;
use axum::http::{header, StatusCode};
use axum::response::Response;
use futures_util::Stream;
use serde_json::{json, Value};
use tokio::sync::mpsc;

pub fn wants_openclaw_sse(stream: Option<&str>, accept: Option<&str>) -> bool {
    if let Some(stream) = stream {
        let normalized = stream.trim().to_lowercase();
        if matches!(normalized.as_str(), "1" | "true" | "yes") {
            return true;
        }
    }
    accept
        .unwrap_or("")
        .to_lowercase()
        .contains("text/event-stream")
}

pub fn format_sse(event: &str, payload: &Value) -> String {
    let data = serde_json::to_string(payload).unwrap_or_else(|_| "{}".to_string());
    if event.trim().is_empty() {
        format!("data: {data}\n\n")
    } else {
        format!("event: {}\ndata: {data}\n\n", event.trim())
    }
}

struct SseStream {
    rx: mpsc::UnboundedReceiver<String>,
}

impl Stream for SseStream {
    type Item = Result<axum::body::Bytes, Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.rx.poll_recv(cx) {
            Poll::Ready(Some(chunk)) => Poll::Ready(Some(Ok(axum::body::Bytes::from(chunk)))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

pub fn sse_response(rx: mpsc::UnboundedReceiver<String>) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::CONNECTION, "keep-alive")
        .header("X-Accel-Buffering", "no")
        .body(Body::from_stream(SseStream { rx }))
        .unwrap()
}

pub fn ws_response_sse_chunk(resp: &crate::ws::WsResponse) -> String {
    let mut payload = json!({ "status": resp.status });
    if !resp.body.is_null() {
        payload["data"] = resp.body.clone();
    }
    if !resp.error.is_empty() {
        payload["error"] = json!(resp.error);
    }
    match resp.status {
        206 => format_sse("chunk", &payload),
        200 => format_sse("result", &payload),
        _ => format_sse("error", &payload),
    }
}

pub fn sse_done(ok: bool, data: Option<Value>) -> String {
    let mut payload = json!({ "ok": ok });
    if let Some(data) = data {
        payload["data"] = data;
    }
    format_sse("done", &payload)
}

pub fn sse_error(message: impl Into<String>) -> String {
    format_sse("error", &json!({ "error": message.into() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_stream_query_and_accept() {
        assert!(wants_openclaw_sse(Some("1"), None));
        assert!(wants_openclaw_sse(Some("true"), None));
        assert!(wants_openclaw_sse(None, Some("text/event-stream")));
        assert!(!wants_openclaw_sse(None, Some("application/json")));
        assert!(!wants_openclaw_sse(Some("0"), None));
    }

    #[test]
    fn maps_ws_chunk_to_sse_events() {
        use crate::ws::WsResponse;

        let chunk = ws_response_sse_chunk(&WsResponse {
            id: "r1".into(),
            status: 206,
            headers: Default::default(),
            body: json!({ "chunk": "hi", "reply": "hi" }),
            error: String::new(),
        });
        assert!(chunk.contains("event: chunk"));
        assert!(chunk.contains("\"chunk\":\"hi\""));

        let result = ws_response_sse_chunk(&WsResponse {
            id: "r1".into(),
            status: 200,
            headers: Default::default(),
            body: json!({ "reply": "done", "latency_ms": 12 }),
            error: String::new(),
        });
        assert!(result.contains("event: result"));
        assert!(result.contains("\"reply\":\"done\""));
    }
}
