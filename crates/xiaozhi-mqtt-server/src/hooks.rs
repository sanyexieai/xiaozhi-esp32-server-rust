//! RMQTT Hook：对齐 Go DeviceHook + AuthHook

use std::net::Ipv4Addr;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use dashmap::{DashMap, DashSet};
use rmqtt::codec::types::{Publish as CodecPublish, QoS};
use rmqtt::codec::v5::SubscribeAckReason;
use rmqtt::context::ServerContext;
use rmqtt::hook::{Handler, HookResult, Parameter, Type};
use rmqtt::types::{
    AuthResult, From, Id, Publish, PublishAclResult, Subscribe, SubscribeAclResult,
};
use xiaozhi_config::MqttServerConfig;
use xiaozhi_protocol::mqtt;

use crate::device;

pub struct HookState {
    pub config: MqttServerConfig,
    pub admin_username: String,
    pub admin_password: String,
    by_client_id: DashMap<String, Id>,
    superseded: DashSet<Id>,
}

impl HookState {
    pub fn new(config: MqttServerConfig) -> Self {
        let admin_username = configured_admin_username(&config);
        let admin_password = configured_admin_password(&config);
        Self {
            config,
            admin_username,
            admin_password,
            by_client_id: DashMap::new(),
            superseded: DashSet::new(),
        }
    }
}

#[derive(Clone)]
pub struct XiaozhiHookHandler {
    state: Arc<HookState>,
}

impl XiaozhiHookHandler {
    pub fn new(state: Arc<HookState>) -> Self {
        Self { state }
    }
}

fn configured_admin_username(cfg: &MqttServerConfig) -> String {
    let u = cfg.username.trim();
    if u.is_empty() {
        "admin".to_string()
    } else {
        u.to_string()
    }
}

fn configured_admin_password(cfg: &MqttServerConfig) -> String {
    let p = cfg.password.trim();
    if p.is_empty() {
        "test!@#".to_string()
    } else {
        p.to_string()
    }
}

fn build_publish(topic: &str, payload: &[u8]) -> Publish {
    let inner = CodecPublish {
        dup: false,
        retain: false,
        qos: QoS::AtMostOnce,
        topic: topic.into(),
        packet_id: None,
        payload: Bytes::copy_from_slice(payload),
        properties: None,
    };
    Publish::from(inner)
}

async fn publish_lifecycle(scx: &ServerContext, state: &str, device_id: &str, client_id: &str) {
    let payload = mqtt::lifecycle_payload(state, device_id, client_id);
    let publish = build_publish(mqtt::LIFECYCLE_TOPIC, &payload);
    let from = From::from_system(Id::from(
        scx.node.id,
        "xiaozhi_broker".into(),
    ));
    if let Err(e) = scx.extends.shared().await.forwards(from, publish).await {
        tracing::warn!(?e, "MQTT lifecycle 广播失败");
    }
}

fn client_id_str(session: &rmqtt::session::Session) -> String {
    session.id.client_id.to_string()
}

