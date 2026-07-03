//! 对话管理与会话状态机

pub mod asr_manager;
pub mod audio_idle;
pub mod chat_queue;
pub mod detect;
pub mod device_mcp;
pub mod endpoint_hub;
pub mod knowledge;
pub mod llm_manager;
pub mod llm_types;
pub mod manager;
pub mod mcp_tool_media;
pub mod opus_codec;
pub mod media_coordinator;
pub mod media_player;
pub mod openclaw_warmup;
pub mod play_music;
pub mod realtime_media_gate;
pub mod outbound;
pub mod pipeline;
pub mod resource_pools;
pub mod sentence;
pub mod session;
pub mod session_media;
pub mod signal_log;
pub mod speak_path;
pub mod state;
pub mod tts_manager;
pub mod tts_turn_policy;
pub mod voice_status;

pub use device_mcp::{
    call_device_tool, has_mcp_feature, refresh_device_tools_json, run_device_mcp_init,
    run_device_mcp_init_json, DeviceMcpRuntime, McpInboundAction,
};
pub use endpoint_hub::{
    parse_tts_audio_route, EndpointHub, EndpointInfo, EndpointKind, EndpointRegistration,
    TtsAudioRoute,
};
pub use manager::{ChatManager, ChatManagerRegistry, SessionCloseReason};
pub use outbound::{OutboundFrame, SpeakDelivery};
pub use resource_pools::{SessionPoolHandles, SharedResourcePools};
pub use signal_log::{SignalEntry, SignalLog};
pub use session::*;
