//! 内部事件总线

use std::sync::Arc;

use dashmap::DashMap;
use serde_json::Value;
use tokio::sync::broadcast;

pub type EventHandler = Arc<dyn Fn(&str, Value) + Send + Sync>;

pub struct EventBus {
    handlers: DashMap<String, Vec<EventHandler>>,
    broadcast_tx: broadcast::Sender<(String, Value)>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (broadcast_tx, _) = broadcast::channel(capacity);
        Self {
            handlers: DashMap::new(),
            broadcast_tx,
        }
    }

    pub fn subscribe(&self, event_type: &str, handler: EventHandler) {
        self.handlers
            .entry(event_type.to_string())
            .or_default()
            .push(handler);
    }

    pub fn publish(&self, event_type: &str, data: Value) {
        if let Some(handlers) = self.handlers.get(event_type) {
            for handler in handlers.iter() {
                handler(event_type, data.clone());
            }
        }
        let _ = self.broadcast_tx.send((event_type.to_string(), data));
    }

    pub fn receiver(&self) -> broadcast::Receiver<(String, Value)> {
        self.broadcast_tx.subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(1024)
    }
}