#[async_trait]
impl Handler for XiaozhiHookHandler {
    async fn hook(&self, param: &Parameter, acc: Option<HookResult>) -> (bool, Option<HookResult>) {
        match param {
            Parameter::ClientAuthenticate(connect_info) => {
                let username = connect_info
                    .username()
                    .map(|u| u.to_string())
                    .unwrap_or_default();
                let password = connect_info
                    .password()
                    .map(|p| String::from_utf8_lossy(p.as_ref()).into_owned())
                    .unwrap_or_default();
                let client_id = connect_info.client_id().to_string();
                let cfg = &self.state.config;

                match xiaozhi_auth::verify_mqtt_broker_connect(
                    &client_id,
                    &username,
                    &password,
                    cfg.enable_auth,
                    cfg.signature_key.trim(),
                    &self.state.admin_username,
                    &self.state.admin_password,
                ) {
                    Ok(true) => {
                        let is_admin = device::is_admin_user(&username, &self.state.admin_username);
                        (
                            false,
                            Some(HookResult::AuthResult(AuthResult::Allow(is_admin, None))),
                        )
                    }
                    Ok(false) => (
                        false,
                        Some(HookResult::AuthResult(AuthResult::BadUsernameOrPassword)),
                    ),
                    Err(e) => {
                        tracing::warn!(client_id = %client_id, "MQTT 认证错误: {e}");
                        (
                            false,
                            Some(HookResult::AuthResult(AuthResult::BadUsernameOrPassword)),
                        )
                    }
                }
            }
            Parameter::ClientSubscribeCheckAcl(session, subscribe) => {
                let client_id = client_id_str(session);
                let is_admin = session.superuser().await.unwrap_or(false);
                if device::acl_allow_subscribe(&client_id, &subscribe.topic_filter, is_admin) {
                    (
                        false,
                        Some(HookResult::SubscribeAclResult(
                            SubscribeAclResult::new_success(subscribe.opts.qos(), None),
                        )),
                    )
                } else {
                    tracing::warn!(
                        client_id = %client_id,
                        filter = %subscribe.topic_filter,
                        "MQTT 订阅被拒绝"
                    );
                    (
                        false,
                        Some(HookResult::SubscribeAclResult(
                            SubscribeAclResult::new_failure(SubscribeAckReason::NotAuthorized),
                        )),
                    )
                }
            }
            Parameter::MessagePublishCheckAcl(session, publish) => {
                let client_id = client_id_str(session);
                let is_admin = session.superuser().await.unwrap_or(false);
                if device::acl_allow_publish(&client_id, &publish.topic, is_admin) {
                    (
                        false,
                        Some(HookResult::PublishAclResult(PublishAclResult::allow())),
                    )
                } else {
                    tracing::warn!(
                        client_id = %client_id,
                        topic = %publish.topic,
                        "MQTT 发布被拒绝"
                    );
                    (
                        false,
                        Some(HookResult::PublishAclResult(PublishAclResult::rejected(
                            false,
                            None,
                        ))),
                    )
                }
            }
            Parameter::MessagePublish(Some(session), _from, publish) => {
                let client_id = client_id_str(session);
                let is_admin = session.superuser().await.unwrap_or(false);
                let new_topic =
                    device::rewrite_device_publish_topic(&client_id, &publish.topic, is_admin);
                if publish.topic.as_ref() == new_topic {
                    (true, acc)
                } else {
                    let rewritten = Publish::new(
                        Box::new(CodecPublish {
                            dup: publish.dup,
                            retain: publish.retain,
                            qos: publish.qos,
                            topic: new_topic.into(),
                            packet_id: publish.packet_id,
                            payload: publish.payload.clone(),
                            properties: publish.properties.clone(),
                        }),
                        publish.target_clientid.clone(),
                        publish.delay_interval,
                        publish.create_time,
                    );
                    (false, Some(HookResult::Publish(rewritten)))
                }
            }
            Parameter::ClientConnected(session) => {
                let client_id = client_id_str(session);
                if let Some(old_id) = self
                    .state
                    .by_client_id
                    .insert(client_id.clone(), session.id.clone())
                {
                    self.state.superseded.insert(old_id);
                }

                if session.superuser().await.unwrap_or(false) {
                    return (true, acc);
                }

                if let Some(topic) = device::auto_subscribe_topic(&client_id) {
                    match Subscribe::from_v3(&topic.into(), QoS::AtLeastOnce, false, false) {
                        Ok(sub) => {
                            let entry = session.scx.extends.shared().await.entry(session.id.clone());
                            if let Err(e) = entry.subscribe(&sub).await {
                                tracing::warn!(client_id = %client_id, "MQTT 自动订阅失败: {e}");
                            }
                        }
                        Err(e) => {
                            tracing::warn!(client_id = %client_id, "MQTT 自动订阅 topic 无效: {e}");
                        }
                    }
                }

                if let Some(device_id) = mqtt::device_id_from_client_id(&client_id) {
                    publish_lifecycle(
                        &session.scx,
                        mqtt::lifecycle::ONLINE,
                        &device_id,
                        &client_id,
                    )
                    .await;
                    tracing::info!(device_id = %device_id, "MQTT 设备上线 (RMQTT)");
                }

                (true, acc)
            }
            Parameter::ClientDisconnected(session, _reason) => {
                let client_id = client_id_str(session);
                if self.state.superseded.remove(&session.id).is_some() {
                    return (true, acc);
                }
                self.state
                    .by_client_id
                    .retain(|_, id| id != &session.id);

                if session.superuser().await.unwrap_or(false) {
                    return (true, acc);
                }

                if let Some(device_id) = mqtt::device_id_from_client_id(&client_id) {
                    publish_lifecycle(
                        &session.scx,
                        mqtt::lifecycle::OFFLINE,
                        &device_id,
                        &client_id,
                    )
                    .await;
                    tracing::info!(device_id = %device_id, "MQTT 设备离线 (RMQTT)");
                }

                (true, acc)
            }
            _ => (true, acc),
        }
    }
}

pub async fn register_hooks(scx: &ServerContext, state: Arc<HookState>) {
    let register = scx.extends.hook_mgr().register();
    let handler = Box::new(XiaozhiHookHandler::new(state));
    register
        .add(Type::ClientAuthenticate, handler.clone())
        .await;
    register
        .add(Type::ClientSubscribeCheckAcl, handler.clone())
        .await;
    register
        .add(Type::MessagePublishCheckAcl, handler.clone())
        .await;
    register.add(Type::MessagePublish, handler.clone()).await;
    register.add(Type::ClientConnected, handler.clone()).await;
    register.add(Type::ClientDisconnected, handler).await;
    register.start().await;
}

pub fn parse_listen_addr(host: &str, port: u16) -> std::net::SocketAddr {
    let host = host.trim();
    if host.is_empty() || host == "0.0.0.0" {
        return (Ipv4Addr::UNSPECIFIED, port).into();
    }
    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        return (ip, port).into();
    }
    (Ipv4Addr::UNSPECIFIED, port).into()
}
