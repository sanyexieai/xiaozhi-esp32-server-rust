//! 小智统一日志体系
//!
//! 基于 `tracing`，支持：
//! - 控制台彩色输出
//! - 滚动文件日志
//! - SQLite 数据库持久化

mod config;
pub mod db;

pub use config::{DatabaseLogOptions, FileLogOptions, LoggingOptions};
pub use db::ensure_schema;

pub use tracing::{debug, error, info, instrument, trace, warn, Level};

use std::io::{self, IsTerminal};

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::fmt::time::ChronoLocal;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, Registry};

use crate::db::DbLogWorker;

/// 保持进程生命周期内的日志后台任务（文件 non-blocking writer、DB 写入线程）
pub struct LoggingGuard {
    _file_guard: Option<WorkerGuard>,
    _db_worker: Option<DbLogWorker>,
}

/// 初始化全局 tracing 订阅者
pub fn init(opts: LoggingOptions) -> anyhow::Result<LoggingGuard> {
    let filter = build_env_filter(&opts);
    let mut file_guard = None;
    let mut db_worker = None;

    let console_layer = opts.stdout.then(|| {
        tracing_subscriber::fmt::layer()
            .with_timer(ChronoLocal::new("%Y-%m-%d %H:%M:%S%.3f".into()))
            .with_target(true)
            .with_thread_ids(false)
            .with_thread_names(false)
            .with_ansi(use_ansi())
            .with_level(true)
            .with_file(false)
            .with_line_number(false)
            .boxed()
    });

    let file_layer = opts.file.as_ref().map(|file_opts| {
        if let Some(parent) = file_opts.dir.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::create_dir_all(&file_opts.dir);

        let rotation = pick_rotation(file_opts.rotation_hours);
        let appender = RollingFileAppender::new(
            rotation,
            &file_opts.dir,
            sanitize_filename(&file_opts.filename),
        );
        let (writer, guard) = tracing_appender::non_blocking(appender);
        file_guard = Some(guard);

        tracing_subscriber::fmt::layer()
            .with_timer(ChronoLocal::new("%Y-%m-%d %H:%M:%S%.3f".into()))
            .with_target(true)
            .with_ansi(false)
            .with_level(true)
            .with_writer(writer)
            .boxed()
    });

    let db_layer = opts.database.as_ref().map(|db_opts| {
        let (worker, layer) = DbLogWorker::spawn(&db_opts.path, &opts.service)
            .expect("数据库日志层初始化失败");
        db_worker = Some(worker);
        layer.boxed()
    });

    Registry::default()
        .with(filter)
        .with(console_layer)
        .with(file_layer)
        .with(db_layer)
        .try_init()
        .map_err(|e| anyhow::anyhow!("日志初始化失败: {e}"))?;

    tracing::info!(
        service = %opts.service,
        stdout = opts.stdout,
        file = ?opts.file.as_ref().map(|f| f.dir.join(&f.filename)),
        database = ?opts.database.as_ref().map(|d| d.path.clone()),
        "统一日志体系已初始化"
    );

    Ok(LoggingGuard {
        _file_guard: file_guard,
        _db_worker: db_worker,
    })
}

fn build_env_filter(opts: &LoggingOptions) -> EnvFilter {
    if let Ok(filter) = EnvFilter::try_from_default_env() {
        return filter;
    }

    let module = opts.service.replace('-', "_");
    let spec = format!("{},{}={}", opts.level, module, opts.level);
    EnvFilter::try_new(&spec).unwrap_or_else(|_| EnvFilter::new("info"))
}

fn use_ansi() -> bool {
    io::stdout().is_terminal()
}

fn pick_rotation(rotation_hours: u32) -> Rotation {
    if rotation_hours <= 1 {
        Rotation::HOURLY
    } else {
        Rotation::DAILY
    }
}

fn sanitize_filename(name: &str) -> String {
    std::path::Path::new(name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("app.log")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiaozhi_config::LogConfig;

    #[test]
    fn builds_env_filter_from_service() {
        let opts = LoggingOptions {
            level: "debug".into(),
            ..LoggingOptions::new("xiaozhi-server")
        };
        let filter = build_env_filter(&opts);
        assert!(filter.to_string().contains("xiaozhi_server"));
    }

    #[test]
    fn maps_log_config_to_options() {
        let log = LogConfig {
            path: "logs".into(),
            file: "server.log".into(),
            level: "info".into(),
            database: true,
            ..Default::default()
        };
        let opts = LoggingOptions::from_log_config("xiaozhi-server", &log);
        assert!(opts.file.is_some());
        assert!(opts.database.is_some());
    }
}
