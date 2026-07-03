//! 资源池管理 (VAD/ASR/LLM/TTS)

pub mod fingerprint;
pub mod pool;
pub mod registry;
pub mod stats;

pub use fingerprint::generate_config_key;
pub use pool::*;
pub use registry::*;
pub use stats::*;
