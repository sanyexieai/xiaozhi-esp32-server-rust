use std::net::{IpAddr, UdpSocket};

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
}
