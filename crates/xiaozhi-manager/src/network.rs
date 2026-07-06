use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::time::Duration;

/// 绑定 TCP 端口；若配置端口被占用则依次尝试后续端口
pub async fn bind_tcp_listener(
    host: &str,
    port: u16,
    force_port: bool,
) -> anyhow::Result<(tokio::net::TcpListener, SocketAddr)> {
    const MAX_OFFSET: u16 = 20;
    let my_pid = std::process::id();

    for offset in 0..=MAX_OFFSET {
        let try_port = port.saturating_add(offset);
        let addr: SocketAddr = format!("{host}:{try_port}")
            .parse()
            .map_err(|e| anyhow::anyhow!("地址解析失败: {e}"))?;

        if let Some(reason) = port_occupied_reason(try_port, my_pid, force_port) {
            tracing::warn!("端口 {try_port} 已被占用 ({reason})，尝试下一个端口");
            continue;
        }

        match TcpListener::bind(addr) {
            Ok(std_listener) => {
                std_listener.set_nonblocking(true)?;
                let listener = tokio::net::TcpListener::from_std(std_listener)?;
                if offset > 0 {
                    tracing::warn!("端口 {port} 已被占用，已自动切换到 {try_port}");
                    tracing::warn!(
                        "实际地址已写入 data/manager.endpoint，xiaozhi-server 会自动连接 {try_port}"
                    );
                }
                return Ok((listener, addr));
            }
            Err(e) if is_addr_in_use(&e) => {
                tracing::warn!(
                    "端口 {try_port} 绑定失败（已被占用{}），尝试下一个端口",
                    port_occupied_reason(try_port, my_pid, force_port)
                        .map(|r| format!(": {r}"))
                        .unwrap_or_default()
                );
            }
            Err(e) => return Err(e.into()),
        }
    }

    anyhow::bail!(
        "端口 {port} ~ {} 均已被占用，请使用 --port 指定其他端口",
        port + MAX_OFFSET
    )
}

/// 综合 netstat、Docker 映射、TCP 探活，判断端口是否被其他服务占用。
fn port_occupied_reason(port: u16, my_pid: u32, force_port: bool) -> Option<String> {
    if let Some((pid, local)) = other_listener_on_port(port, my_pid) {
        return Some(format!(
            "进程 PID {pid}{} 监听 {local}",
            process_name_hint(pid)
        ));
    }
    if let Some(container) = docker_publishes_host_port(port) {
        return Some(format!("Docker 容器 {container} 已映射 host:{port}"));
    }
    if port_accepts_tcp(port) {
        return Some("已有服务在响应 TCP 连接".to_string());
    }
    if !force_port && port == 8080 && docker_desktop_running() {
        return Some(
            "Docker Desktop 正在运行，8080 为常用容器映射端口，已主动避让".to_string(),
        );
    }
    None
}

