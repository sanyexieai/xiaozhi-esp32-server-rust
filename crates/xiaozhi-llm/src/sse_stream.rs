/// 解析 HTTP SSE 文本（`event:` / `data:` 行，事件以空行分隔）。
pub fn iter_sse_events(body: &str) -> impl Iterator<Item = SseEvent> + '_ {
    let mut events = Vec::new();
    let mut event_type = String::new();
    let mut data_lines: Vec<String> = Vec::new();

    let flush = |event_type: &mut String, data_lines: &mut Vec<String>, out: &mut Vec<SseEvent>| {
        if data_lines.is_empty() {
            event_type.clear();
            return;
        }
        out.push(SseEvent {
            event_type: if event_type.is_empty() {
                "message".to_string()
            } else {
                event_type.clone()
            },
            data: data_lines.join("\n"),
        });
        event_type.clear();
        data_lines.clear();
    };

    for line in body.lines() {
        if line.is_empty() {
            flush(&mut event_type, &mut data_lines, &mut events);
            continue;
        }
        if let Some(rest) = line.strip_prefix("event:") {
            event_type = rest.trim_start().to_string();
            continue;
        }
        if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.trim_start().to_string());
        }
    }
    flush(&mut event_type, &mut data_lines, &mut events);
    events.into_iter()
}

#[derive(Debug, Clone)]
pub struct SseEvent {
    pub event_type: String,
    pub data: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_chunk_and_done() {
        let body = "event: chunk\ndata: {\"a\":1}\n\nevent: done\ndata: {}\n\n";
        let events: Vec<_> = iter_sse_events(body).collect();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, "chunk");
        assert_eq!(events[0].data, "{\"a\":1}");
    }
}
