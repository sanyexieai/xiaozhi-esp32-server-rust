//! 小智统一日志初始化与配置

use std::path::{Path, PathBuf};

use xiaozhi_config::LogConfig;

/// 日志初始化选项
#[derive(Debug, Clone)]
pub struct LoggingOptions {
    /// 服务名，如 `xiaozhi-server` / `xiaozhi-manager`
    pub service: String,
    /// 日志级别（未设置 `RUST_LOG` 时生效）
    pub level: String,
    /// 是否输出到控制台（彩色）
    pub stdout: bool,
    /// 文件日志
    pub file: Option<FileLogOptions>,
    /// 数据库日志
    pub database: Option<DatabaseLogOptions>,
    /// 配置文件所在目录，用于解析相对 `log.path`
    pub config_base_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct FileLogOptions {
    pub dir: PathBuf,
    pub filename: String,
    pub max_age: u32,
    pub rotation_hours: u32,
}

#[derive(Debug, Clone)]
pub struct DatabaseLogOptions {
    pub path: PathBuf,
}

impl LoggingOptions {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            level: "info".into(),
            stdout: true,
            file: None,
            database: None,
            config_base_dir: PathBuf::from("."),
        }
    }

    pub fn from_log_config(service: impl Into<String>, log: &LogConfig) -> Self {
        Self::from_log_config_with_base(service, log, PathBuf::from("."))
    }

    pub fn from_log_config_with_base(
        service: impl Into<String>,
        log: &LogConfig,
        config_base_dir: PathBuf,
    ) -> Self {
        let service = service.into();
        let mut opts = Self {
            service: service.clone(),
            level: log.level.clone(),
            stdout: log.stdout,
            config_base_dir: config_base_dir.clone(),
            file: None,
            database: None,
        };

        if !log.path.is_empty() || !log.file.is_empty() {
            let dir = resolve_log_dir(log, &config_base_dir);
            opts.file = Some(FileLogOptions {
                dir,
                filename: log.file.clone(),
                max_age: log.max_age,
                rotation_hours: log.rotation_time,
            });
        }

        if log.database {
            let db_path = if log.database_path.is_empty() {
                default_database_path(&service, &config_base_dir)
            } else {
                resolve_path(&log.database_path, &config_base_dir)
            };
            opts.database = Some(DatabaseLogOptions { path: db_path });
        }

        opts
    }

    pub fn with_database_path(mut self, path: PathBuf) -> Self {
        self.database = Some(DatabaseLogOptions { path });
        self
    }
}

fn resolve_log_dir(log: &LogConfig, base: &Path) -> PathBuf {
    if log.path.is_empty() {
        return base.join("logs");
    }
    resolve_path(&log.path, base)
}

fn resolve_path(raw: &str, base: &Path) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

fn default_database_path(service: &str, base: &Path) -> PathBuf {
    base.join("data").join("logs").join(format!("{service}.db"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_log_dir_from_config_base() {
        let log = LogConfig {
            path: "../logs/".into(),
            file: "server.log".into(),
            ..Default::default()
        };
        let opts = LoggingOptions::from_log_config_with_base(
            "xiaozhi-server",
            &log,
            PathBuf::from("config"),
        );
        let file = opts.file.unwrap();
        assert_eq!(file.dir, PathBuf::from("config/../logs/"));
    }
}
