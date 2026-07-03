use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::Response,
    routing::{delete, get, patch, post, put},
    Json, Router,
};
use parking_lot::RwLock;
use serde_json::Value;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use xiaozhi_config::AppConfig;

use crate::auth::{decode_token, Claims};
use crate::captcha::CaptchaStore;
use crate::db::Database;
use crate::handlers;
use crate::pool_stats::PoolStatsStore;
use crate::voice_clone_worker;
use crate::ws::WsHub;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Database>,
    pub app_config: Arc<RwLock<AppConfig>>,
    pub config_path: PathBuf,
    pub auth_token: String,
    pub endpoint_auth_token: String,
    pub static_dir: PathBuf,
    pub data_dir: PathBuf,
    pub captcha: Arc<CaptchaStore>,
    pub pool_stats: Arc<PoolStatsStore>,
    pub ws_hub: WsHub,
}

impl AppState {
    pub fn new(
        app_config: AppConfig,
        config_path: &Path,
        db_path: &Path,
        static_dir: &Path,
    ) -> anyhow::Result<Self> {
        let db = Arc::new(Database::open(db_path)?);
        voice_clone_worker::reload_pending_voice_clone_tasks(Arc::clone(&db));
        let ws_hub = WsHub::new(db.clone());
        let data_dir = db_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("data"));
        std::fs::create_dir_all(&data_dir)?;
        let state = Self {
            db,
            auth_token: app_config.manager.auth_token.clone(),
            endpoint_auth_token: app_config.manager.endpoint_auth_token.clone(),
            app_config: Arc::new(RwLock::new(app_config)),
            config_path: config_path.to_path_buf(),
            static_dir: static_dir.to_path_buf(),
            data_dir,
            captcha: Arc::new(CaptchaStore::new()),
            pool_stats: Arc::new(PoolStatsStore::new()),
            ws_hub,
        };
        state.import_yaml_configs()?;
        if let Err(e) = crate::system_configs::sync_app_config_from_db(&state) {
            tracing::warn!("启动时合并 DB 系统配置失败: {e:#}");
        }
        Ok(state)
    }

    /// 仅在数据库为空时，从 `config.yaml` 读取默认块做**一次性种子导入**。
    /// 文件不存在或解析失败时跳过；后台增删改只写数据库，不回写 yaml。
    pub fn import_yaml_configs(&self) -> anyhow::Result<()> {
        let cfg = if self.config_path.exists() {
            match xiaozhi_config::load_config(&self.config_path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(
                        "种子配置读取失败，跳过数据库导入: {} ({:#})",
                        self.config_path.display(),
                        e
                    );
                    return Ok(());
                }
            }
        } else {
            tracing::info!(
                "种子配置文件不存在，跳过数据库导入: {}",
                self.config_path.display()
            );
            return Ok(());
        };
        for (kind, section) in [
            ("llm", &cfg.llm),
            ("asr", &cfg.asr),
            ("tts", &cfg.tts),
            ("vad", &cfg.vad),
        ] {
            if self.db.count_configs(kind)? > 0 {
                continue;
            }
            if section.provider.is_empty() {
                continue;
            }
            let mut json_value = section
                .providers
                .get(&section.provider)
                .cloned()
                .unwrap_or(Value::Object(Default::default()));
            let provider = if kind == "llm" {
                let normalized =
                    xiaozhi_llm::normalize_llm_provider(&section.provider, &section.provider, &json_value);
                if let Value::Object(ref mut map) = json_value {
                    map.insert("provider".into(), Value::String(normalized.clone()));
                }
                normalized
            } else {
                section.provider.clone()
            };
            let json_data = json_value.to_string();
            let _ = self.db.create_config(&crate::db::ConfigInput {
                r#type: kind.to_string(),
                name: format!("默认{kind}配置"),
                config_id: section.provider.clone(),
                provider,
                json_data,
                enabled: true,
                is_default: true,
            });
        }
        if self.db.count_configs("auth")? == 0 {
            let auth_json = serde_json::to_string(&cfg.auth)?;
            let _ = self.db.create_config(&crate::db::ConfigInput {
                r#type: "auth".to_string(),
                name: "默认认证配置".to_string(),
                config_id: "default".to_string(),
                provider: String::new(),
                json_data: auth_json,
                enabled: true,
                is_default: true,
            });
        }
        if self.db.count_configs("chat")? == 0 {
            let mut chat = cfg.chat.clone();
            if chat.global_system_prompt.is_empty() && !cfg.system_prompt.is_empty() {
                chat.global_system_prompt = cfg.system_prompt.clone();
            }
            let chat_json = serde_json::to_string(&chat)?;
            let _ = self.db.create_config(&crate::db::ConfigInput {
                r#type: "chat".to_string(),
                name: "默认聊天配置".to_string(),
                config_id: "default".to_string(),
                provider: String::new(),
                json_data: chat_json,
                enabled: true,
                is_default: true,
            });
        }
        if self.db.count_configs("vision_base")? == 0 {
            let vision_base_json = serde_json::json!({
                "enable_auth": cfg.vision.enable_auth,
                "vision_url": cfg.vision.vision_url,
            })
            .to_string();
            let _ = self.db.create_config(&crate::db::ConfigInput {
                r#type: "vision_base".to_string(),
                name: "默认 Vision 基础配置".to_string(),
                config_id: "default".to_string(),
                provider: String::new(),
                json_data: vision_base_json,
                enabled: true,
                is_default: true,
            });
        }
        Ok(())
    }
}

