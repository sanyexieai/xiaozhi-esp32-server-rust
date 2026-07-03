mod app;
mod bridge;
mod config_test;
mod device_handler;
mod manager_client;
mod mcp_api;
mod mcp_ws;
mod mqtt_runtime;
mod mqtt_service;
mod openclaw_chat;
mod openclaw_ws;
mod shared_config;
mod vision;
mod websocket;

use std::path::PathBuf;

use clap::Parser;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser, Debug)]
#[command(name = "xiaozhi-server", about = "小智 ESP32 AI 语音服务端 (Rust)")]
struct Args {
    #[arg(short, long, default_value = "config/config.yaml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,xiaozhi_server=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = if args.config.exists() {
        match xiaozhi_config::load_config(&args.config) {
            Ok(cfg) => cfg,
            Err(e) => {
                tracing::warn!(
                    "配置文件加载失败，使用内置默认值: {} ({:#})",
                    args.config.display(),
                    e
                );
                xiaozhi_config::AppConfig::default()
            }
        }
    } else {
        tracing::warn!(
            "配置文件不存在，使用内置默认值: {}",
            args.config.display()
        );
        xiaozhi_config::AppConfig::default()
    };

    tracing::info!("小智 ESP32 Server (Rust) 启动中...");
    tracing::info!(
        "WebSocket: {}:{}",
        config.websocket.host,
        config.websocket.port
    );

    let app = app::App::new(config).await?;
    app.run().await
}
