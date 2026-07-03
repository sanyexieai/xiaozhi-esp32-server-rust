use async_trait::async_trait;

use crate::manager::{ChatHook, HookEvent};

pub struct StatisticPlugin;

#[async_trait]
impl ChatHook for StatisticPlugin {
    fn name(&self) -> &str {
        "statistic_plugin"
    }

    fn priority(&self) -> i32 {
        100
    }

    async fn on_event(&self, event: &HookEvent) -> Result<(), String> {
        tracing::info!(
            "[statistic] {} device={}",
            event.event_type,
            event.device_id
        );
        Ok(())
    }
}
