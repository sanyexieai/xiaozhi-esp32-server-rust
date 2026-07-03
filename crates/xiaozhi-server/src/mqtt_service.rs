//! MQTT+UDP 设备接入（对齐 Go `mqtt_udp_adapter.go`）

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use rumqttc::{AsyncClient, Event, EventLoop, MqttOptions, Packet, QoS, Transport};
use tokio::net::UdpSocket;
use tokio::sync::RwLock;
use xiaozhi_chat::{ChatManager, OutboundFrame};
use xiaozhi_config::{MqttClientConfig, UdpConfig};
use xiaozhi_protocol::messages::{ClientMessage, ServerMessage};
use xiaozhi_protocol::mqtt::{self, MqttLifecycleEvent};
use xiaozhi_transport::udp::UdpCrypto;

use crate::device_handler::{process_client_message, DeviceRuntime, UdpHelloInfo, UdpSession};

/// 供 Bridge / speak 路径在设备离线唤醒前建立 MQTT 硬件通道。
#[derive(Default)]
pub struct MqttDeviceGateway {
    ctx: RwLock<Option<Arc<MqttHandlerCtx>>>,
}

impl MqttDeviceGateway {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn attach(&self, ctx: Arc<MqttHandlerCtx>) {
        *self.ctx.write().await = Some(ctx);
    }

    pub async fn detach(&self) {
        *self.ctx.write().await = None;
    }

    /// 确保 ChatManager 存在且已注册 hardware 出站（MQTT publish 循环）。
    pub async fn prepare_hardware_wake(&self, device_id: &str) -> Result<(), String> {
        let ctx = self
            .ctx
            .read()
            .await
            .clone()
            .ok_or_else(|| "MQTT+UDP 服务未就绪".to_string())?;
        let chat_mgr = ctx
            .runtime
            .ensure_chat_manager(device_id)
            .await
            .map_err(|e| e.to_string())?;
        ensure_device_session(ctx, device_id, chat_mgr).await;
        Ok(())
    }

    pub async fn is_broker_online(&self, device_id: &str) -> Option<bool> {
        let ctx = self.ctx.read().await.clone()?;
        Some(device_broker_online(&ctx.lifecycle_states, device_id))
    }

    pub async fn has_active_udp_session(&self, device_id: &str) -> Option<bool> {
        let ctx = self.ctx.read().await.clone()?;
        Some(active_conn_for_device(&ctx.conn_map, device_id).is_some())
    }
}

const MQTT_RECONNECT_DELAY: Duration = Duration::from_secs(2);
/// 对齐 Go `mqtt_udp_conn.go` MaxIdleDuration：300s 无上下行则销毁 transport
const TRANSPORT_MAX_IDLE_SECS: i64 = 300;
const TRANSPORT_ACTIVE_CHECK_INTERVAL: Duration = Duration::from_secs(30);
/// hello 轮换后设备可能仍用旧 conn_id 上行，保留旧会话以便解密/过渡。
const UDP_CONN_STALE_GRACE: Duration = Duration::from_secs(60);
/// hello 后设备需先上行一包 UDP 才能获知 ephemeral 端口；欢迎语 TTS 合成较慢，需更长等待。
const UDP_REMOTE_ADDR_WAIT: Duration = Duration::from_secs(10);
/// MQTT 信令与 UDP 音频分通道；设备 `SetDeviceState(Speaking)` 经 Schedule 异步，需短余量。
/// 过长会推迟首包 UDP，设备常在 tts start 后 ~4s 自发 listen start 导致欢迎语无声。
const MQTT_TTS_START_UDP_GRACE: Duration = Duration::from_millis(400);
/// hello 后 UDP 远端地址已建立时，设备状态切换余量可更短。
const MQTT_TTS_START_UDP_GRACE_WARM: Duration = Duration::from_millis(200);
/// 对齐 Go `mqtt_udp_conn.go` SendCmd：QoS 0。QoS1 会等 hello/MCP 的 PUBACK，阻塞 tts start 实际下发。
const MQTT_CMD_QOS: QoS = QoS::AtMostOnce;
/// 避免对同一设备频繁下发 goodbye
const REHELLO_COOLDOWN: Duration = Duration::from_secs(30);
const UDP_SSRC_WARN_INTERVAL: Duration = Duration::from_secs(5);
/// 服务端重启后设备可能仍用旧 ssrc 上行；此窗口内等待设备 hello，勿因 MQTT 误发 goodbye。
const ORPHAN_UDP_REHELLO_WAIT: Duration = Duration::from_secs(300);

fn is_tts_start_command(data: &[u8]) -> bool {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(data) else {
        return false;
    };
    matches!(
        (v.get("type").and_then(|t| t.as_str()), v.get("state").and_then(|s| s.as_str())),
        (Some("tts"), Some("start"))
    )
}

async fn wait_udp_remote_addr(
    device_addrs: &DashMap<String, SocketAddr>,
    device_id: &str,
    timeout: Duration,
) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if device_addrs.contains_key(device_id) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

struct DeviceMqttSession {
    chat_mgr: Arc<ChatManager>,
    out_tx: tokio::sync::mpsc::UnboundedSender<OutboundFrame>,
    udp: Option<UdpSession>,
    udp_send_state: Arc<UdpSendState>,
    last_active_ts: AtomicI64,
    retained_until_ns: AtomicI64,
    broker_online: AtomicBool,
}