pub fn build_router(state: AppState) -> Router {
    let api = Router::new()
        .route("/setup/status", get(handlers::setup::status))
        .route("/setup/local-ip", get(handlers::setup::local_ip))
        .route("/setup/initialize", post(handlers::setup::initialize))
        .route("/captcha/status", get(handlers::captcha::captcha_status))
        .route("/captcha/challenge", get(handlers::captcha::captcha_challenge))
        .route("/login", post(handlers::auth::login))
        .route("/register", post(handlers::auth::register))
        .route("/profile", get(handlers::auth::profile))
        .route("/dashboard/stats", get(handlers::dashboard::stats))
        .route("/user/agents", get(handlers::agents::list).post(handlers::agents::create))
        .route(
            "/user/agents/{id}",
            get(handlers::agents::get)
                .put(handlers::agents::update)
                .delete(handlers::agents::delete),
        )
        .route(
            "/user/agents/{id}/devices",
            get(handlers::agent_devices::list_agent_devices)
                .post(handlers::agent_devices::add_device_to_agent),
        )
        .route(
            "/user/agents/{id}/devices/{device_id}",
            delete(handlers::agent_devices::remove_device_from_agent),
        )
        .route(
            "/user/devices/pending",
            get(handlers::devices::list_pending),
        )
        .route("/user/devices/claim", post(handlers::devices::claim))
        .route("/user/devices", get(handlers::devices::list).post(handlers::devices::create))
        .route(
            "/user/devices/live-status",
            post(handlers::devices::user_live_status),
        )
        .route(
            "/user/devices/{id}",
            put(handlers::devices::update).delete(handlers::devices::delete),
        )
        .route("/devices/{id}/apply-role", post(handlers::roles::apply_to_device))
        .route("/user/roles", get(handlers::roles::list).post(handlers::roles::create))
        .route(
            "/user/roles/{id}",
            put(handlers::roles::update).delete(handlers::roles::delete),
        )
        .route("/user/roles/{id}/toggle", patch(handlers::roles::toggle))
        .route(
            "/user/knowledge-bases",
            get(handlers::knowledge::list).post(handlers::knowledge::create),
        )
        .route("/user/llm-configs", get(handlers::config::get_llm))
        .route("/user/tts-configs", get(handlers::config::get_tts))
        .route("/user/mcp-services/options", get(handlers::mcp::service_options))
        .route(
            "/user/agents/{id}/mcp-endpoint",
            get(handlers::mcp::agent_mcp_endpoint),
        )
        .route(
            "/user/agents/{id}/openclaw-endpoint",
            get(handlers::mcp::agent_openclaw_endpoint),
        )
        .route("/user/agents/{id}/mcp-tools", get(handlers::mcp::agent_mcp_tools))
        .route("/user/agents/{id}/mcp-call", post(handlers::mcp::agent_mcp_call))
        .route(
            "/user/agents/{id}/openclaw-chat-test",
            post(handlers::mcp::openclaw_chat_test),
        )
        .route("/user/devices/{id}/mcp-tools", get(handlers::mcp::device_mcp_tools))
        .route("/user/devices/{id}/mcp-call", post(handlers::mcp::device_mcp_call))
        .route(
            "/user/devices/{id}/endpoints",
            get(handlers::devices::user_device_endpoints),
        )
        .route(
            "/user/devices/{id}/signals",
            get(handlers::devices::user_device_signals),
        )
        .route(
            "/user/devices/{id}/speak",
            post(handlers::devices::user_device_speak),
        )
        .route(
            "/user/devices/{id}/abort",
            post(handlers::devices::user_device_abort),
        )
        .route(
            "/user/devices/{id}/goodbye",
            post(handlers::devices::user_device_goodbye),
        )
        .route("/user/devices/inject-message", post(handlers::speakers::inject_message))
        .route("/user/voice-options", get(handlers::voice::voice_options))
        .route(
            "/admin/users/{id}/voice-options",
            get(handlers::voice::admin_voice_options),
        )
        .route(
            "/user/voice-clone/capabilities",
            get(handlers::voice::capabilities),
        )
        .route(
            "/user/voice-clones",
            get(handlers::voice::list).post(handlers::voice::create),
        )
        .route(
            "/user/voice-clones/{id}",
            put(handlers::voice::update).delete(handlers::voice::delete),
        )
        .route("/user/voice-clones/{id}/retry", post(handlers::voice::retry))
        .route(
            "/user/voice-clones/{id}/append-audio",
            post(handlers::voice::append_audio),
        )
        .route("/user/voice-clones/{id}/preview", get(handlers::voice::preview))
        .route("/user/voice-clones/{id}/audios", get(handlers::voice::list_audios))
        .route(
            "/user/voice-clones/audios/{audio_id}/file",
            get(handlers::voice::audio_file),
        )
        .route(
            "/admin/users/{id}/voice-clones",
            get(handlers::voice::admin_list),
        )
        .route(
            "/user/speaker-groups",
            get(handlers::speakers::list).post(handlers::speakers::create),
        )
        .route(
            "/user/speaker-groups/{id}",
            get(handlers::speakers::get)
                .put(handlers::speakers::update)
                .delete(handlers::speakers::delete),
        )
        .route(
            "/user/speaker-groups/{id}/verify",
            post(handlers::speakers::verify),
        )
        .route(
            "/user/speaker-groups/{id}/samples",
            get(handlers::speakers::list_samples).post(handlers::speakers::add_sample),
        )
        .route(
            "/user/speaker-groups/{id}/samples/{sample_id}",
            delete(handlers::speakers::delete_sample),
        )
        .route(
            "/user/speaker-groups/{id}/samples/{sample_id}/file",
            get(handlers::speakers::sample_file),
        )
        .route(
            "/user/history/sessions",
            get(handlers::history::list_sessions),
        )
        .route(
            "/user/history/sessions/{session_id}/messages",
            get(handlers::history::get_session_messages),
        )
        .route(
            "/user/history/messages",
            get(handlers::history::list_messages),
        )
        .route(
            "/user/history/export",
            get(handlers::history::export_messages),
        )
        .route(
            "/user/history/agents/{agent_id}/messages",
            get(handlers::history::list_agent_messages),
        )
        .route(
            "/user/history/messages/{id}",
            delete(handlers::history::delete_message),
        )
        .route(
            "/user/history/messages/{id}/audio",
            get(handlers::history::get_message_audio),
        )
        .route(
            "/admin/history/sessions",
            get(handlers::history::admin_list_sessions),
        )
        .route(
            "/admin/history/sessions/{session_id}/messages",
            get(handlers::history::admin_get_session_messages),
        )
        .route(
            "/admin/history/messages",
            get(handlers::history::admin_list_messages),
        )
        .route(
            "/admin/history/messages/{id}",
            delete(handlers::history::admin_delete_message),
        )
        .route("/user/role-templates", get(handlers::roles::list_global))
        .route(
            "/user/knowledge-bases/{id}",
            put(handlers::knowledge::update).delete(handlers::knowledge::delete),
        )
        .route("/user/knowledge-bases/{id}/sync", post(handlers::knowledge::sync))
        .route(
            "/user/knowledge-bases/{id}/test-search",
            post(handlers::knowledge::test_search),
        )
        .route(
            "/user/knowledge-bases/{id}/documents",
            get(handlers::knowledge::list_documents).post(handlers::knowledge::create_document),
        )
        .route(
            "/user/knowledge-bases/{id}/documents/upload",
            post(handlers::knowledge::upload_document),
        )
        .route(
            "/user/knowledge-bases/{kb_id}/documents/{doc_id}",
            put(handlers::knowledge::update_document).delete(handlers::knowledge::delete_document),
        )
        .route(
            "/user/knowledge-bases/{kb_id}/documents/{doc_id}/sync",
            post(handlers::knowledge::sync_document),
        )
        .route(
            "/user/api-tokens",
            get(handlers::api_tokens::list).post(handlers::api_tokens::create),
        )
        .route("/user/api-tokens/{id}", delete(handlers::api_tokens::delete))
        .route(
            "/admin/users",
            get(handlers::users::list).post(handlers::users::create),
        )
        .route(
            "/admin/users/{id}",
            put(handlers::users::update).delete(handlers::users::delete),
        )
        .route(
            "/admin/users/{id}/reset-password",
            post(handlers::users::reset_password),
        )
        .route(
            "/admin/users/{id}/voice-clone-quotas",
            get(handlers::users::voice_clone_quotas).put(handlers::users::update_voice_clone_quotas),
        )
        .route(
            "/admin/users/{id}/knowledge-bases",
            get(handlers::knowledge::admin_list_for_user),
        )
        .route("/admin/devices", get(handlers::devices::admin_list).post(handlers::devices::admin_create))
        .route(
            "/admin/devices/{id}",
            put(handlers::devices::admin_update).delete(handlers::devices::admin_delete),
        )
        .route(
            "/admin/devices/{id}/endpoints",
            get(handlers::devices::admin_device_endpoints),
        )
        .route(
            "/admin/devices/{id}/signals",
            get(handlers::devices::admin_device_signals),
        )
        .route(
            "/admin/devices/{id}/speak",
            post(handlers::devices::admin_device_speak),
        )
        .route(
            "/admin/devices/{id}/abort",
            post(handlers::devices::admin_device_abort),
        )
        .route(
            "/admin/devices/{id}/goodbye",
            post(handlers::devices::admin_device_goodbye),
        )
        .route(
            "/admin/devices/live-status",
            post(handlers::devices::admin_live_status),
        )
        .route(
            "/admin/devices/inject-message",
            post(handlers::speakers::admin_inject_message),
        )
        .route("/admin/agents", get(handlers::agent_devices::admin_list).post(handlers::agents::admin_create))
        .route(
            "/admin/agents/{id}",
            put(handlers::agents::admin_update).delete(handlers::agents::admin_delete),
        )
        .route(
            "/admin/roles/global",
            get(handlers::roles::list_global).post(handlers::roles::create_global),
        )
        .route(
            "/admin/roles/global/{id}",
            put(handlers::roles::update_global).delete(handlers::roles::delete_global),
        )
        .route(
            "/admin/roles/global/{id}/toggle",
            patch(handlers::roles::toggle_global),
        )
        .route(
            "/admin/roles/global/{id}/default",
            patch(handlers::roles::set_default_global),
        )
        .route("/admin/llm-configs", get(handlers::config::get_llm).post(handlers::config::create_llm))
        .route(
            "/admin/llm-configs/{id}",
            put(handlers::config::update_llm).delete(handlers::config::delete_llm),
        )
        .route("/admin/asr-configs", get(handlers::config::get_asr).post(handlers::config::create_asr))
        .route(
            "/admin/asr-configs/{id}",
            put(handlers::config::update_asr).delete(handlers::config::delete_asr),
        )
        .route("/admin/tts-configs", get(handlers::config::get_tts).post(handlers::config::create_tts))
        .route(
            "/admin/tts-configs/{id}",
            put(handlers::config::update_tts).delete(handlers::config::delete_tts),
        )
        .route("/admin/vad-configs", get(handlers::config::get_vad).post(handlers::config::create_vad))
        .route(
            "/admin/vad-configs/{id}",
            put(handlers::config::update_vad).delete(handlers::config::delete_vad),
        )
        .route("/admin/ota-configs", get(handlers::config::get_ota).post(handlers::config::create_ota))
        .route(
            "/admin/ota-configs/{id}",
            put(handlers::config::update_ota).delete(handlers::config::delete_ota),
        )
        .route("/admin/mqtt-configs", get(handlers::config::get_mqtt).post(handlers::config::create_mqtt))
        .route(
            "/admin/mqtt-configs/{id}",
            put(handlers::config::update_mqtt).delete(handlers::config::delete_mqtt),
        )
        .route(
            "/admin/mqtt-server-configs",
            get(handlers::config::get_mqtt_server).post(handlers::config::create_mqtt_server),
        )
        .route(
            "/admin/mqtt-server-configs/{id}",
            put(handlers::config::update_mqtt_server).delete(handlers::config::delete_mqtt_server),
        )
        .route("/admin/udp-configs", get(handlers::config::get_udp).post(handlers::config::create_udp))
        .route(
            "/admin/udp-configs/{id}",
            put(handlers::config::update_udp).delete(handlers::config::delete_udp),
        )
        .route("/admin/chat-settings", get(handlers::config::get_chat_settings).put(handlers::config::save_chat_settings))
        .route("/admin/configs/{id}/toggle", post(handlers::config::toggle_config))
        .route("/admin/configs/test", post(handlers::config::test_config))
        .route("/admin/configs/export", get(handlers::config::export_config))
        .route("/admin/configs/import", post(handlers::config::import_config))
        .route("/admin/pool/stats/summary", get(handlers::pool::summary))
        .route("/admin/pool/stats", get(handlers::pool::query))
        .route(
            "/admin/vision-configs",
            get(handlers::config::get_vision).post(handlers::config::create_vision),
        )
        .route(
            "/admin/vision-configs/{id}",
            put(handlers::config::update_vision).delete(handlers::config::delete_vision),
        )
        .route(
            "/admin/vision-base-config",
            get(handlers::config::get_vision_base).put(handlers::config::save_vision_base),
        )
        .route(
            "/admin/memory-configs",
            get(handlers::config::get_memory).post(handlers::config::create_memory),
        )
        .route(
            "/admin/memory-configs/{id}",
            put(handlers::config::update_memory).delete(handlers::config::delete_memory),
        )
        .route(
            "/admin/memory-configs/{id}/set-default",
            post(handlers::config::set_memory_default),
        )
        .route(
            "/admin/speaker-configs",
            get(handlers::config::get_speaker).post(handlers::config::create_speaker),
        )
        .route(
            "/admin/speaker-configs/{id}",
            put(handlers::config::update_speaker).delete(handlers::config::delete_speaker),
        )
        .route(
            "/admin/knowledge-search-configs",
            get(handlers::config::get_knowledge_search).post(handlers::config::create_knowledge_search),
        )
        .route(
            "/admin/knowledge-search-configs/{id}",
            put(handlers::config::update_knowledge_search).delete(handlers::config::delete_knowledge_search),
        )
        .route(
            "/admin/knowledge-search-configs/weknora/models",
            post(handlers::config::weknora_models),
        )
        .route(
            "/admin/mcp-configs",
            get(handlers::config::get_mcp).post(handlers::config::create_mcp),
        )
        .route(
            "/admin/mcp-configs/{id}",
            put(handlers::config::update_mcp).delete(handlers::config::delete_mcp),
        )
        .route(
            "/admin/mcp-configs/discover-tools",
            post(handlers::mcp::discover_tools),
        )
        .route("/admin/mcp-market/providers", get(handlers::mcp::market_providers))
        .route(
            "/admin/mcp-markets",
            get(handlers::mcp::market_list).post(handlers::mcp::create_mcp_market),
        )
        .route(
            "/admin/mcp-markets/{id}",
            put(handlers::mcp::update_mcp_market).delete(handlers::config::delete_mcp_market),
        )
        .route(
            "/admin/mcp-markets/{id}/test",
            post(handlers::mcp::test_market),
        )
        .route(
            "/admin/mcp-market/services/{market_id}/{*service_id}",
            get(handlers::mcp::market_service_detail),
        )
        .route(
            "/admin/mcp-market/imported-services/{id}/tools",
            get(handlers::mcp::imported_service_tools),
        )
        .route("/admin/mcp-market/services", get(handlers::mcp::market_services))
        .route("/admin/mcp-market/import", post(handlers::mcp::import_service))
        .route(
            "/admin/mcp-market/imported-services",
            get(handlers::mcp::imported_services).post(handlers::mcp::create_imported_service),
        )
        .route(
            "/admin/mcp-market/imported-services/{id}",
            put(handlers::mcp::update_imported_service)
                .delete(handlers::mcp::delete_imported_service),
        )
        .route(
            "/admin/agents/{id}/mcp-endpoint",
            get(handlers::mcp::agent_mcp_endpoint),
        )
        .route(
            "/admin/agents/{id}/openclaw-chat-test",
            post(handlers::mcp::openclaw_chat_test),
        )
        .route(
            "/admin/agents/{id}/openclaw-endpoint",
            get(handlers::mcp::agent_openclaw_endpoint),
        )
        .route("/admin/agents/{id}/mcp-tools", get(handlers::mcp::agent_mcp_tools))
        .route("/admin/agents/{id}/mcp-call", post(handlers::mcp::agent_mcp_call))
        .route(
            "/admin/devices/{id}/mcp-tools",
            get(handlers::mcp::device_mcp_tools),
        )
        .route("/admin/devices/{id}/mcp-call", post(handlers::mcp::device_mcp_call))
        .route(
            "/user/device-chat/config",
            get(handlers::device_simulator::get_user_config),
        )
        .route(
            "/user/device-chat/ws",
            get(handlers::device_simulator::user_ws_handler),
        )
        .route(
            "/admin/device-simulator/config",
            get(handlers::device_simulator::get_config),
        )
        .route(
            "/admin/device-simulator/ws",
            get(handlers::device_simulator::ws_handler),
        )
        .route("/internal/device/activated", get(handlers::internal::device_activated))
        .route("/internal/device/activation", get(handlers::internal::device_activation))
        .route("/internal/device/activate", post(handlers::internal::device_activate))
        .route(
            "/internal/device/{device_id}/role",
            post(handlers::internal::switch_device_role),
        )
        .route(
            "/internal/device/{device_id}/role/default",
            post(handlers::internal::restore_device_default_role),
        )
        .route("/internal/configs/{device_id}", get(handlers::internal::device_config))
        .route("/internal/system/configs", get(handlers::internal::system_config))
        .route("/internal/chat/history", post(handlers::history::save_chat_legacy))
        .route("/internal/history/messages", post(handlers::history::save_message_internal))
        .route(
            "/internal/history/sessions/{session_id}/dialogue",
            get(handlers::history::internal_session_dialogue),
        )
        .route(
            "/internal/history/messages/{message_id}/audio",
            put(handlers::history::update_message_audio_internal),
        )
        .route("/internal/pool/stats", post(handlers::internal::pool_stats))
        .route("/internal/device/presence", post(handlers::internal::device_presence))
        .route("/internal/device/touch", post(handlers::internal::device_touch))
        .route(
            "/internal/knowledge/search",
            post(handlers::internal::knowledge_search),
        )
        // OpenAPI v1（JWT 或 API Token 认证）
        .route("/open/v1/profile", get(handlers::auth::profile))
        .route(
            "/open/v1/devices",
            get(handlers::devices::list).post(handlers::devices::create),
        )
        .route(
            "/open/v1/agents",
            get(handlers::agents::list).post(handlers::agents::create),
        )
        .route(
            "/open/v1/agents/{id}",
            get(handlers::agents::get)
                .put(handlers::agents::update)
                .delete(handlers::agents::delete),
        )
        .route(
            "/open/v1/history/messages",
            get(handlers::history::list_messages),
        )
        .route(
            "/open/v1/history/export",
            get(handlers::history::export_messages),
        )
        .route(
            "/open/v1/devices/inject-message",
            post(handlers::speakers::inject_message),
        )
        .route(
            "/open/v1/agents/{id}/mcp-tools",
            get(handlers::mcp::agent_mcp_tools),
        )
        .route(
            "/open/v1/agents/{id}/mcp-call",
            post(handlers::mcp::agent_mcp_call),
        )
        .route(
            "/open/v1/devices/{id}/mcp-tools",
            get(handlers::mcp::device_mcp_tools),
        )
        .route(
            "/open/v1/devices/{id}/mcp-call",
            post(handlers::mcp::device_mcp_call),
        )
        .layer(middleware::from_fn_with_state(state.clone(), jwt_middleware))
        .with_state(state.clone());

    let mut router = Router::new()
        .nest("/api", api)
        .route("/ws", get(handlers::ws::ws_handler))
        .with_state(state.clone())
        .layer(CorsLayer::permissive());

    if state.static_dir.join("index.html").exists() {
        let index = state.static_dir.join("index.html");
        let serve_dir =
            ServeDir::new(&state.static_dir).fallback(ServeFile::new(index));
        router = router.fallback_service(serve_dir);
    }

    router
}