#[cfg(windows)]
fn docker_desktop_running() -> bool {
    std::process::Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq com.docker.backend.exe", "/FO", "CSV", "/NH"])
        .output()
        .ok()
        .map(|o| {
            let text = String::from_utf8_lossy(&o.stdout);
            text.contains("com.docker.backend.exe")
        })
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn docker_desktop_running() -> bool {
    std::process::Command::new("docker")
        .args(["info"])
        .output()
        .ok()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 向本机端口发起连接；能连上说明已有服务在监听（含 Docker 端口转发）。
fn port_accepts_tcp(port: u16) -> bool {
    for target in [format!("127.0.0.1:{port}"), format!("[::1]:{port}")] {
        if let Ok(addr) = target.parse::<SocketAddr>() {
            if TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok() {
                return true;
            }
        }
    }
    false
}

/// 检查 Docker 是否已将 host 端口映射给某个容器（Windows 上 netstat 可能漏检）。
fn docker_publishes_host_port(port: u16) -> Option<String> {
    let output = std::process::Command::new("docker")
        .args(["ps", "--format", "{{.Names}}|{{.Ports}}"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let host_port = format!(":{port}->");
    for line in text.lines() {
        let (name, ports) = line.split_once('|')?;
        if ports.contains(&host_port) {
            return Some(name.to_string());
        }
    }
    None
}

/// 查询是否有**其他进程**正在监听该端口（含 IPv4 / IPv6，Windows 上两者可并存）。
fn other_listener_on_port(port: u16, my_pid: u32) -> Option<(u32, String)> {
    port_listeners(port)
        .into_iter()
        .find(|entry| entry.pid != my_pid && entry.pid != 0)
        .map(|entry| (entry.pid, entry.local_addr))
}

struct PortListener {
    local_addr: String,
    pid: u32,
}

fn port_listeners(port: u16) -> Vec<PortListener> {
    #[cfg(windows)]
    {
        port_listeners_netstat(port)
    }
    #[cfg(not(windows))]
    {
        let _ = port;
        Vec::new()
    }
}

#[cfg(windows)]
fn port_listeners_netstat(port: u16) -> Vec<PortListener> {
    let output = match std::process::Command::new("netstat").args(["-ano"]).output() {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let mut listeners = Vec::new();

    for line in text.lines() {
        if !line.contains("LISTENING") {
            continue;
        }
        let mut parts = line.split_whitespace();
        let local = match parts.nth(1) {
            Some(v) => v,
            None => continue,
        };
        if local_port(local) != Some(port) {
            continue;
        }
        let pid: u32 = match parts.last().and_then(|p| p.parse().ok()) {
            Some(v) => v,
            None => continue,
        };
        listeners.push(PortListener {
            local_addr: local.to_string(),
            pid,
        });
    }
    listeners
}

/// 从 netstat 本地地址列解析端口号（支持 `0.0.0.0:8080`、`[::]:8080`、`[::1]:8080`）。
fn local_port(local: &str) -> Option<u16> {
    if local.starts_with('[') {
        local.rsplit_once("]:").and_then(|(_, p)| p.parse().ok())
    } else {
        local.rsplit_once(':').and_then(|(_, p)| p.parse().ok())
    }
}

#[cfg(windows)]
fn process_name_hint(pid: u32) -> String {
    let output = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output()
        .ok();
    let Some(output) = output else {
        return String::new();
    };
    let line = String::from_utf8_lossy(&output.stdout);
    let name = line
        .split(',')
        .next()
        .unwrap_or("")
        .trim_matches('"');
    if name.is_empty() {
        String::new()
    } else {
        format!(" ({name})")
    }
}

#[cfg(not(windows))]
fn process_name_hint(_pid: u32) -> String {
    String::new()
}

fn is_addr_in_use(err: &std::io::Error) -> bool {
    err.kind() == std::io::ErrorKind::AddrInUse
        || (cfg!(windows) && err.raw_os_error() == Some(10048))
}

/// 获取本机用于出网的局域网 IPv4（供 OTA / WebSocket 默认值）
pub fn primary_lan_ip() -> Option<String> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let ip = socket.local_addr().ok()?.ip();
    if is_usable_lan_ip(&ip) {
        Some(ip.to_string())
    } else {
        None
    }
}

fn is_usable_lan_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => !v4.is_unspecified() && !v4.is_loopback() && (v4.is_private() || v4.is_link_local()),
        IpAddr::V6(v6) => !v6.is_unspecified() && !v6.is_loopback(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn primary_lan_ip_is_private_or_empty() {
        if let Some(ip) = primary_lan_ip() {
            let parsed: Ipv4Addr = ip.parse().expect("ipv4");
            assert!(parsed.is_private() || parsed.is_link_local());
        }
    }

    #[test]
    fn local_port_parses_ipv4_and_ipv6() {
        assert_eq!(local_port("0.0.0.0:8080"), Some(8080));
        assert_eq!(local_port("127.0.0.1:8080"), Some(8080));
        assert_eq!(local_port("[::]:8080"), Some(8080));
        assert_eq!(local_port("[::1]:8080"), Some(8080));
        assert_eq!(local_port("192.168.1.1:8990"), Some(8990));
        assert_eq!(local_port("0.0.0.0:808"), Some(808));
    }
}