impl DeviceMqttSession {
    fn touch_active(&self) {
        self.last_active_ts.store(chrono::Utc::now().timestamp(), Ordering::Relaxed);
    }

    fn mark_broker_online(&self) {
        self.broker_online.store(true, Ordering::Relaxed);
        self.retained_until_ns.store(0, Ordering::Relaxed);
        self.touch_active();
    }

    fn mark_broker_offline(&self, grace: Duration) {
        self.broker_online.store(false, Ordering::Relaxed);
        let until = chrono::Utc::now() + grace;
        self.retained_until_ns.store(
            until.timestamp_nanos_opt().unwrap_or(0),
            Ordering::Relaxed,
        );
    }

    /// 对齐 Go `MqttUdpConn.IsActive`
    fn is_active(&self) -> bool {
        if self.broker_online.load(Ordering::Relaxed) {
            return true;
        }
        let retained = self.retained_until_ns.load(Ordering::Relaxed);
        if retained > 0 {
            let now_ns = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
            if now_ns < retained {
                return true;
            }
            return false;
        }
        let last = self.last_active_ts.load(Ordering::Relaxed);
        if last <= 0 {
            return true;
        }
        chrono::Utc::now().timestamp() - last < TRANSPORT_MAX_IDLE_SECS
    }
}

/// 下行 UDP 包 sequence，hello 轮换时需与设备 remote_sequence_ 同步归零。
struct UdpSendState {
    sequence: AtomicU32,
    /// hello 轮换递增，供 UDP 发送任务重置「首包」日志。
    session_generation: AtomicU32,
}

impl UdpSendState {
    fn new() -> Self {
        Self {
            sequence: AtomicU32::new(0),
            session_generation: AtomicU32::new(0),
        }
    }

    fn reset(&self) {
        self.sequence.store(0, Ordering::Relaxed);
        self.session_generation.fetch_add(1, Ordering::Relaxed);
    }

    fn next_sequence(&self) -> u32 {
        self.sequence.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn session_generation(&self) -> u32 {
        self.session_generation.load(Ordering::Relaxed)
    }
}

struct ConnState {
    device_id: String,
    crypto: UdpCrypto,
    conn_id: u32,
    /// hello 轮换后旧 conn 保留至该时刻，供设备仍用旧 ssrc 的上行包解密。
    stale_until: Option<Instant>,
}

struct LifecycleState {
    broker_online: AtomicBool,
    last_event_ts: AtomicI64,
    cleanup_version: AtomicU64,
    cleanup: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl LifecycleState {
    fn new() -> Self {
        Self {
            broker_online: AtomicBool::new(false),
            last_event_ts: AtomicI64::new(0),
            cleanup_version: AtomicU64::new(0),
            cleanup: tokio::sync::Mutex::new(None),
        }
    }
}

pub struct MqttUdpService {
    runtime: DeviceRuntime,
    mqtt: MqttClientConfig,
    udp: UdpConfig,
}

impl MqttUdpService {
    pub fn new(runtime: DeviceRuntime, mqtt: MqttClientConfig, udp: UdpConfig) -> Self {
        Self {
            runtime,
            mqtt,
            udp,
        }
    }