async fn jwt_middleware(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let path = req.uri().path();

    if is_public(path) || is_internal(path, req.headers().get(header::AUTHORIZATION)) {
        if is_internal(path, req.headers().get(header::AUTHORIZATION)) {
            let auth = req
                .headers()
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            let token = auth.strip_prefix("Bearer ").unwrap_or("");
            if token != state.auth_token {
                return Err(StatusCode::UNAUTHORIZED);
            }
        }
        return Ok(next.run(req).await);
    }

    let auth = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let mut token = auth.strip_prefix("Bearer ").unwrap_or("").to_string();
    if token.is_empty() {
        if let Some(api_token) = req.headers().get("X-API-Token").and_then(|v| v.to_str().ok()) {
            token = api_token.to_string();
        }
    }
    if token.is_empty()
        && (path.ends_with("/admin/device-simulator/ws") || path.ends_with("/user/device-chat/ws"))
    {
        if let Some(query_token) = extract_query_param(req.uri().query(), "token") {
            token = query_token;
        }
    }

    if let Ok(claims) = decode_token(&token) {
        req.extensions_mut().insert(claims);
        return Ok(next.run(req).await);
    }

    if let Ok(Some((user_id, username, role))) = state.db.verify_api_token(&token) {
        req.extensions_mut().insert(Claims {
            sub: user_id,
            username,
            role,
            exp: 0,
        });
        return Ok(next.run(req).await);
    }

    Err(StatusCode::UNAUTHORIZED)
}

