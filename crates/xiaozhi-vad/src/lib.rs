//! 语音活动检测 (VAD) 模块

mod ffi;
pub mod factory;
pub mod silero_vad;
pub mod ten_vad;
pub mod traits;
pub mod webrtc;

pub use factory::*;
pub use traits::*;
