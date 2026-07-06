mod app;
mod auth;
mod captcha;
mod db;
mod network;
mod ota_test;
mod system_configs;
mod extractors;
mod handlers;
mod knowledge_global;
mod knowledge_search;
mod knowledge_search_test;
mod knowledge_sync;
mod knowledge_upload;
mod mcp_config_test;
mod mcp_imported_merge;
mod mcp_market;
mod openclaw_sse;
mod pool_stats;
mod speaker_client;
mod uconfig_builder;
mod voice_clone_api;
mod voice_clone_doubao;
mod voice_clone_preview;
mod voice_clone_validate;
mod voice_clone_worker;
mod voice_constants;
mod voice_options;
mod weknora_models;
mod ws;

use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "xiaozhi-manager", about = "小智 ESP32 管理控制台")]
struct Args {
    #[arg(short, long, default_value = "config/config.yaml")]
    config: PathBuf,

    #[arg(long)]
    port: Option<u16>,

    #[arg(long, default_value = "data/manager.db")]
    db: PathBuf,

    #[arg(long, default_value = "frontend/dist")]
    static_dir: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let app_config = if args.config.exists() {
        match xiaozhi_config::load_config(&args.config) {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!(
                    "配置文件加载失败，使用内置默认值: {} ({e:#})",
                    args.config.display()
                );
                xiaozhi_config::AppConfig::default()
            }
        }
    } else {
        eprintln!(
            "配置文件不存在，使用内置默认值: {}",
            args.config.display()
        );
        xiaozhi_config::AppConfig::default()
    };

    let config_base = args
        .config
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    let mut log_opts = xiaozhi_logging::LoggingOptions::from_log_config_with_base(
        "xiaozhi-manager",
        &app_config.log,
        config_base,
    );
    if app_config.log.database {
        log_opts = log_opts.with_database_path(args.db.clone());
    }

    let _logging = xiaozhi_logging::init(log_opts)?;

    let listen_port = args.port.unwrap_or_else(|| {
        xiaozhi_config::backend_url_port(&app_config.manager.backend_url)
            .unwrap_or(xiaozhi_config::DEFAULT_MANAGER_PORT)
    });
    let force_port = args.port.is_some();

    let (listener, addr) =
        network::bind_tcp_listener("0.0.0.0", listen_port, force_port).await?;
    let browse_url = if addr.ip().is_unspecified() {
        format!("http://127.0.0.1:{}", addr.port())
    } else {
        format!("http://{addr}")
    };
    tracing::info!("Manager 控制台: {browse_url} （监听 {addr}）");

    let endpoint_path = args
        .db
        .parent()
        .map(|p| p.join("manager.endpoint"))
        .unwrap_or_else(|| PathBuf::from("data/manager.endpoint"));
    let backend_url = format!("http://127.0.0.1:{}", addr.port());
    match xiaozhi_config::write_manager_endpoint(&endpoint_path, &backend_url) {
        Ok(()) => tracing::info!(
            "Manager 端点已写入 {} ({backend_url})，xiaozhi-server 将自动发现",
            endpoint_path.display()
        ),
        Err(e) => tracing::warn!("写入 Manager 端点文件失败: {e:#}"),
    }

    if let Some(parent) = args.db.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut static_dir = args.static_dir.clone();
    if static_dir.is_relative() && !static_dir.exists() {
        let mut manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        manifest.pop();
        manifest.pop();
        let candidate = manifest.join("frontend").join("dist");
        if candidate.join("index.html").exists() {
            static_dir = candidate;
        }
    }

    let state = app::AppState::new(app_config, &args.config, &args.db, &static_dir)?;
    let router = app::build_router(state);
    if static_dir.join("index.html").exists() {
        tracing::info!("静态资源: {}", static_dir.display());
    } else {
        tracing::warn!(
            "前端未构建，请运行: cd frontend && npm install && npm run build"
        );
    }

    axum::serve(listener, router).await?;
    Ok(())
}