fn is_public(path: &str) -> bool {
    matches!(
        path,
        "/api/setup/status"
            | "/api/setup/local-ip"
            | "/api/setup/initialize"
            | "/api/login"
            | "/api/register"
            | "/api/captcha/status"
            | "/api/captcha/challenge"
            | "/setup/status"
            | "/setup/local-ip"
            | "/setup/initialize"
            | "/login"
            | "/register"
            | "/captcha/status"
            | "/captcha/challenge"
    )
}

fn is_internal(path: &str, auth: Option<&axum::http::HeaderValue>) -> bool {
    (path.starts_with("/api/internal/") || path.starts_with("/internal/")) && auth.is_some()
}

fn extract_query_param(query: Option<&str>, key: &str) -> Option<String> {
    let query = query?;
    query.split('&').find_map(|pair| {
        let mut parts = pair.splitn(2, '=');
        let k = parts.next()?;
        if k != key {
            return None;
        }
        let v = parts.next().unwrap_or("");
        urlencoding::decode(v).ok().map(|s| s.into_owned())
    })
}

pub fn json_success<T: serde::Serialize>(value: T) -> Json<Value> {
    Json(serde_json::json!({ "success": true, "data": value }))
}

pub fn json_data<T: serde::Serialize>(value: T) -> Json<Value> {
    Json(serde_json::json!({ "data": value }))
}

pub fn json_ok<T: serde::Serialize>(value: T) -> Json<Value> {
    Json(serde_json::json!(value))
}

pub fn json_error(status: StatusCode, msg: &str) -> (StatusCode, Json<Value>) {
    (status, Json(serde_json::json!({ "error": msg })))
}
