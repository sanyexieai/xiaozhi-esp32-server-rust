use std::collections::HashSet;
use std::fs;
use std::path::Path;

pub const DEFAULT_MANAGER_PORT: u16 = 8099;

/// Manager 启动后写入的实际 HTTP 地址，供 xiaozhi-server 自动发现（端口切换时无需改 config.yaml）。
pub const DEFAULT_ENDPOINT_FILE: &str = "data/manager.endpoint";

/// 从 `manager.backend_url` 解析 HTTP 端口。
pub fn backend_url_port(backend_url: &str) -> Option<u16> {
    parse_host_port(backend_url).map(|(_, port)| port)
}

pub fn write_manager_endpoint(path: impl AsRef<Path>, backend_url: &str) -> std::io::Result<()> {
    let url = backend_url.trim();
    if url.is_empty() {
        return Ok(());
    }
    if let Some(parent) = path.as_ref().parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path.as_ref(), format!("{url}\n"))
}

pub fn read_manager_endpoint(path: impl AsRef<Path>) -> Option<String> {
    let content = fs::read_to_string(path.as_ref()).ok()?;
    let url = content.trim();
    if url.is_empty() {
        None
    } else {
        Some(url.to_string())
    }
}

/// 优先读端点文件，否则回退 config.yaml 中的 `manager.backend_url`。
pub fn resolve_manager_backend_url(config_url: &str) -> String {
    resolve_manager_backend_url_from(config_url, Path::new(DEFAULT_ENDPOINT_FILE))
}

pub fn resolve_manager_backend_url_from(config_url: &str, endpoint_file: &Path) -> String {
    read_manager_endpoint(endpoint_file).unwrap_or_else(|| config_url.to_string())
}

pub fn backend_to_ws_url(backend_url: &str) -> String {
    let url = backend_url.trim_end_matches('/');
    if let Some(rest) = url.strip_prefix("https://") {
        format!("wss://{rest}/ws")
    } else if let Some(rest) = url.strip_prefix("http://") {
        format!("ws://{rest}/ws")
    } else {
        format!("ws://{url}/ws")
    }
}

/// 按优先级返回 Manager WS 候选地址：端点文件 > config > 同 host 端口递增扫描。
pub fn manager_ws_url_candidates(config_url: &str) -> Vec<String> {
    manager_ws_url_candidates_from(config_url, Path::new(DEFAULT_ENDPOINT_FILE))
}

pub fn manager_ws_url_candidates_from(config_url: &str, endpoint_file: &Path) -> Vec<String> {
    const MAX_OFFSET: u16 = 20;
    let mut seen = HashSet::new();
    let mut out = Vec::new();

    let mut push = |backend: &str| {
        let ws = backend_to_ws_url(backend);
        if seen.insert(ws.clone()) {
            out.push(ws);
        }
    };

    if let Some(file_url) = read_manager_endpoint(endpoint_file) {
        push(&file_url);
    }
    push(config_url);

    if let Some((host, base_port)) = parse_host_port(config_url) {
        for offset in 1..=MAX_OFFSET {
            let port = base_port.saturating_add(offset);
            push(&format!("http://{host}:{port}"));
        }
    }

    out
}

fn parse_host_port(url: &str) -> Option<(String, u16)> {
    let url = url.trim().trim_end_matches('/');
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))?;

    if rest.starts_with('[') {
        let (_, after) = rest.split_once("]:")?;
        let host = rest.trim_end_matches(after).trim_end_matches(':');
        let port: u16 = after.parse().ok()?;
        return Some((host.to_string(), port));
    }

    if let Some((host, port_str)) = rest.rsplit_once(':') {
        if !host.contains(':') {
            if let Ok(port) = port_str.parse() {
                return Some((host.to_string(), port));
            }
        }
    }

    Some((rest.to_string(), 80))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_endpoint_file() -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        std::env::temp_dir().join(format!("manager-endpoint-{nanos}.txt"))
    }

    #[test]
    fn write_and_read_endpoint_roundtrip() {
        let path = temp_endpoint_file();
        write_manager_endpoint(&path, "http://127.0.0.1:8081").expect("write");
        assert_eq!(
            read_manager_endpoint(&path),
            Some("http://127.0.0.1:8081".to_string())
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn resolve_prefers_endpoint_file() {
        let path = temp_endpoint_file();
        write_manager_endpoint(&path, "http://127.0.0.1:8082").expect("write");
        assert_eq!(
            resolve_manager_backend_url_from("http://127.0.0.1:8080", &path),
            "http://127.0.0.1:8082"
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn ws_candidates_put_endpoint_file_first() {
        let path = temp_endpoint_file();
        write_manager_endpoint(&path, "http://127.0.0.1:8083").expect("write");
        let urls = manager_ws_url_candidates_from("http://127.0.0.1:8080", &path);
        assert_eq!(urls.first().map(String::as_str), Some("ws://127.0.0.1:8083/ws"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn backend_to_ws_url_handles_http_and_https() {
        assert_eq!(
            backend_to_ws_url("http://127.0.0.1:8080"),
            "ws://127.0.0.1:8080/ws"
        );
        assert_eq!(
            backend_to_ws_url("https://manager.example.com"),
            "wss://manager.example.com/ws"
        );
    }
}
