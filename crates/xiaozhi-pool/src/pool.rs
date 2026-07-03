use std::sync::Arc;
use std::time::Instant;

use dashmap::DashMap;
use xiaozhi_config::ResourcePoolConfig;
use xiaozhi_core::{Error, Result};

use crate::stats::PoolStats;

pub struct ResourcePool<T: Send + Sync + 'static> {
    resources: Arc<DashMap<String, Vec<PooledResource<T>>>>,
    config: ResourcePoolConfig,
    factory: Arc<dyn Fn(&str, &serde_json::Value) -> Result<Arc<T>> + Send + Sync>,
    stats: Arc<PoolStats>,
}

struct PooledResource<T> {
    resource: Arc<T>,
    last_used: Instant,
    in_use: bool,
}

impl<T: Send + Sync + 'static> ResourcePool<T> {
    pub fn new(
        config: ResourcePoolConfig,
        factory: Arc<dyn Fn(&str, &serde_json::Value) -> Result<Arc<T>> + Send + Sync>,
    ) -> Self {
        Self {
            resources: Arc::new(DashMap::new()),
            config,
            factory,
            stats: Arc::new(PoolStats::default()),
        }
    }

    pub fn acquire(&self, pool_key: &str, config: &serde_json::Value) -> Result<PooledHandle<T>> {
        self.stats.record_acquire(pool_key);

        if let Some(mut entry) = self.resources.get_mut(pool_key) {
            for item in entry.iter_mut() {
                if !item.in_use {
                    item.in_use = true;
                    item.last_used = Instant::now();
                    return Ok(PooledHandle {
                        resource: item.resource.clone(),
                        pool_key: pool_key.to_string(),
                        resources: self.resources.clone(),
                        stats: self.stats.clone(),
                    });
                }
            }
        }

        if self.count_total() >= self.config.max_size {
            return Err(Error::Pool(format!("资源池已满: {pool_key}")));
        }

        let resource = (self.factory)(pool_key, config)?;

        self.resources
            .entry(pool_key.to_string())
            .or_default()
            .push(PooledResource {
                resource: resource.clone(),
                last_used: Instant::now(),
                in_use: true,
            });

        Ok(PooledHandle {
            resource,
            pool_key: pool_key.to_string(),
            resources: self.resources.clone(),
            stats: self.stats.clone(),
        })
    }

    fn count_total(&self) -> usize {
        self.resources.iter().map(|e| e.value().len()).sum()
    }

    pub fn runtime_snapshot(&self) -> serde_json::Map<String, serde_json::Value> {
        use serde_json::json;
        let mut map = serde_json::Map::new();
        for entry in self.resources.iter() {
            let pool_key = entry.key().clone();
            let resources = entry.value();
            let total = resources.len();
            let in_use = resources.iter().filter(|r| r.in_use).count();
            map.insert(
                pool_key,
                json!({
                    "total_resources": total,
                    "available_resources": total.saturating_sub(in_use),
                    "in_use_resources": in_use,
                    "max_size": self.config.max_size,
                    "min_size": self.config.min_size,
                    "max_idle": self.config.max_idle,
                    "is_closed": false,
                }),
            );
        }
        map
    }

    pub fn stats(&self) -> Arc<PoolStats> {
        self.stats.clone()
    }
}

pub struct PooledHandle<T> {
    resource: Arc<T>,
    pool_key: String,
    resources: Arc<DashMap<String, Vec<PooledResource<T>>>>,
    stats: Arc<PoolStats>,
}

impl<T> PooledHandle<T> {
    pub fn get(&self) -> &T {
        &self.resource
    }
}

impl<T> Drop for PooledHandle<T> {
    fn drop(&mut self) {
        if let Some(mut entry) = self.resources.get_mut(&self.pool_key) {
            for item in entry.iter_mut() {
                if item.in_use {
                    item.in_use = false;
                    item.last_used = Instant::now();
                    break;
                }
            }
        }
        self.stats.record_release(&self.pool_key);
    }
}