    pub async fn start(
        self,
        shutdown: Arc<AtomicBool>,
        gateway: Arc<MqttDeviceGateway>,
    ) -> anyhow::Result<()> {
        let devices: Arc<DashMap<String, DeviceMqttSession>> = Arc::new(DashMap::new());
        let conn_map: Arc<DashMap<u32, ConnState>> = Arc::new(DashMap::new());
        let device_addrs: Arc<DashMap<String, SocketAddr>> = Arc::new(DashMap::new());
        let lifecycle_states: Arc<DashMap<String, Arc<LifecycleState>>> = Arc::new(DashMap::new());
        let rehello_cooldown: Arc<DashMap<String, Instant>> = Arc::new(DashMap::new());
        let orphan_udp_seen: Arc<DashMap<String, Instant>> = Arc::new(DashMap::new());
        let udp_ssrc_warn_cooldown: Arc<DashMap<(u32, SocketAddr), Instant>> =
            Arc::new(DashMap::new());

        let udp_addr: SocketAddr = format!("0.0.0.0:{}", self.udp.listen_port).parse()?;
        let udp_socket = Arc::new(bind_udp_with_retry(udp_addr).await?);
        tracing::info!("UDP 音频监听: {udp_addr}");

        let mqtt_cfg = self.mqtt.clone();
        let mqtt_client: Arc<RwLock<Option<AsyncClient>>> = Arc::new(RwLock::new(None));

        let ctx = Arc::new(MqttHandlerCtx {
            runtime: self.runtime.clone(),
            mqtt_client: mqtt_client.clone(),
            devices,
            conn_map,
            device_addrs,
            lifecycle_states,
            rehello_cooldown,
            orphan_udp_seen,
            udp_ssrc_warn_cooldown,
            udp_socket,
            external_host: self.udp.external_host.clone(),
            external_port: self.udp.external_port,
            offline_grace: Duration::from_secs(self.mqtt.transport_offline_grace_period_secs.max(1)),
        });
        gateway.attach(ctx.clone()).await;

        let udp_recv_task = spawn_udp_receiver(shutdown.clone(), ctx.clone());
        let transport_checker_task =
            spawn_transport_active_checker(shutdown.clone(), ctx.clone());

        'service: loop {
            if shutdown.load(Ordering::Relaxed) {
                break;
            }

            let (client, mut eventloop) = match connect_mqtt_client(&mqtt_cfg).await {
                Ok(conn) => conn,
                Err(e) => {
                    tracing::error!("MQTT 客户端连接失败: {e:#}，{MQTT_RECONNECT_DELAY:?} 后重试");
                    tokio::time::sleep(MQTT_RECONNECT_DELAY).await;
                    continue;
                }
            };
            *mqtt_client.write().await = Some(client);

            loop {
                if shutdown.load(Ordering::Relaxed) {
                    break 'service;
                }

                let mut need_reconnect = false;
                tokio::select! {
                    event = eventloop.poll() => {
                        match event {
                            Ok(Event::Incoming(Packet::Publish(p))) => {
                                let topic = p.topic.clone();
                                let payload = p.payload.to_vec();
                                let ctx_spawn = ctx.clone();
                                tokio::spawn(async move {
                                    if topic == mqtt::LIFECYCLE_TOPIC {
                                        handle_lifecycle_message(ctx_spawn, &payload).await;
                                        return;
                                    }
                                    let Some(device_id) = mqtt::device_id_from_public_topic(&topic) else {
                                        return;
                                    };
                                    let Ok(client_msg) = serde_json::from_slice::<ClientMessage>(&payload) else {
                                        return;
                                    };
                                    handle_device_message(ctx_spawn, &device_id, client_msg).await;
                                });
                            }
                            Ok(Event::Incoming(Packet::Disconnect)) => {
                                tracing::warn!("MQTT 连接断开，将重连");
                                need_reconnect = true;
                            }
                            Ok(_) => {}
                            Err(e) => {
                                tracing::warn!("MQTT 事件循环异常: {e}，将重连");
                                need_reconnect = true;
                            }
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_millis(200)) => {}
                }

                if need_reconnect {
                    *mqtt_client.write().await = None;
                    break;
                }
            }

            if shutdown.load(Ordering::Relaxed) {
                break;
            }
            tracing::info!("MQTT 客户端重连中…");
            tokio::time::sleep(MQTT_RECONNECT_DELAY).await;
        }

        tracing::info!("MQTT+UDP 服务已停止（热重载）");
        gateway.detach().await;
        udp_recv_task.abort();
        transport_checker_task.abort();
        let _ = tokio::time::timeout(Duration::from_secs(2), udp_recv_task).await;
        let _ = tokio::time::timeout(Duration::from_secs(1), transport_checker_task).await;
        // Windows 上 abort 后 UDP 端口释放可能有延迟
        tokio::time::sleep(Duration::from_millis(400)).await;

        Ok(())
    }
}

struct MqttHandlerCtx {
    runtime: DeviceRuntime,
    mqtt_client: Arc<RwLock<Option<AsyncClient>>>,
    devices: Arc<DashMap<String, DeviceMqttSession>>,
    conn_map: Arc<DashMap<u32, ConnState>>,
    device_addrs: Arc<DashMap<String, SocketAddr>>,
    lifecycle_states: Arc<DashMap<String, Arc<LifecycleState>>>,
    rehello_cooldown: Arc<DashMap<String, Instant>>,
    orphan_udp_seen: Arc<DashMap<String, Instant>>,
    udp_ssrc_warn_cooldown: Arc<DashMap<(u32, SocketAddr), Instant>>,
    udp_socket: Arc<UdpSocket>,
    external_host: String,
    external_port: u16,
    offline_grace: Duration,
}

fn spawn_udp_receiver(
    shutdown: Arc<AtomicBool>,
    ctx: Arc<MqttHandlerCtx>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        loop {
            if shutdown.load(Ordering::Relaxed) {
                break;
            }
            match ctx.udp_socket.recv_from(&mut buf).await {
                Ok((len, addr)) => {
                    let packet = &buf[..len];
                    if packet.len() < 16 {
                        continue;
                    }
                    let ssrc = u32::from_be_bytes(packet[4..8].try_into().unwrap());
                    if let Some(conn) = ctx.conn_map.get(&ssrc) {
                        let is_new = !ctx.device_addrs.contains_key(&conn.device_id);
                        ctx.device_addrs.insert(conn.device_id.clone(), addr);
                        if is_new {
                            clear_orphan_udp_seen(&ctx, &conn.device_id);
                            tracing::info!(
                                device_id = %conn.device_id,
                                %addr,
                                ssrc,
                                "MQTT UDP 远端地址已建立"
                            );
                        }
                        if conn.stale_until.is_some() {
                            tracing::debug!(
                                device_id = %conn.device_id,
                                ssrc,
                                %addr,
                                "设备仍使用 hello 轮换前的 UDP conn_id"
                            );
                        }
                        match conn.crypto.decrypt(packet) {
                            Ok((_ts, _seq, payload)) => {
                                if let Some(entry) = ctx.devices.get(&conn.device_id) {
                                    entry.touch_active();
                                }
                                if let Some(mgr) = ctx.runtime.chat_registry.get(&conn.device_id) {
                                    mgr.touch_udp_transport_active();
                                    if let Err(e) = mgr.handle_audio(&payload).await {
                                        tracing::error!("MQTT UDP 音频处理失败: {e}");
                                    }
                                }
                            }
                            Err(e) => tracing::warn!(ssrc, "UDP 解密失败: {e}"),
                        }
                    } else if packet[0] == 0x01 {
                        let now = Instant::now();
                        let warn_key = (ssrc, addr);
                        let should_warn = ctx
                            .udp_ssrc_warn_cooldown
                            .get(&warn_key)
                            .is_none_or(|t| now.duration_since(*t) >= UDP_SSRC_WARN_INTERVAL);
                        if should_warn {
                            ctx.udp_ssrc_warn_cooldown.insert(warn_key, now);
                            tracing::warn!(
                                ssrc,
                                len = packet.len(),
                                %addr,
                                "UDP 包 ssrc 未匹配当前会话（可能 hello 轮换后设备仍用旧 conn_id）"
                            );
                            if let Some(device_id) = guess_device_for_orphan_udp(&ctx, addr) {
                                mark_orphan_udp_seen(&ctx, &device_id);
                                tracing::debug!(
                                    device_id = %device_id,
                                    %addr,
                                    ssrc,
                                    "orphan UDP 包，等待设备 hello 同步，不下发 goodbye"
                                );
                            }
                        }
                    }
                }
                Err(e) => tracing::error!("UDP 接收失败: {e}"),
            }
        }
    })
}

