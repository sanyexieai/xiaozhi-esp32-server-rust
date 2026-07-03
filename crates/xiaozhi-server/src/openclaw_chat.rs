use std::time::{Duration, Instant};

use serde_json::json;
use xiaozhi_openclaw::{
    openclaw_test_device_id, parse_openclaw_timeout_ms, parse_stream_events, OpenClawManager,
    OPENCLAW_CHAT_TEST_SESSION_ID,
};

use crate::bridge::{WsRequest, WsResponse};

pub async fn handle_openclaw_chat<F, Fut>(
    openclaw: &OpenClawManager,
    req: WsRequest,
    mut send: F,
) where
    F: FnMut(WsResponse) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let agent_id = req
        .body
        .get("agent_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let message = req
        .body
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let session_id = req
        .body
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| OPENCLAW_CHAT_TEST_SESSION_ID.to_string());
    let timeout_ms = parse_openclaw_timeout_ms(req.body.get("timeout_ms"));
    let stream_events = parse_stream_events(req.body.get("stream_events"));

    if agent_id.is_empty() {
        send(WsResponse::err(req.id.clone(), 400, "missing agent_id")).await;
        return;
    }
    if message.is_empty() {
        send(WsResponse::err(req.id.clone(), 400, "missing message")).await;
        return;
    }
    if openclaw.get_agent_session(&agent_id).is_none() {
        send(WsResponse::err(
            req.id.clone(),
            409,
            format!("openclaw session not connected for agent {agent_id}"),
        ))
        .await;
        return;
    }

    let test_device_id = openclaw_test_device_id(&agent_id);
    openclaw.clear_offline_messages(&test_device_id);

    let start = Instant::now();
    let message_id = match openclaw.send_message(&agent_id, &test_device_id, &message, &session_id)
    {
        Ok(id) => id,
        Err(e) => {
            let msg = e.to_string().to_lowercase();
            if msg.contains("未连接") || msg.contains("not found") {
                send(WsResponse::err(
                    req.id.clone(),
                    409,
                    format!("openclaw session not connected for agent {agent_id}"),
                ))
                .await;
            } else {
                send(WsResponse::err(
                    req.id.clone(),
                    500,
                    format!("openclaw send failed: {e}"),
                ))
                .await;
            }
            return;
        }
    };

    let deadline = start + Duration::from_millis(timeout_ms);
    let mut reply = String::new();
    let mut chunks: Vec<String> = Vec::new();
    let mut done = false;
    let mut first_chunk_latency_ms = -1i64;

    while Instant::now() < deadline {
        for msg in openclaw.drain_offline_messages(&test_device_id) {
            if !msg.correlation_id.is_empty() && msg.correlation_id != message_id {
                continue;
            }
            let chunk = msg.text.trim().to_string();
            if !chunk.is_empty() {
                reply.push_str(&chunk);
                chunks.push(chunk.clone());
                if first_chunk_latency_ms < 0 {
                    first_chunk_latency_ms = start.elapsed().as_millis() as i64;
                }
                if stream_events {
                    let mut body = json!({
                        "agent_id": agent_id,
                        "message_id": message_id,
                        "chunk": chunk,
                        "chunk_index": chunks.len(),
                        "reply": reply.trim(),
                        "latency_ms": start.elapsed().as_millis(),
                        "done": false,
                    });
                    if first_chunk_latency_ms >= 0 {
                        body["first_chunk_latency_ms"] = json!(first_chunk_latency_ms);
                    }
                    send(WsResponse {
                        id: req.id.clone(),
                        status: 206,
                        headers: Default::default(),
                        body,
                        error: String::new(),
                    })
                    .await;
                }
            }
            if msg.is_end {
                done = true;
                break;
            }
        }
        if done {
            break;
        }
        tokio::time::sleep(Duration::from_millis(120)).await;
    }

    let reply = reply.trim().to_string();
    if !done {
        openclaw.clear_offline_messages(&test_device_id);
        if reply.is_empty() {
            send(WsResponse::err(req.id.clone(), 504, "openclaw response timeout")).await;
            return;
        }
        let mut body = json!({
            "agent_id": agent_id,
            "message_id": message_id,
            "reply": reply,
            "chunks": chunks,
            "chunk_count": chunks.len(),
            "latency_ms": start.elapsed().as_millis(),
            "timeout_ms": timeout_ms,
            "finished": false,
        });
        if first_chunk_latency_ms >= 0 {
            body["first_chunk_latency_ms"] = json!(first_chunk_latency_ms);
        }
        send(WsResponse {
            id: req.id.clone(),
            status: 504,
            headers: Default::default(),
            body,
            error: "openclaw response timeout (partial reply received)".into(),
        })
        .await;
        return;
    }

    let mut body = json!({
        "agent_id": agent_id,
        "message_id": message_id,
        "reply": reply,
        "chunks": chunks,
        "chunk_count": chunks.len(),
        "latency_ms": start.elapsed().as_millis(),
        "timeout_ms": timeout_ms,
        "finished": true,
    });
    if first_chunk_latency_ms >= 0 {
        body["first_chunk_latency_ms"] = json!(first_chunk_latency_ms);
    }
    send(WsResponse::ok(req.id, body)).await;
}
