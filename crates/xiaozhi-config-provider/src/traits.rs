use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use xiaozhi_config::user::{ActivationPayload, UConfig};
use xiaozhi_core::Result;

pub type EventHandler = Arc<dyn Fn(&str, &HashMap<String, Value>) + Send + Sync>;
pub type WsDeviceEventNotifier = Arc<dyn Fn(String, HashMap<String, Value>) + Send + Sync>;

/// 设备事件路径（对齐 Go `config/types/event.go`）
pub mod events {
    pub const DEVICE_ONLINE: &str = "/api/device/active";
    pub const DEVICE_OFFLINE: &str = "/api/device/inactive";
    pub const HANDLE_MESSAGE_INJECT: &str = "/api/device/inject_msg";
}

#[async_trait]
pub trait UserConfigProvider: Send + Sync {
    async fn is_device_activated(&self, device_id: &str, client_id: &str) -> Result<bool>;
    async fn get_activation_info(
        &self,
        device_id: &str,
        client_id: &str,
    ) -> Result<(String, String, String, i32)>;
    async fn verify_challenge(
        &self,
        device_id: &str,
        client_id: &str,
        payload: ActivationPayload,
    ) -> Result<bool>;
    async fn get_user_config(&self, device_id: &str) -> Result<UConfig>;
    async fn switch_device_role_by_name(&self, device_id: &str, role_name: &str) -> Result<String>;
    async fn restore_device_default_role(&self, device_id: &str) -> Result<()>;
    async fn get_system_config(&self) -> Result<String>;
    async fn touch_device_activity(&self, device_id: &str) -> Result<()> {
        let _ = device_id;
        Ok(())
    }
    async fn report_device_presence(&self, device_id: &str, online: bool) -> Result<()> {
        let _ = (device_id, online);
        Ok(())
    }
    fn notify_device_event(&self, event_type: &str, event_data: HashMap<String, Value>);
    fn register_message_event_handler(&self, event_type: &str, handler: EventHandler);
    fn attach_ws_notifier(&self, _notifier: WsDeviceEventNotifier) {}
    fn invoke_message_handlers(&self, _event_type: &str, _event_data: &HashMap<String, Value>) {}
    async fn invalidate_user_config_cache(&self, _device_id: &str) -> Result<()> {
        Ok(())
    }
}