fn device_broker_online(
    lifecycle_states: &DashMap<String, Arc<LifecycleState>>,
    device_id: &str,
) -> bool {
    lifecycle_states
        .get(device_id)
        .is_some_and(|s| s.broker_online.load(Ordering::SeqCst))
}

fn devices_missing_udp_session(ctx: &MqttHandlerCtx) -> Vec<String> {
    ctx.devices
        .iter()
        .filter(|entry| {
            device_broker_online(&ctx.lifecycle_states, entry.key())
                && active_conn_for_device(&ctx.conn_map, entry.key()).is_none()
        })
        .map(|entry| entry.key().clone())
        .collect()
}

fn guess_device_for_orphan_udp(ctx: &MqttHandlerCtx, addr: SocketAddr) -> Option<String> {
    if let Some(entry) = ctx
        .device_addrs
        .iter()
        .find(|e| *e.value() == addr)
    {
        return Some(entry.key().clone());
    }
    let missing = devices_missing_udp_session(ctx);
    match missing.len() {
        1 => Some(missing[0].clone()),
        _ => None,
    }
}

fn mark_orphan_udp_seen(ctx: &MqttHandlerCtx, device_id: &str) {
    ctx.orphan_udp_seen
        .insert(device_id.to_string(), Instant::now());
}

fn clear_orphan_udp_seen(ctx: &MqttHandlerCtx, device_id: &str) {
    ctx.orphan_udp_seen.remove(device_id);
}

fn recently_saw_orphan_udp(ctx: &MqttHandlerCtx, device_id: &str) -> bool {
    ctx.orphan_udp_seen
        .get(device_id)
        .is_some_and(|seen_at| Instant::now().duration_since(*seen_at) < ORPHAN_UDP_REHELLO_WAIT)
}

fn is_mqtt_control_without_udp_requirement(msg_type: &str) -> bool {
    matches!(
        msg_type,
        xiaozhi_core::message::HELLO
            | xiaozhi_core::message::LISTEN
            | xiaozhi_core::message::SPEAK_READY
            | xiaozhi_core::message::ABORT
            | xiaozhi_core::message::GOODBYE
            | xiaozhi_core::message::MCP
    )
}

async fn should_request_rehello_for_missing_udp(
    ctx: &MqttHandlerCtx,
    device_id: &str,
    msg_type: &str,
    chat_mgr: &Arc<ChatManager>,
) -> bool {
    if active_conn_for_device(&ctx.conn_map, device_id).is_some() {
        return false;
    }
    if is_mqtt_control_without_udp_requirement(msg_type) {
        return false;
    }
    if chat_mgr.should_protect_active_speak_flow().await {
        return false;
    }
    if chat_mgr.is_hello_inited() {
        tracing::debug!(
            device_id = %device_id,
            msg_type,
            "MQTT 无 UDP 会话但设备已 hello，等待设备重连，不下发 goodbye"
        );
        return false;
    }
    if recently_saw_orphan_udp(ctx, device_id) {
        tracing::debug!(
            device_id = %device_id,
            msg_type,
            "MQTT 无 UDP 但近期收到 orphan UDP，等待设备 hello 同步，不下发 goodbye"
        );
        return false;
    }
    if ctx.devices.contains_key(device_id)
        && device_broker_online(&ctx.lifecycle_states, device_id)
    {
        tracing::debug!(
            device_id = %device_id,
            msg_type,
            "MQTT 在线但 UDP 映射缺失（可能服务端重启），等待设备 hello，不下发 goodbye"
        );
        return false;
    }
    true
}

async fn request_device_rehello(ctx: &MqttHandlerCtx, device_id: &str, reason: &str) {
    let now = Instant::now();
    if let Some(entry) = ctx.rehello_cooldown.get(device_id) {
        if now.duration_since(*entry) < REHELLO_COOLDOWN {
            return;
        }
    }
    ctx.rehello_cooldown.insert(device_id.to_string(), now);

    let topic = mqtt::device_sub_topic(device_id);
    let msg = ServerMessage::goodbye(None);
    let Ok(data) = serde_json::to_vec(&msg) else {
        return;
    };
    if let Some(client) = ctx.mqtt_client.read().await.as_ref() {
        if client
            .publish(topic, MQTT_CMD_QOS, false, data)
            .await
            .is_ok()
        {
            tracing::info!(
                device_id = %device_id,
                reason,
                "已下发 goodbye，请求设备重新 hello 以同步 UDP 会话"
            );
        }
    }
}

