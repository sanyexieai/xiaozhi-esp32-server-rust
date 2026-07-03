//! 文本转语音 (TTS) 模块

pub mod http_client;
pub mod aliyun_qwen;
pub mod audio_decoder;
pub mod cosyvoice;
pub mod doubao;
pub mod doubao_http;
pub mod doubao_model;
pub mod doubao_v3_ws;
pub mod doubao_ws;
pub mod edge;
pub mod edge_offline;
pub mod factory;
pub mod minimax;
pub mod openai;
pub mod providers;
pub mod traits;
pub mod volcengine_protocol;
pub mod xiaozhi;
pub mod xunfei;
pub mod zhipu;

pub use factory::*;
pub use traits::*;
pub use audio_decoder::{wrap_tts_audio_stream, wrap_tts_audio_stream_with_source};
