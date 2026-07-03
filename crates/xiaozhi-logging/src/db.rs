//! SQLite 日志持久化 Layer

use std::fmt;
use std::path::Path;
use std::sync::mpsc::{self, SyncSender};
use std::thread;

use chrono::Utc;
use rusqlite::{params, Connection};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS system_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at TEXT NOT NULL,
    service TEXT NOT NULL,
    level TEXT NOT NULL,
    target TEXT NOT NULL,
    message TEXT NOT NULL,
    fields TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS idx_system_logs_created_at ON system_logs(created_at);
CREATE INDEX IF NOT EXISTS idx_system_logs_service ON system_logs(service);
CREATE INDEX IF NOT EXISTS idx_system_logs_level ON system_logs(level);
"#;

const MAX_ROWS_PER_SERVICE: i64 = 10_000;

#[derive(Debug)]
struct LogEntry {
    created_at: String,
    service: String,
    level: String,
    target: String,
    message: String,
    fields: String,
}

struct EventVisitor {
    message: String,
    fields: serde_json::Map<String, serde_json::Value>,
}

impl EventVisitor {
    fn new() -> Self {
        Self {
            message: String::new(),
            fields: serde_json::Map::new(),
        }
    }
}

impl Visit for EventVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}").trim_matches('"').to_string();
        } else {
            self.fields.insert(
                field.name().to_string(),
                serde_json::Value::String(format!("{value:?}")),
            );
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.fields.insert(
                field.name().to_string(),
                serde_json::Value::String(value.to_string()),
            );
        }
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields.insert(
            field.name().to_string(),
            serde_json::Value::Number(value.into()),
        );
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields
            .insert(field.name().to_string(), serde_json::json!(value));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields
            .insert(field.name().to_string(), serde_json::Value::Bool(value));
    }
}

pub struct DbLogLayer {
    tx: SyncSender<LogEntry>,
    service: String,
}

pub struct DbLogWorker;

impl DbLogWorker {
    pub fn spawn(db_path: &Path, service: &str) -> anyhow::Result<(Self, DbLogLayer)> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(db_path)?;
        conn.execute_batch(SCHEMA)?;

        let (tx, rx) = mpsc::sync_channel(4096);
        let service_owned = service.to_string();
        let db_path = db_path.to_path_buf();

        let handle = thread::Builder::new()
            .name("xiaozhi-db-log".into())
            .spawn(move || db_writer_loop(rx, db_path, service_owned))?;
        let _ = handle;

        let layer = DbLogLayer {
            tx: tx.clone(),
            service: service.to_string(),
        };

        Ok((Self, layer))
    }
}

fn db_writer_loop(rx: mpsc::Receiver<LogEntry>, db_path: std::path::PathBuf, service: String) {
    let Ok(conn) = Connection::open(&db_path) else {
        eprintln!("xiaozhi-logging: 无法打开日志数据库 {}", db_path.display());
        return;
    };
    let _ = conn.execute_batch(SCHEMA);

    let mut insert_count = 0u64;
    while let Ok(entry) = rx.recv() {
        if conn
            .execute(
                "INSERT INTO system_logs (created_at, service, level, target, message, fields)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    entry.created_at,
                    entry.service,
                    entry.level,
                    entry.target,
                    entry.message,
                    entry.fields,
                ],
            )
            .is_ok()
        {
            insert_count += 1;
            if insert_count.is_multiple_of(500) {
                trim_old_logs(&conn, &service);
            }
        }
    }

    trim_old_logs(&conn, &service);
}

fn trim_old_logs(conn: &Connection, service: &str) {
    let _ = conn.execute(
        "DELETE FROM system_logs WHERE service = ?1 AND id NOT IN (
            SELECT id FROM system_logs WHERE service = ?1 ORDER BY id DESC LIMIT ?2
        )",
        params![service, MAX_ROWS_PER_SERVICE],
    );
}

impl DbLogLayer {
    fn send(&self, entry: LogEntry) {
        let _ = self.tx.try_send(entry);
    }
}

impl<S> Layer<S> for DbLogLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = EventVisitor::new();
        event.record(&mut visitor);

        let message = if visitor.message.is_empty() {
            event.metadata().name().to_string()
        } else {
            visitor.message
        };

        let fields = serde_json::Value::Object(visitor.fields).to_string();
        let meta = event.metadata();

        self.send(LogEntry {
            created_at: Utc::now().to_rfc3339(),
            service: self.service.clone(),
            level: meta.level().to_string(),
            target: meta.target().to_string(),
            message,
            fields,
        });
    }
}

/// 在已有 SQLite 连接上确保 system_logs 表存在（供 manager 主库使用）
pub fn ensure_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA)
}
