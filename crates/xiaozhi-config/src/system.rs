use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub chat: ChatConfig,
    #[serde(default)]
    pub chat_hooks: ChatHooksConfig,
    #[serde(default)]
    pub config_provider: ConfigProviderConfig,
    #[serde(default)]
    pub manager: ManagerConfig,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub log: LogConfig,
    #[serde(default)]
    pub redis: RedisConfig,
    #[serde(default)]
    pub websocket: WebsocketConfig,
    #[serde(default)]
    pub mqtt: MqttClientConfig,
    #[serde(default)]
    pub mqtt_server: MqttServerConfig,
    #[serde(default)]
    pub udp: UdpConfig,
    #[serde(default)]
    pub resource_pools: ResourcePoolConfig,
    #[serde(default)]
    pub vad: ProviderSection,
    #[serde(default)]
    pub asr: ProviderSection,
    #[serde(default)]
    pub tts: ProviderSection,
    #[serde(default)]
    pub llm: ProviderSection,
    #[serde(default)]
    pub vision: VisionConfig,
    #[serde(default)]
    pub ota: OtaConfig,
    #[serde(default)]
    pub mcp: McpConfig,
    #[serde(default)]
    pub local_mcp: LocalMcpConfig,
    #[serde(default)]
    pub memory: ProviderSection,
    #[serde(default)]
    pub voice_identify: VoiceIdentifyConfig,
    #[serde(default)]
    pub enable_greeting: bool,
    #[serde(default)]
    pub greeting_list: Vec<String>,
    #[serde(default)]
    pub wakeup_words: Vec<String>,
    #[serde(default)]
    pub knowledge: KnowledgeConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            auth: AuthConfig::default(),
            chat: ChatConfig::default(),
            chat_hooks: ChatHooksConfig::default(),
            config_provider: ConfigProviderConfig::default(),
            manager: ManagerConfig::default(),
            system_prompt: String::new(),
            log: LogConfig::default(),
            redis: RedisConfig::default(),
            websocket: WebsocketConfig::default(),
            mqtt: MqttClientConfig::default(),
            mqtt_server: MqttServerConfig::default(),
            udp: UdpConfig::default(),
            resource_pools: ResourcePoolConfig::default(),
            vad: ProviderSection::default(),
            asr: ProviderSection::default(),
            tts: ProviderSection::default(),
            llm: ProviderSection::default(),
            vision: VisionConfig::default(),
            ota: OtaConfig::default(),
            mcp: McpConfig::default(),
            local_mcp: LocalMcpConfig::default(),
            memory: ProviderSection::default(),
            voice_identify: VoiceIdentifyConfig::default(),
            enable_greeting: false,
            greeting_list: vec![],
            wakeup_words: vec![],
            knowledge: KnowledgeConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ServerConfig {
    #[serde(default)]
    pub pprof: PprofConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PprofConfig {
    #[serde(default)]
    pub enable: bool,
    #[serde(default = "default_pprof_port")]
    pub port: u16,
}

fn default_pprof_port() -> u16 {
    6060
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    #[serde(default = "default_true")]
    pub enable: bool,
    #[serde(default = "default_true")]
    pub login_captcha_enabled: bool,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            enable: true,
            login_captcha_enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatConfig {
    #[serde(default = "default_max_idle")]
    pub max_idle_duration: u64,
    #[serde(default = "default_silence")]
    pub chat_max_silence_duration: u64,
    #[serde(default = "default_speak_reuse")]
    pub speak_request_reuse_window_ms: u64,
    #[serde(default = "default_speak_ready_timeout")]
    pub speak_ready_timeout_ms: u64,
    #[serde(default = "default_retained_session_idle")]
    pub retained_session_idle_timeout_ms: u64,
    #[serde(default = "default_realtime_mode")]
    pub realtime_mode: u8,
    #[serde(default)]
    pub global_system_prompt: String,
}

impl Default for ChatConfig {
    fn default() -> Self {
        Self {
            max_idle_duration: default_max_idle(),
            chat_max_silence_duration: default_silence(),
            speak_request_reuse_window_ms: default_speak_reuse(),
            speak_ready_timeout_ms: default_speak_ready_timeout(),
            retained_session_idle_timeout_ms: default_retained_session_idle(),
            realtime_mode: default_realtime_mode(),
            global_system_prompt: String::new(),
        }
    }
}

fn default_max_idle() -> u64 {
    30000
}
fn default_silence() -> u64 {
    400
}
fn default_speak_reuse() -> u64 {
    60000
}
fn default_speak_ready_timeout() -> u64 {
    15000
}
fn default_retained_session_idle() -> u64 {
    600_000
}
fn default_realtime_mode() -> u8 {
    4
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatHooksConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, rename = "async")]
    pub async_config: HookAsyncConfig,
    #[serde(default)]
    pub plugins: HashMap<String, HookPluginConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HookAsyncConfig {
    #[serde(default = "default_queue_size")]
    pub queue_size: usize,
    #[serde(default = "default_worker_count")]
    pub worker_count: usize,
    #[serde(default)]
    pub drop_when_full: bool,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_queue_size() -> usize {
    1024
}
fn default_worker_count() -> usize {
    1
}
fn default_timeout_ms() -> u64 {
    200
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HookPluginConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub priority: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigProviderConfig {
    #[serde(default = "default_config_provider_type")]
    pub r#type: String,
    #[serde(default = "default_true")]
    pub enable_periodic_update: bool,
    #[serde(default = "default_update_interval")]
    pub update_interval: String,
}

impl Default for ConfigProviderConfig {
    fn default() -> Self {
        Self {
            r#type: default_config_provider_type(),
            enable_periodic_update: true,
            update_interval: default_update_interval(),
        }
    }
}

fn default_config_provider_type() -> String {
    "manager".to_string()
}
fn default_update_interval() -> String {
    "5m".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagerConfig {
    #[serde(default = "default_backend_url")]
    pub backend_url: String,
    #[serde(default = "default_auth_token")]
    pub auth_token: String,
    #[serde(default = "default_endpoint_auth_token")]
    pub endpoint_auth_token: String,
    #[serde(default = "default_history_timeout")]
    pub history_timeout: String,
    /// Manager 拉取设备配置失败时是否回退到 config.yaml（默认 true，便于本地开发）
    #[serde(default = "default_true")]
    pub fallback_to_local_config: bool,
}

impl Default for ManagerConfig {
    fn default() -> Self {
        Self {
            backend_url: default_backend_url(),
            auth_token: default_auth_token(),
            endpoint_auth_token: default_endpoint_auth_token(),
            history_timeout: default_history_timeout(),
            fallback_to_local_config: default_true(),
        }
    }
}

fn default_backend_url() -> String {
    "http://127.0.0.1:8080".to_string()
}
fn default_auth_token() -> String {
    "xiaozhi_admin_secret_key".to_string()
}
fn default_endpoint_auth_token() -> String {
    "xiaozhi_mcp_openclaw_secret_key".to_string()
}
fn default_history_timeout() -> String {
    "5s".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LogConfig {
    #[serde(default)]
    pub path: String,
    #[serde(default = "default_log_file")]
    pub file: String,
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default = "default_max_age")]
    pub max_age: u32,
    #[serde(default = "default_rotation_time")]
    pub rotation_time: u32,
    #[serde(default = "default_true")]
    pub stdout: bool,
    /// 是否写入 SQLite 日志表
    #[serde(default)]
    pub database: bool,
    /// 日志库路径（空则使用 data/logs/{service}.db）
    #[serde(default)]
    pub database_path: String,
}

fn default_log_file() -> String {
    "server.log".to_string()
}
fn default_log_level() -> String {
    "info".to_string()
}
fn default_max_age() -> u32 {
    3
}
fn default_rotation_time() -> u32 {
    10
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RedisConfig {
    #[serde(default = "default_redis_host")]
    pub host: String,
    #[serde(default = "default_redis_port")]
    pub port: u16,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub db: u8,
    #[serde(default = "default_key_prefix")]
    pub key_prefix: String,
}

fn default_redis_host() -> String {
    "127.0.0.1".to_string()
}
fn default_redis_port() -> u16 {
    6379
}
fn default_key_prefix() -> String {
    "xiaozhi".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebsocketConfig {
    #[serde(default = "default_ws_host")]
    pub host: String,
    #[serde(default = "default_ws_port")]
    pub port: u16,
}

impl Default for WebsocketConfig {
    fn default() -> Self {
        Self {
            host: default_ws_host(),
            port: default_ws_port(),
        }
    }
}

fn default_ws_host() -> String {
    "0.0.0.0".to_string()
}
fn default_ws_port() -> u16 {
    8989
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MqttClientConfig {
    #[serde(default = "default_true")]
    pub enable: bool,
    #[serde(default = "default_mqtt_broker")]
    pub broker: String,
    #[serde(default = "default_mqtt_type")]
    pub r#type: String,
    #[serde(default = "default_mqtt_port")]
    pub port: u16,
    #[serde(default = "default_mqtt_client_id")]
    pub client_id: String,
    #[serde(default = "default_mqtt_username")]
    pub username: String,
    #[serde(default = "default_mqtt_password")]
    pub password: String,
    /// MQTT broker 上报 offline 后销毁 transport 的宽限期（秒），对齐 Go `mqtt.transport_offline_grace_period`
    #[serde(default = "default_transport_offline_grace_secs")]
    pub transport_offline_grace_period_secs: u64,
}

impl Default for MqttClientConfig {
    fn default() -> Self {
        Self {
            enable: true,
            broker: default_mqtt_broker(),
            r#type: default_mqtt_type(),
            port: default_mqtt_port(),
            client_id: default_mqtt_client_id(),
            username: default_mqtt_username(),
            password: default_mqtt_password(),
            transport_offline_grace_period_secs: default_transport_offline_grace_secs(),
        }
    }
}

fn default_transport_offline_grace_secs() -> u64 {
    120
}

fn default_mqtt_broker() -> String {
    "127.0.0.1".to_string()
}
fn default_mqtt_type() -> String {
    "tcp".to_string()
}
fn default_mqtt_port() -> u16 {
    1883
}
fn default_mqtt_client_id() -> String {
    "xiaozhi_server".to_string()
}
fn default_mqtt_username() -> String {
    "admin".to_string()
}
fn default_mqtt_password() -> String {
    "test!@#".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MqttServerConfig {
    #[serde(default = "default_true")]
    pub enable: bool,
    #[serde(default = "default_ws_host")]
    pub listen_host: String,
    #[serde(default = "default_mqtt_port")]
    pub listen_port: u16,
    #[serde(default = "default_mqtt_client_id")]
    pub client_id: String,
    #[serde(default = "default_mqtt_username")]
    pub username: String,
    #[serde(default = "default_mqtt_password")]
    pub password: String,
    #[serde(default)]
    pub signature_key: String,
    #[serde(default)]
    pub enable_auth: bool,
    #[serde(default)]
    pub tls: MqttTlsConfig,
}

impl Default for MqttServerConfig {
    fn default() -> Self {
        Self {
            enable: true,
            listen_host: default_ws_host(),
            listen_port: default_mqtt_port(),
            client_id: default_mqtt_client_id(),
            username: default_mqtt_username(),
            password: default_mqtt_password(),
            signature_key: String::new(),
            enable_auth: false,
            tls: MqttTlsConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct MqttTlsConfig {
    #[serde(default)]
    pub enable: bool,
    #[serde(default = "default_tls_port")]
    pub port: u16,
    #[serde(default)]
    pub pem: String,
    #[serde(default)]
    pub key: String,
}

fn default_tls_port() -> u16 {
    8883
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UdpConfig {
    #[serde(default = "default_udp_host")]
    pub external_host: String,
    #[serde(default = "default_udp_port")]
    pub external_port: u16,
    #[serde(default = "default_ws_host")]
    pub listen_host: String,
    #[serde(default = "default_udp_port")]
    pub listen_port: u16,
}

impl Default for UdpConfig {
    fn default() -> Self {
        Self {
            external_host: default_udp_host(),
            external_port: default_udp_port(),
            listen_host: default_ws_host(),
            listen_port: default_udp_port(),
        }
    }
}

fn default_udp_host() -> String {
    "127.0.0.1".to_string()
}
fn default_udp_port() -> u16 {
    8990
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourcePoolConfig {
    #[serde(default = "default_pool_max")]
    pub max_size: usize,
    #[serde(default = "default_pool_min")]
    pub min_size: usize,
    #[serde(default = "default_pool_idle")]
    pub max_idle: usize,
    #[serde(default = "default_acquire_timeout")]
    pub acquire_timeout: String,
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout: String,
    #[serde(default = "default_true")]
    pub validate_on_borrow: bool,
    #[serde(default = "default_true")]
    pub validate_on_return: bool,
}

impl Default for ResourcePoolConfig {
    fn default() -> Self {
        Self {
            max_size: default_pool_max(),
            min_size: default_pool_min(),
            max_idle: default_pool_idle(),
            acquire_timeout: default_acquire_timeout(),
            idle_timeout: default_idle_timeout(),
            validate_on_borrow: true,
            validate_on_return: true,
        }
    }
}

fn default_pool_max() -> usize {
    1000
}
fn default_pool_min() -> usize {
    1
}
fn default_pool_idle() -> usize {
    50
}
fn default_acquire_timeout() -> String {
    "5s".to_string()
}
fn default_idle_timeout() -> String {
    "10m".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderSection {
    #[serde(default)]
    pub provider: String,
    #[serde(flatten)]
    pub providers: HashMap<String, Value>,
}

impl ProviderSection {
    pub fn active_config(&self) -> Option<&Value> {
        if self.provider.is_empty() {
            return None;
        }
        self.providers.get(&self.provider)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VisionConfig {
    #[serde(default)]
    pub enable_auth: bool,
    #[serde(default)]
    pub vision_url: String,
    #[serde(default)]
    pub vllm: ProviderSection,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OtaConfig {
    #[serde(default)]
    pub signature_key: String,
    #[serde(default)]
    pub test: OtaEnvironment,
    #[serde(default)]
    pub external: OtaEnvironment,
}

impl OtaConfig {
    /// 优先 test 环境 URL，其次 external，最后回退为空
    pub fn preferred_websocket_url(&self) -> &str {
        let test_url = self.test.websocket.url.trim();
        if !test_url.is_empty() {
            return test_url;
        }
        self.external.websocket.url.trim()
    }

    /// 与 Go 一致：内网 IP 用 test，否则 external
    pub fn select_env_for_client_ip(&self, client_ip: &str) -> &OtaEnvironment {
        if is_private_client_ip(client_ip) {
            &self.test
        } else {
            &self.external
        }
    }
}

/// 判断是否为内网/本机客户端（与 Go `handleOta` 一致）
pub fn is_private_client_ip(client_ip: &str) -> bool {
    let ip = client_ip.trim();
    ip.starts_with("192.168.")
        || ip.starts_with("10.")
        || ip.starts_with("127.0.0.1")
        || ip == "::1"
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OtaEnvironment {
    #[serde(default)]
    pub websocket: OtaWebsocketConfig,
    #[serde(default)]
    pub mqtt: OtaMqttConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OtaWebsocketConfig {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OtaMqttConfig {
    #[serde(default)]
    pub enable: bool,
    #[serde(default)]
    pub endpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpConfig {
    #[serde(default)]
    pub global: McpGlobalConfig,
    #[serde(default)]
    pub device: McpDeviceConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpDeviceConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_mcp_ws_path")]
    pub websocket_path: String,
    #[serde(default = "default_mcp_max_conn")]
    pub max_connections_per_device: u32,
}

impl Default for McpDeviceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            websocket_path: default_mcp_ws_path(),
            max_connections_per_device: default_mcp_max_conn(),
        }
    }
}

fn default_mcp_ws_path() -> String {
    "/xiaozhi/mcp/".to_string()
}

fn default_mcp_max_conn() -> u32 {
    5
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpGlobalConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub servers: Vec<McpServerEntry>,
    #[serde(default)]
    pub reconnect_interval: u64,
    #[serde(default)]
    pub max_reconnect_attempts: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpServerEntry {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub r#type: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalMcpConfig {
    #[serde(default = "default_true")]
    pub exit_conversation: bool,
    #[serde(default = "default_true")]
    pub clear_conversation_history: bool,
    #[serde(flatten)]
    pub tools: HashMap<String, Value>,
}

impl Default for LocalMcpConfig {
    fn default() -> Self {
        Self {
            exit_conversation: true,
            clear_conversation_history: true,
            tools: HashMap::new(),
        }
    }
}

impl LocalMcpConfig {
    /// 对齐 Go `viper.IsSet("local_mcp."+name) && !viper.GetBool(...)`：仅显式设为 false 时禁用。
    pub fn is_tool_enabled(&self, name: &str) -> bool {
        if let Some(enabled) = self.tools.get(name).and_then(|v| v.as_bool()) {
            return enabled;
        }
        match name {
            "exit_conversation" => self.exit_conversation,
            "clear_conversation_history" => self.clear_conversation_history,
            _ => true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VoiceIdentifyConfig {
    #[serde(default)]
    pub enable: bool,
    #[serde(default)]
    pub base_url: String,
    #[serde(default = "default_voice_threshold")]
    pub threshold: f64,
}

fn default_voice_threshold() -> f64 {
    0.5
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KnowledgeConfig {
    #[serde(default)]
    pub providers: HashMap<String, Value>,
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod local_mcp_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_enable_core_tools() {
        let cfg = LocalMcpConfig::default();
        assert!(cfg.is_tool_enabled("exit_conversation"));
        assert!(cfg.is_tool_enabled("search_knowledge"));
    }

    #[test]
    fn explicit_false_disables_tool() {
        let cfg = LocalMcpConfig {
            exit_conversation: false,
            ..Default::default()
        };
        assert!(!cfg.is_tool_enabled("exit_conversation"));
    }

    #[test]
    fn flatten_map_overrides_play_music() {
        let mut tools = HashMap::new();
        tools.insert("play_music".to_string(), json!(false));
        let cfg = LocalMcpConfig {
            tools,
            ..Default::default()
        };
        assert!(!cfg.is_tool_enabled("play_music"));
    }
}
