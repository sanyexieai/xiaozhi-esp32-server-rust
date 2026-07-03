use std::collections::VecDeque;

use parking_lot::RwLock;
use serde_json::Value;

const MAX_RECORDS: usize = 1000;

#[derive(Default)]
struct PoolStatsInner {
    records: VecDeque<(String, Value)>,
}

pub struct PoolStatsStore {
    inner: RwLock<PoolStatsInner>,
}

impl Clone for PoolStatsStore {
    fn clone(&self) -> Self {
        Self {
            inner: RwLock::new(PoolStatsInner {
                records: self.inner.read().records.clone(),
            }),
        }
    }
}

impl Default for PoolStatsStore {
    fn default() -> Self {
        Self::new()
    }
}

impl PoolStatsStore {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(PoolStatsInner::default()),
        }
    }

    pub fn save(&self, stats: Value) {
        let ts = chrono::Utc::now().to_rfc3339();
        let mut guard = self.inner.write();
        guard.records.push_back((ts, stats));
        while guard.records.len() > MAX_RECORDS {
            guard.records.pop_front();
        }
    }

    pub fn summary(&self) -> Value {
        let guard = self.inner.read();
        if guard.records.is_empty() {
            return serde_json::json!({
                "total_records": 0,
                "storage_duration": 0,
                "oldest_timestamp": null,
                "newest_timestamp": null,
            });
        }
        let oldest = guard.records.front().map(|(t, _)| t.clone());
        let newest = guard.records.back().map(|(t, _)| t.clone());
        serde_json::json!({
            "total_records": guard.records.len(),
            "storage_duration": guard.records.len(),
            "oldest_timestamp": oldest,
            "newest_timestamp": newest,
        })
    }

    pub fn query(&self, kind: &str, start: Option<&str>, end: Option<&str>) -> Value {
        let guard = self.inner.read();
        if guard.records.is_empty() {
            return serde_json::json!({
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "stats": {},
            });
        }
        let pick: Vec<_> = match kind {
            "all" => guard.records.iter().collect(),
            "range" => guard
                .records
                .iter()
                .filter(|(ts, _)| {
                    let after_start = start.map(|s| ts.as_str() >= s).unwrap_or(true);
                    let before_end = end.map(|e| ts.as_str() <= e).unwrap_or(true);
                    after_start && before_end
                })
                .collect(),
            _ => vec![guard.records.back().unwrap()],
        };
        if pick.is_empty() {
            return serde_json::json!({
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "stats": {},
            });
        }
        let (ts, stats) = pick.last().unwrap();
        serde_json::json!({
            "timestamp": ts,
            "stats": stats,
        })
    }
}
