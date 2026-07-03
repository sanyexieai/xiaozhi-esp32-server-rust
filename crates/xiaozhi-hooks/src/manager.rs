use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::mpsc;
use xiaozhi_config::ChatHooksConfig;

pub struct HookEvent {
    pub event_type: String,
    pub device_id: String,
    pub data: Value,
}

#[async_trait]
pub trait ChatHook: Send + Sync {
    fn name(&self) -> &str;
    fn priority(&self) -> i32;
    async fn on_event(&self, event: &HookEvent) -> Result<(), String>;
}

pub struct HookManager {
    #[allow(dead_code)]
    hooks: Vec<Box<dyn ChatHook>>,
    tx: mpsc::Sender<HookEvent>,
}

impl HookManager {
    pub fn new(config: &ChatHooksConfig) -> Self {
        let (tx, mut rx) = mpsc::channel::<HookEvent>(config.async_config.queue_size);
        let mut hooks: Vec<Box<dyn ChatHook>> = Vec::new();

        if config
            .plugins
            .get("statistic_plugin")
            .map(|p| p.enabled)
            .unwrap_or(false)
        {
            hooks.push(Box::new(crate::statistic::StatisticPlugin));
        }

        hooks.sort_by_key(|h| std::cmp::Reverse(h.priority()));

        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                tracing::debug!(
                    "Hook event: {} device={}",
                    event.event_type,
                    event.device_id
                );
            }
        });

        Self { hooks, tx }
    }

    pub async fn emit(&self, event: HookEvent) {
        let _ = self.tx.send(event).await;
    }
}
