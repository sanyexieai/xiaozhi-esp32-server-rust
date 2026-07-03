use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

static REGISTRY: Mutex<Vec<Arc<dyn PoolStatsCollector + Send + Sync>>> = Mutex::new(Vec::new());

pub trait PoolStatsCollector: Send + Sync {
    fn collect(&self) -> HashMap<String, Value>;
}

pub fn register_collector(collector: Arc<dyn PoolStatsCollector + Send + Sync>) {
    if let Ok(mut guard) = REGISTRY.lock() {
        guard.push(collector);
    }
}

pub fn collect_all_stats() -> Value {
    let mut merged = Map::new();
    if let Ok(guard) = REGISTRY.lock() {
        for collector in guard.iter() {
            for (key, stats) in collector.collect() {
                merged.insert(key, stats);
            }
        }
    }
    Value::Object(merged)
}

use serde_json::Map;

/// 基于 acquire/release 计数的轻量快照（无活跃池时为空）
pub fn snapshot_from_counters(
    pool_key: &str,
    acquires: u64,
    releases: u64,
    max_size: usize,
    min_size: usize,
    max_idle: usize,
) -> Value {
    let in_use = acquires.saturating_sub(releases);
    json!({
        "total_resources": in_use,
        "available_resources": 0,
        "in_use_resources": in_use,
        "max_size": max_size,
        "min_size": min_size,
        "max_idle": max_idle,
        "is_closed": false,
        "acquires": acquires,
        "releases": releases,
        "pool_key": pool_key,
    })
}