async fn bind_udp_with_retry(addr: SocketAddr) -> anyhow::Result<UdpSocket> {
    const MAX_ATTEMPTS: u32 = 8;
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 1..=MAX_ATTEMPTS {
        match UdpSocket::bind(addr).await {
            Ok(socket) => return Ok(socket),
            Err(e) => {
                last_err = Some(e.into());
                if attempt < MAX_ATTEMPTS {
                    tokio::time::sleep(Duration::from_millis(250 * attempt as u64)).await;
                }
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("UDP bind 失败")))
}

fn build_mqtt_client(cfg: &MqttClientConfig) -> anyhow::Result<(AsyncClient, EventLoop)> {
    let mut opts = MqttOptions::new(cfg.client_id.clone(), cfg.broker.clone(), cfg.port);
    if !cfg.username.is_empty() {
        opts.set_credentials(cfg.username.clone(), cfg.password.clone());
    }
    // 设备对话可能阻塞 handler 较久；适当拉长 keepalive，降低 Broker 误判超时概率
    opts.set_keep_alive(Duration::from_secs(60));
    let transport = cfg.r#type.to_ascii_lowercase();
    if transport == "tls" || transport == "ssl" {
        opts.set_transport(Transport::tls_with_default_config());
    }
    Ok(AsyncClient::new(opts, 64))
}

async fn connect_mqtt_client(cfg: &MqttClientConfig) -> anyhow::Result<(AsyncClient, EventLoop)> {
    let (client, eventloop) = build_mqtt_client(cfg)?;
    client.subscribe(mqtt::SERVER_SUB_TOPIC, MQTT_CMD_QOS).await?;
    client.subscribe(mqtt::LIFECYCLE_TOPIC, MQTT_CMD_QOS).await?;
    tracing::info!(
        "MQTT 客户端已连接 {}:{} (type={})",
        cfg.broker,
        cfg.port,
        cfg.r#type
    );
    Ok((client, eventloop))
}

fn lifecycle_state(states: &DashMap<String, Arc<LifecycleState>>, device_id: &str) -> Arc<LifecycleState> {
    states
        .entry(device_id.to_string())
        .or_insert_with(|| Arc::new(LifecycleState::new()))
        .clone()
}

async fn handle_lifecycle_message(ctx: Arc<MqttHandlerCtx>, payload: &[u8]) {
    let Ok(event) = serde_json::from_slice::<MqttLifecycleEvent>(payload) else {
        tracing::warn!("解析 MQTT lifecycle 失败");
        return;
    };
    let device_id = event.device_id.trim();
    if device_id.is_empty() {
        return;
    }
    match event.state.as_str() {
        mqtt::lifecycle::ONLINE => {
            let state = lifecycle_state(&ctx.lifecycle_states, device_id);
            if mark_device_online(&state, event.ts) {
                cancel_lifecycle_cleanup(&state).await;
            }
            if let Ok(chat_mgr) = ctx.runtime.ensure_chat_manager(device_id).await {
                ensure_device_session(ctx.clone(), device_id, chat_mgr.clone()).await;
                if let Some(entry) = ctx.devices.get(device_id) {
                    entry.mark_broker_online();
                }
                chat_mgr.handle_mqtt_transport_ready().await;
                // 设备上线后会自行 hello 建立 UDP；勿在此处发 goodbye，否则打断唤醒
            }
        }
        mqtt::lifecycle::OFFLINE => {
            let state = lifecycle_state(&ctx.lifecycle_states, device_id);
            let version = mark_device_offline(&state, event.ts);
            if version == 0 {
                return;
            }
            if let Some(entry) = ctx.devices.get(device_id) {
                entry.mark_broker_offline(ctx.offline_grace);
            }
            schedule_offline_cleanup(ctx, device_id.to_string(), state, version).await;
        }
        other => tracing::warn!(device_id = %device_id, state = %other, "未知 lifecycle 状态"),
    }
}

fn mark_device_online(state: &LifecycleState, event_ts: i64) -> bool {
    let ts = if event_ts > 0 {
        event_ts
    } else {
        chrono::Utc::now().timestamp_millis()
    };
    let last = state.last_event_ts.load(Ordering::SeqCst);
    if last > 0 && ts < last {
        return false;
    }
    if ts > last {
        state.last_event_ts.store(ts, Ordering::SeqCst);
    }
    let was = state.broker_online.swap(true, Ordering::SeqCst);
    state.cleanup_version.fetch_add(1, Ordering::SeqCst);
    !was
}

fn mark_device_offline(state: &LifecycleState, event_ts: i64) -> u64 {
    let ts = if event_ts > 0 {
        event_ts
    } else {
        chrono::Utc::now().timestamp_millis()
    };
    let last = state.last_event_ts.load(Ordering::SeqCst);
    if last > 0 && ts < last {
        return 0;
    }
    if ts > last {
        state.last_event_ts.store(ts, Ordering::SeqCst);
    }
    state.broker_online.store(false, Ordering::SeqCst);
    state.cleanup_version.fetch_add(1, Ordering::SeqCst) + 1
}

async fn cancel_lifecycle_cleanup(state: &LifecycleState) {
    let mut guard = state.cleanup.lock().await;
    if let Some(handle) = guard.take() {
        handle.abort();
    }
}

async fn schedule_offline_cleanup(
    ctx: Arc<MqttHandlerCtx>,
    device_id: String,
    state: Arc<LifecycleState>,
    version: u64,
) {
    let grace = ctx.offline_grace;
    cancel_lifecycle_cleanup(&state).await;
    let state_for_task = state.clone();
    let handle = tokio::spawn(async move {
        tokio::time::sleep(grace).await;
        if state_for_task.broker_online.load(Ordering::SeqCst) {
            return;
        }
        if state_for_task.cleanup_version.load(Ordering::SeqCst) != version {
            return;
        }
        destroy_device_transport(&ctx, &device_id, "broker_offline_grace").await;
        ctx.lifecycle_states.remove(&device_id);
        tracing::info!(device_id = %device_id, "MQTT 离线宽限期结束，transport 已销毁");
    });
    *state.cleanup.lock().await = Some(handle);
}

async fn handle_device_message(
    ctx: Arc<MqttHandlerCtx>,
    device_id: &str,
    client_msg: ClientMessage,
) {
    let state = lifecycle_state(&ctx.lifecycle_states, device_id);
    let notify = mark_device_online(&state, chrono::Utc::now().timestamp_millis());

    let chat_mgr = match ctx.runtime.ensure_chat_manager(device_id).await {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("创建 ChatManager 失败 device={device_id}: {e}");
            return;
        }
    };

    ensure_device_session(ctx.clone(), device_id, chat_mgr.clone()).await;
    chat_mgr
        .record_inbound_client("mqtt", &client_msg)
        .await;
    if let Some(entry) = ctx.devices.get(device_id) {
        entry.touch_active();
        entry.mark_broker_online();
    }
    if notify {
        chat_mgr.handle_mqtt_transport_ready().await;
    }

    // speak_request / TTS 播报期间 UDP 可能尚未写入 conn_map；此时下发 goodbye 会打断播放。
    // 服务端重启后设备仍用旧 UDP ssrc 时，应等待设备 hello，勿对普通 MQTT 信令发 goodbye。
    let msg_type = client_msg.msg_type.as_str();
    if msg_type != xiaozhi_core::message::HELLO
        && should_request_rehello_for_missing_udp(&ctx, device_id, msg_type, &chat_mgr).await
    {
        request_device_rehello(&ctx, device_id, "mqtt_without_udp_session").await;
    }

    if client_msg.msg_type.as_str() == xiaozhi_core::message::HELLO {
        clear_orphan_udp_seen(&ctx, device_id);
        if !chat_mgr.should_preserve_udp_session_on_hello().await {
            rotate_udp_session(&ctx, device_id).await;
        } else {
            tracing::info!(
                device_id = %device_id,
                "主动播报进行中，保留现有 UDP 会话"
            );
        }
        chat_mgr.set_udp_binding_active(true);
        chat_mgr.on_hardware_hello_received();
        chat_mgr.refresh_speak_path_warm_from_transport().await;
    }

    if client_msg.msg_type.as_str() == xiaozhi_core::message::LISTEN {
        let _ = chat_mgr
            .handle_listen_message(
                client_msg.state.as_deref(),
                client_msg.mode.as_deref(),
                client_msg.text.as_deref(),
            )
            .await;
        return;
    }

    let mut udp_hello: Option<UdpHelloInfo> = None;
    if client_msg.msg_type.as_str() == xiaozhi_core::message::HELLO {
        if let Some(entry) = ctx.devices.get(device_id) {
            if let Some(ref session) = entry.udp {
                udp_hello = Some(UdpHelloInfo {
                    server_host: ctx.external_host.clone(),
                    server_port: ctx.external_port,
                    key: session.key,
                    nonce: session.nonce,
                });
            }
        }
    }

    let responses = process_client_message(&chat_mgr, client_msg, udp_hello.as_ref()).await;
    if let Some(entry) = ctx.devices.get(device_id) {
        for resp in responses {
            if let Ok(data) = serde_json::to_vec(&resp) {
                let _ = entry.out_tx.send(OutboundFrame::Command(data));
            }
        }
    }
}

