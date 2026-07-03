use std::time::{Duration, Instant};

use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    connect_async,
    tungstenite::client::IntoClientRequest,
};
use xiaozhi_core::constants::ota_test::{CLIENT_ID, DEVICE_ID};

const OTA_HTTP_PATH: &str = "/xiaozhi/ota/";
/// Manager OTA 测试指定 test/external，避免本机请求被 `select_env_for_client_ip` 误判为 test
const OTA_TEST_ENV_HEADER: &str = "X-Ota-Test-Env";

/// 对 OTA 配置做真实连通性测试（对齐 Go `testOTAConfigWithMQTTUDP`）。
pub async fn test_ota_config(cfg: &Value, env: Option<&str>) -> Value {
    let (ws_url_from_config, env_data) = match resolve_ws_and_env(cfg, env) {
        Some(v) => v,
        None => {
            return json!({
                "ok": false,
                "message": "未配置 WebSocket URL",
                "websocket": {
                    "ok": false,
                    "message": "未配置 WebSocket URL",
                    "first_packet_ms": 0,
                },
            });
        }
    };

    let mqtt_enabled = env_data
        .get("mqtt")
        .and_then(|v| v.get("enable"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let Some(ota_http_url) = ws_url_to_ota_url(&ws_url_from_config) else {
        return json!({
            "ok": false,
            "message": "URL 解析失败",
            "websocket": {
                "ok": false,
                "message": "URL 解析失败",
                "first_packet_ms": 0,
            },
        });
    };

    let http_t0 = Instant::now();
    let (ota_ok, ota_body, http_msg) = call_ota_api(&ota_http_url, env).await;
    let http_ms = http_t0.elapsed().as_millis() as u64;

    if !ota_ok {
        return json!({
            "ok": false,
            "message": http_msg,
            "websocket": {
                "ok": false,
                "message": http_msg,
                "first_packet_ms": http_ms,
            },
            "ota_response": ota_body.unwrap_or_default(),
        });
    }

    let body_text = ota_body.unwrap_or_default();
    let ota_resp: Value = match serde_json::from_str(&body_text) {
        Ok(v) => v,
        Err(_) => {
            return json!({
                "ok": false,
                "message": "OTA 响应非 JSON",
                "websocket": {
                    "ok": false,
                    "message": "OTA 响应非 JSON",
                    "first_packet_ms": http_ms,
                },
                "ota_response": body_text,
            });
        }
    };

    let ws_url = ota_resp
        .get("websocket")
        .and_then(|v| v.get("url"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    if ws_url.is_empty() {
        return json!({
            "ok": false,
            "message": "OTA 响应中无 websocket.url",
            "websocket": {
                "ok": false,
                "message": "OTA 响应中无 websocket.url",
                "first_packet_ms": http_ms,
            },
            "ota_response": body_text,
        });
    }

    let ws_t0 = Instant::now();
    let (ws_ok, ws_msg) = test_websocket(&ws_url).await;
    let ws_total_ms = http_ms + ws_t0.elapsed().as_millis() as u64;

    let ws_message = if ws_ok {
        "WebSocket 连接正常".to_string()
    } else {
        ws_msg
    };

    let mut result = json!({
        "ok": ws_ok,
        "message": ws_message.clone(),
        "first_packet_ms": ws_total_ms,
        "websocket": {
            "ok": ws_ok,
            "message": ws_message,
            "first_packet_ms": ws_total_ms,
        },
        "ota_response": body_text,
    });

    if mqtt_enabled {
        let mqtt_result = test_mqtt_from_ota_response(&ota_resp).await;
        let mqtt_ok = mqtt_result
            .get("ok")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        result["mqtt_udp"] = mqtt_result;
        result["ok"] = json!(ws_ok && mqtt_ok);
        if !mqtt_ok && ws_ok {
            result["message"] = result["mqtt_udp"]
                .get("message")
                .cloned()
                .unwrap_or(json!("MQTT UDP 测试失败"));
        }
    }

    result
}

/// 与 Go 一致：指定 env 时用该环境；否则优先 external，再 fallback test。
pub fn resolve_ws_and_env(cfg: &Value, env: Option<&str>) -> Option<(String, Value)> {
    if let Some(key) = env {
        let env_data = cfg.get(key)?.clone();
        let ws = env_data
            .get("websocket")
            .and_then(|v| v.get("url"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        return if ws.is_empty() { None } else { Some((ws, env_data)) };
    }

    for key in ["external", "test"] {
        if let Some(env_data) = cfg.get(key) {
            let ws = env_data
                .get("websocket")
                .and_then(|v| v.get("url"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if !ws.is_empty() {
                return Some((ws, env_data.clone()));
            }
        }
    }
    None
}

fn ws_url_to_ota_url(ws_url: &str) -> Option<String> {
    let parsed = url::Url::parse(ws_url).ok()?;
    let scheme = match parsed.scheme() {
        "ws" => "http",
        "wss" => "https",
        _ => return None,
    };
    let host = parsed.host_str()?;
    let authority = match parsed.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    };
    Some(format!("{scheme}://{authority}{OTA_HTTP_PATH}"))
}

fn build_http_client(ota_url: &str) -> reqwest::Client {
    let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(5));
    if should_bypass_proxy(ota_url) {
        builder = builder.no_proxy();
    }
    builder.build().unwrap_or_else(|_| reqwest::Client::new())
}

fn should_bypass_proxy(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return true;
    };
    let Some(host) = parsed.host_str() else {
        return true;
    };
    let host = host.to_lowercase();
    if host == "localhost" || host == "127.0.0.1" || host == "::1" {
        return true;
    }
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return match ip {
            std::net::IpAddr::V4(v4) => {
                v4.is_loopback() || v4.is_private() || v4.is_link_local()
            }
            std::net::IpAddr::V6(v6) => v6.is_loopback(),
        };
    }
    false
}

async fn call_ota_api(ota_url: &str, env: Option<&str>) -> (bool, Option<String>, String) {
    let client = build_http_client(ota_url);

    let mut headers = HeaderMap::new();
    headers.insert("Device-Id", HeaderValue::from_static(DEVICE_ID));
    headers.insert("Client-Id", HeaderValue::from_static(CLIENT_ID));
    if let Some(env) = env {
        if let Ok(value) = HeaderValue::from_str(env) {
            headers.insert(OTA_TEST_ENV_HEADER, value);
        }
    }

    let resp = match client
        .post(ota_url)
        .headers(headers)
        .header("Content-Type", "application/json")
        .body("{}")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return (
                false,
                None,
                format!("OTA 请求失败: {e}"),
            );
        }
    };

    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return (
            false,
            Some(text),
            format!("OTA 返回 HTTP {status}"),
        );
    }

    (true, Some(text), String::new())
}

async fn test_websocket(ws_url: &str) -> (bool, String) {
    let mut request = match ws_url.into_client_request() {
        Ok(r) => r,
        Err(e) => return (false, format!("WebSocket 连接失败: {e}")),
    };
    let headers = request.headers_mut();
    let _ = headers.insert("Device-Id", HeaderValue::from_static(DEVICE_ID));
    let _ = headers.insert("Client-Id", HeaderValue::from_static(CLIENT_ID));

    match tokio::time::timeout(Duration::from_secs(5), connect_async(request)).await {
        Ok(Ok((mut stream, _))) => {
            let _ = stream.close(None).await;
            (true, String::new())
        }
        Ok(Err(e)) => (false, format!("WebSocket 连接失败: {e}")),
        Err(_) => (false, "WebSocket 连接超时".to_string()),
    }
}

async fn test_mqtt_from_ota_response(ota_resp: &Value) -> Value {
    let Some(mqtt) = ota_resp.get("mqtt") else {
        return json!({
            "ok": false,
            "message": "OTA响应未返回MQTT配置，无法测试MQTT UDP",
            "first_packet_ms": 0,
        });
    };

    let endpoint = mqtt
        .get("endpoint")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let client_id = mqtt
        .get("client_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let username = mqtt
        .get("username")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let password = mqtt
        .get("password")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let publish_topic = mqtt
        .get("publish_topic")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();

    if endpoint.is_empty() {
        return json!({
            "ok": false,
            "message": "OTA响应中MQTT endpoint为空",
            "first_packet_ms": 0,
        });
    }
    if publish_topic.is_empty() {
        return json!({
            "ok": false,
            "message": "OTA响应中MQTT publish_topic为空",
            "first_packet_ms": 0,
        });
    }

    match test_mqtt_connect_and_hello(endpoint, client_id, username, password, publish_topic).await
    {
        Ok(ms) => json!({
            "ok": true,
            "message": "MQTT 连接并收到 hello 响应",
            "first_packet_ms": ms,
        }),
        Err(msg) => json!({
            "ok": false,
            "message": msg,
            "first_packet_ms": 0,
        }),
    }
}

fn device_sub_topic_from_client_id(client_id: &str) -> Option<String> {
    let parts: Vec<&str> = client_id.split("@@@").collect();
    if parts.len() >= 2 && !parts[1].is_empty() {
        Some(format!("/p2p/device_sub/{}", parts[1]))
    } else {
        None
    }
}

async fn test_mqtt_connect_and_hello(
    endpoint: &str,
    client_id: &str,
    username: &str,
    password: &str,
    publish_topic: &str,
) -> Result<u64, String> {
    use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS, Transport};

    let sub_topic = device_sub_topic_from_client_id(client_id).ok_or_else(|| {
        format!("无法从 client_id 解析订阅 topic: {client_id}（需 GID_xxx@@@mac@@@uuid 格式）")
    })?;

    let (host, port) = parse_mqtt_host_port(endpoint)?;
    let mut opts = MqttOptions::new(client_id.to_string(), host, port);
    opts.set_credentials(username.to_string(), password.to_string());
    opts.set_keep_alive(Duration::from_secs(30));
    if port == 8883 || port == 8884 {
        opts.set_transport(Transport::tls_with_default_config());
    }

    let (client, mut eventloop) = AsyncClient::new(opts, 16);
    let start = Instant::now();

    let mut connected = false;
    while !connected {
        if start.elapsed() > Duration::from_secs(5) {
            return Err("MQTT 连接超时".to_string());
        }
        match eventloop.poll().await {
            Ok(Event::Incoming(Packet::ConnAck(r))) => {
                if r.code == rumqttc::ConnectReturnCode::Success {
                    connected = true;
                } else {
                    return Err(format!("MQTT CONNACK 失败: {:?}", r.code));
                }
            }
            Ok(_) => {}
            Err(e) => return Err(format!("MQTT 连接错误: {e}")),
        }
    }

    client
        .subscribe(&sub_topic, QoS::AtLeastOnce)
        .await
        .map_err(|e| format!("MQTT 订阅 {sub_topic} 失败: {e}"))?;

    let mut subscribed = false;
    while !subscribed {
        if start.elapsed() > Duration::from_secs(5) {
            return Err(format!("MQTT 订阅 {sub_topic} 确认超时"));
        }
        match eventloop.poll().await {
            Ok(Event::Incoming(Packet::SubAck(_))) => subscribed = true,
            Ok(_) => {}
            Err(e) => return Err(format!("MQTT 订阅事件错误: {e}")),
        }
    }

    client
        .publish(
            publish_topic,
            QoS::AtLeastOnce,
            false,
            br#"{"type":"hello","version":1,"transport":"mqtt_udp","features":{}}"#,
        )
        .await
        .map_err(|e| format!("MQTT 发布 hello 失败: {e}"))?;

    let deadline = start + Duration::from_secs(8);
    loop {
        if Instant::now() > deadline {
            return Err(
                "等待 MQTT hello 响应超时（请确认 xiaozhi-server 已开启 MQTT 客户端服务并订阅 Broker）"
                    .to_string(),
            );
        }
        tokio::select! {
            event = eventloop.poll() => {
                match event {
                    Ok(Event::Incoming(Packet::Publish(p))) => {
                        if let Ok(v) = serde_json::from_slice::<Value>(&p.payload) {
                            if v.get("type").and_then(|t| t.as_str()) == Some("hello") {
                                return Ok(start.elapsed().as_millis() as u64);
                            }
                        }
                    }
                    Ok(Event::Incoming(Packet::ConnAck(_))) => {}
                    Ok(_) => {}
                    Err(e) => return Err(format!("MQTT 事件错误: {e}")),
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }
}

fn parse_mqtt_host_port(endpoint: &str) -> Result<(String, u16), String> {
    let endpoint = endpoint.trim();
    if endpoint.contains("://") {
        let parsed = url::Url::parse(endpoint).map_err(|e| format!("endpoint 无效: {e}"))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| "endpoint 缺少 host".to_string())?
            .to_string();
        let port = parsed.port().unwrap_or(1883);
        return Ok((host, port));
    }
    if let Some((host, port)) = endpoint.rsplit_once(':') {
        let port: u16 = port.parse().map_err(|_| format!("端口号无效: {port}"))?;
        return Ok((host.to_string(), port));
    }
    Ok((endpoint.to_string(), 1883))
}

async fn test_tcp_endpoint(endpoint: &str) -> (bool, String, Option<u64>) {
    let addr = match parse_host_port(endpoint) {
        Some(a) => a,
        None => return (false, format!("端点格式无效: {endpoint}"), None),
    };

    let start = Instant::now();
    match tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(&addr)).await {
        Ok(Ok(_)) => {
            let ms = start.elapsed().as_millis() as u64;
            (true, "端点可达".to_string(), Some(ms))
        }
        Ok(Err(e)) => (false, format!("连接失败: {e}"), None),
        Err(_) => (false, "连接超时".to_string(), None),
    }
}

fn parse_host_port(endpoint: &str) -> Option<String> {
    let endpoint = endpoint.trim();
    if endpoint.is_empty() {
        return None;
    }
    if endpoint.contains("://") {
        url::Url::parse(endpoint).ok().and_then(|u| {
            let host = u.host_str()?;
            let port = u.port().unwrap_or(1883);
            Some(format!("{host}:{port}"))
        })
    } else if endpoint.contains(':') {
        Some(endpoint.to_string())
    } else {
        Some(format!("{endpoint}:1883"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_to_ota_url() {
        assert_eq!(
            ws_url_to_ota_url("ws://127.0.0.1:8989/xiaozhi/v1/").as_deref(),
            Some("http://127.0.0.1:8989/xiaozhi/ota/")
        );
        assert_eq!(
            ws_url_to_ota_url("ws://192.168.3.11:8989/xiaozhi/v1/").as_deref(),
            Some("http://192.168.3.11:8989/xiaozhi/ota/")
        );
    }

    #[test]
    fn bypass_proxy_for_private_ip() {
        assert!(should_bypass_proxy("http://192.168.3.11:8989/xiaozhi/ota/"));
        assert!(should_bypass_proxy("http://127.0.0.1:8989/xiaozhi/ota/"));
        assert!(!should_bypass_proxy("https://api.example.com/xiaozhi/ota/"));
    }
}
