use std::sync::Arc;

use serde_json::{json, Value};

use crate::registry::collect_all_stats;

pub struct StatsReporter;

impl StatsReporter {
    pub fn start_monitor(stats: Arc<PoolStats>, interval_secs: u64) {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            loop {
                ticker.tick().await;
                let snap = stats.snapshot();
                tracing::info!("资源池统计: {:?}", snap);
            }
        });
    }

    /// 定期上报到 manager
    pub fn start_pool_reporter<F, Fut>(interval_secs: u64, reporter: F)
    where
        F: Fn(Value) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            loop {
                ticker.tick().await;
                let stats = collect_all_stats();
                if stats.as_object().is_some_and(|m| !m.is_empty()) {
                    reporter(json!({ "stats": stats })).await;
                }
            }
        });
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct PoolStatsSnapshot {
    pub acquires: u64,
    pub releases: u64,
    pub per_pool: std::collections::HashMap<String, PoolEntryStats>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct PoolEntryStats {
    pub acquires: u64,
    pub releases: u64,
}

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
pub struct PoolStats {
    acquires: AtomicU64,
    releases: AtomicU64,
    per_pool: Mutex<HashMap<String, PoolEntryStats>>,
}

impl PoolStats {
    pub fn record_acquire(&self, pool_key: &str) {
        self.acquires.fetch_add(1, Ordering::Relaxed);
        self.per_pool
            .lock()
            .entry(pool_key.to_string())
            .or_default()
            .acquires += 1;
    }

    pub fn record_release(&self, pool_key: &str) {
        self.releases.fetch_add(1, Ordering::Relaxed);
        self.per_pool
            .lock()
            .entry(pool_key.to_string())
            .or_default()
            .releases += 1;
    }

    pub fn snapshot(&self) -> PoolStatsSnapshot {
        PoolStatsSnapshot {
            acquires: self.acquires.load(Ordering::Relaxed),
            releases: self.releases.load(Ordering::Relaxed),
            per_pool: self.per_pool.lock().clone(),
        }
    }
}