async fn ensure_device_session(
    ctx: Arc<MqttHandlerCtx>,
    device_id: &str,
    chat_mgr: Arc<ChatManager>,
) {
    if ctx.devices.contains_key(device_id) {
        return;
    }

    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<OutboundFrame>();
    let (udp_audio_tx, udp_audio_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    chat_mgr.set_mqtt_transport(true);
    chat_mgr.set_outbound(out_tx.clone()).await;

    let mqtt_client = ctx.mqtt_client.clone();
    let conn_map = ctx.conn_map.clone();
    let device_addrs = ctx.device_addrs.clone();
    let devices_cleanup = ctx.devices.clone();
    let udp_socket = ctx.udp_socket.clone();
    let device_id_owned = device_id.to_string();
    let mac = mqtt::mac_underscore_from_device_id(device_id);
    let chat_mgr_for_audio = chat_mgr.clone();
    let device_addrs_for_outbound = device_addrs.clone();

    let udp_send_state = Arc::new(UdpSendState::new());
    let devices_for_udp = ctx.devices.clone();
    spawn_udp_audio_sender(
        udp_audio_rx,
        conn_map.clone(),
        device_addrs.clone(),
        devices_for_udp,
        udp_socket.clone(),
        device_id_owned.clone(),
        Arc::clone(&udp_send_state),
    );

    let ctx_for_cleanup = ctx.clone();
    tokio::spawn(async move {
        let mut udp_grace_until: Option<Instant> = None;
        let mut logged_udp_send = false;
        while let Some(frame) = out_rx.recv().await {
            match frame {
                OutboundFrame::Command(data) => {
                    let tts_start = is_tts_start_command(&data);
                    if tts_start {
                        let udp_ready = device_addrs_for_outbound.contains_key(&device_id_owned);
                        if !udp_ready {
                            let ready = wait_udp_remote_addr(
                                &device_addrs_for_outbound,
                                &device_id_owned,
                                Duration::from_secs(5),
                            )
                            .await;
                            if !ready {
                                tracing::warn!(
                                    device_id = %device_id_owned,
                                    "tts start 前 UDP 远端地址未就绪，仍将下发信令"
                                );
                            }
                        }
                    }
                    let topic = mqtt::device_sub_topic_mac(&mac);
                    let publish_ok = match mqtt_client.read().await.as_ref() {
                        Some(client) => {
                            client
                                .publish(topic, MQTT_CMD_QOS, false, data)
                                .await
                                .is_ok()
                        }
                        None => false,
                    };
                    if !publish_ok {
                        break;
                    }
                    if let Some(entry) = devices_cleanup.get(&device_id_owned) {
                        entry.touch_active();
                    }
                    if tts_start {
                        let udp_ready = device_addrs_for_outbound.contains_key(&device_id_owned);
                        // 给 broker/设备处理 tts start 留一点网络余量，再开始 UDP grace 倒计时。
                        tokio::time::sleep(Duration::from_millis(if udp_ready { 100 } else { 200 }))
                            .await;
                        let grace = if udp_ready {
                            MQTT_TTS_START_UDP_GRACE_WARM
                        } else {
                            MQTT_TTS_START_UDP_GRACE
                        };
                        udp_grace_until = Some(Instant::now() + grace);
                        logged_udp_send = false;
                        tracing::info!(
                            device_id = %device_id_owned,
                            udp_ready,
                            grace_ms = grace.as_millis(),
                            "MQTT tts start 信令已下发，等待设备进入 Speaking"
                        );
                    }
                }
                OutboundFrame::Audio(data) => {
                    if let Some(deadline) = udp_grace_until.take() {
                        let now = Instant::now();
                        if now < deadline {
                            tokio::time::sleep(deadline - now).await;
                        }
                    }
                    // ESP32 `mqtt_protocol.cc` UDP 载荷为裸 Opus；TTS 层按 WS BinaryProtocol 做了 v2/v3 打包，此处还原。
                    let proto = chat_mgr_for_audio.binary_protocol_version();
                    let opus = xiaozhi_protocol::unpack_device_audio(&data, proto);
                    if !logged_udp_send {
                        logged_udp_send = true;
                        tracing::info!(
                            device_id = %device_id_owned,
                            opus_bytes = opus.len(),
                            "MQTT UDP TTS 首帧已入发送队列"
                        );
                    }
                    if udp_audio_tx.send(opus.to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
        destroy_device_transport(&ctx_for_cleanup, &device_id_owned, "outbound_closed").await;
    });

    ctx.devices.insert(
        device_id.to_string(),
        DeviceMqttSession {
            chat_mgr,
            out_tx,
            udp: None,
            udp_send_state,
            last_active_ts: AtomicI64::new(chrono::Utc::now().timestamp()),
            retained_until_ns: AtomicI64::new(0),
            broker_online: AtomicBool::new(true),
        },
    );
}

fn spawn_udp_audio_sender(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    conn_map: Arc<DashMap<u32, ConnState>>,
    device_addrs: Arc<DashMap<String, SocketAddr>>,
    devices: Arc<DashMap<String, DeviceMqttSession>>,
    udp_socket: Arc<UdpSocket>,
    device_id: String,
    send_state: Arc<UdpSendState>,
) {
    tokio::spawn(async move {
        let mut logged_first_send = false;
        let mut logged_session_gen = send_state.session_generation();
        while let Some(data) = rx.recv().await {
            let deadline = Instant::now() + UDP_REMOTE_ADDR_WAIT;
            let mut sent = false;
            while Instant::now() < deadline {
                let conn_key = active_conn_for_device(&conn_map, &device_id);
                let conn = conn_key.and_then(|k| conn_map.get(&k));
                let addr = device_addrs.get(&device_id);
                match (conn, addr) {
                    (Some(conn), Some(addr)) => {
                        let session_gen = send_state.session_generation();
                        if session_gen != logged_session_gen {
                            logged_session_gen = session_gen;
                            logged_first_send = false;
                        }
                        let sequence = send_state.next_sequence();
                        let encrypted = conn.crypto.encrypt(sequence, &data);
                        let target = *addr;
                        match udp_socket.send_to(&encrypted, target).await {
                            Ok(n) => {
                                sent = true;
                                if let Some(entry) = devices.get(&device_id) {
                                    entry.touch_active();
                                }
                                if !logged_first_send {
                                    logged_first_send = true;
                                    tracing::info!(
                                        device_id = %device_id,
                                        %target,
                                        bytes = n,
                                        sequence,
                                        payload_bytes = data.len(),
                                        conn_id = conn.conn_id,
                                        session_gen,
                                        "MQTT UDP TTS 首包已发送"
                                    );
                                } else {
                                    tracing::trace!(
                                        device_id = %device_id,
                                        %target,
                                        bytes = n,
                                        sequence,
                                        "MQTT UDP 音频已发送"
                                    );
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    device_id = %device_id,
                                    addr = %target,
                                    "MQTT UDP 音频发送失败: {e}"
                                );
                            }
                        }
                        break;
                    }
                    (None, _) => {
                        tracing::trace!(
                            device_id = %device_id,
                            "等待 UDP 会话（conn_map）"
                        );
                    }
                    (Some(_), None) => {
                        tracing::trace!(
                            device_id = %device_id,
                            "等待设备 UDP 首包以建立远端地址"
                        );
                    }
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            if !sent {
                tracing::warn!(
                    device_id = %device_id,
                    bytes = data.len(),
                    wait_secs = UDP_REMOTE_ADDR_WAIT.as_secs(),
                    has_conn = active_conn_for_device(&conn_map, &device_id).is_some(),
                    has_addr = device_addrs.contains_key(&device_id),
                    "UDP 远端地址未建立，TTS 音频被丢弃"
                );
            }
        }
    });
}

fn active_conn_for_device(
    conn_map: &DashMap<u32, ConnState>,
    device_id: &str,
) -> Option<u32> {
    conn_map
        .iter()
        .find(|e| e.device_id == device_id && e.stale_until.is_none())
        .map(|e| *e.key())
}

fn mark_device_conns_stale(conn_map: &DashMap<u32, ConnState>, device_id: &str) {
    let stale_until = Instant::now() + UDP_CONN_STALE_GRACE;
    for mut entry in conn_map.iter_mut() {
        if entry.device_id == device_id && entry.stale_until.is_none() {
            entry.stale_until = Some(stale_until);
        }
    }
}

fn purge_expired_conns(conn_map: &DashMap<u32, ConnState>) {
    let now = Instant::now();
    conn_map.retain(|_, v| v.stale_until.is_none_or(|t| t > now));
}

async fn rotate_udp_session(ctx: &MqttHandlerCtx, device_id: &str) {
    mark_device_conns_stale(&ctx.conn_map, device_id);
    purge_expired_conns(&ctx.conn_map);
    ctx.device_addrs.remove(device_id);
    let session = DeviceRuntime::new_udp_session(device_id);
    let conn_id = session.conn_id;
    ctx.conn_map.insert(
        conn_id,
        ConnState {
            device_id: device_id.to_string(),
            crypto: UdpCrypto::new(session.key, session.nonce),
            conn_id,
            stale_until: None,
        },
    );
    if let Some(mut entry) = ctx.devices.get_mut(device_id) {
        entry.udp = Some(session);
        entry.udp_send_state.reset();
    }
    tracing::info!(
        device_id = %device_id,
        conn_id,
        "hello 轮换 UDP 会话（旧 conn 保留 {}s）",
        UDP_CONN_STALE_GRACE.as_secs()
    );
}

fn conn_map_retain_device(conn_map: &DashMap<u32, ConnState>, device_id: &str) {
    conn_map.retain(|_, v| v.device_id != device_id);
}

fn cleanup_device_session(
    device_id: &str,
    devices: &DashMap<String, DeviceMqttSession>,
    conn_map: &DashMap<u32, ConnState>,
    device_addrs: &DashMap<String, SocketAddr>,
) {
    conn_map_retain_device(conn_map, device_id);
    device_addrs.remove(device_id);
    devices.remove(device_id);
}

async fn destroy_device_transport(ctx: &Arc<MqttHandlerCtx>, device_id: &str, reason: &str) {
    if let Some((_, session)) = ctx.devices.remove(device_id) {
        session.chat_mgr.set_udp_binding_active(false);
        let hub_empty = session.chat_mgr.unregister_endpoint("hardware");
        if !hub_empty && !session.chat_mgr.has_hardware_endpoint() {
            session.chat_mgr.set_mqtt_transport(false);
        }
        drop(session.out_tx);
        if hub_empty {
            ctx.runtime
                .chat_registry
                .remove_and_shutdown(device_id)
                .await;
        }
    }
    cleanup_device_session(
        device_id,
        &ctx.devices,
        &ctx.conn_map,
        &ctx.device_addrs,
    );
    tracing::info!(device_id = %device_id, reason, "MQTT transport 已销毁");
}

fn spawn_transport_active_checker(
    shutdown: Arc<AtomicBool>,
    ctx: Arc<MqttHandlerCtx>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(TRANSPORT_ACTIVE_CHECK_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            if shutdown.load(Ordering::Relaxed) {
                break;
            }
            interval.tick().await;
            let stale: Vec<String> = ctx
                .devices
                .iter()
                .filter(|entry| !entry.is_active())
                .map(|entry| entry.key().clone())
                .collect();
            for device_id in stale {
                destroy_device_transport(&ctx, &device_id, "transport_idle").await;
            }
        }
    })
}
