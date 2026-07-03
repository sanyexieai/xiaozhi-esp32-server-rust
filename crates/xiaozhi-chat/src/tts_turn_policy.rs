//! TTS 轮次结束策略（对齐 Go `ttsTurnEndPolicy` / `injectedSpeechTTSTurnEndPolicy`）

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TtsTurnEndPolicy {
    #[default]
    None,
    /// 播报结束后下发 goodbye 并关闭会话（MQTT 一次性主动播报）
    GoodbyeAndIdle,
}

pub fn injected_speech_tts_turn_end_policy(auto_listen: bool) -> TtsTurnEndPolicy {
    if auto_listen {
        TtsTurnEndPolicy::None
    } else {
        TtsTurnEndPolicy::GoodbyeAndIdle
    }
}
