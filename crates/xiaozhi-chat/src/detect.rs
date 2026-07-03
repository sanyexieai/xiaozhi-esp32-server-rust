//! 唤醒词 detect 逻辑（对齐 Go `session.go` resolveDetectAction / util.go）

use xiaozhi_config::AppConfig;

use crate::state::{ClientState, ListenPhase};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectAction {
    Silent,
    Welcome,
    Llm,
}

/// abort 信令来源：设备 MQTT 自发 vs 管理端/调试界面显式打断
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbortOrigin {
    Device,
    Explicit,
}

pub fn remove_punctuation(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_ascii_punctuation() && !c.is_whitespace())
        .collect()
}

pub fn is_wakeup_word(text: &str, wakeup_words: &[String]) -> bool {
    wakeup_words.iter().any(|w| w == text)
}

pub fn is_auto_listen_active(state: &ClientState) -> bool {
    state.listen_mode == "auto"
        && matches!(
            state.listen_phase,
            ListenPhase::Listening | ListenPhase::Processing
        )
}

/// 欢迎语 TTS 正在下发/播放（对齐 Go `IsWelcomePlaying`）
pub fn is_welcome_playing(welcome_playing: bool) -> bool {
    welcome_playing
}

pub fn should_ignore_listen_start_during_welcome(mode: &str, welcome_playing: bool) -> bool {
    mode != "realtime" && welcome_playing
}

/// 主动注入播报（auto_listen=false）期间设备常会自发 listen start，不应打断 TTS
pub fn should_ignore_listen_start_during_injected_speech(
    mode: &str,
    injected_speech_guard: bool,
) -> bool {
    mode != "realtime" && injected_speech_guard
}

/// LLM 播报期间设备常会补发无音频的 listen start（与 welcome 期间行为一致），不应抢占 TTS。
/// 若携带 prelisten 音频则视为用户插话（barge-in），仍允许打断。
pub fn should_ignore_listen_start_during_speak(
    mode: &str,
    tts_active: bool,
    speaking: bool,
    has_prelisten_audio: bool,
) -> bool {
    mode != "realtime" && (tts_active || speaking) && !has_prelisten_audio
}

/// 主动注入播报期间设备可能上报唤醒 detect，不应抢占当前 TTS
pub fn should_ignore_detect_during_injected_speech(injected_speech_guard: bool) -> bool {
    injected_speech_guard
}

pub fn should_interrupt_output_on_listen_start(mode: &str, welcome_playing: bool) -> bool {
    !(mode == "realtime" && welcome_playing)
}

pub fn resolve_detect_action(
    text: &str,
    app_config: &AppConfig,
    welcome_already_spoken: bool,
    auto_listen_active: bool,
    welcome_playing: bool,
) -> DetectAction {
    if text.is_empty() {
        return DetectAction::Silent;
    }
    if welcome_playing {
        return DetectAction::Silent;
    }
    if app_config.enable_greeting && is_wakeup_word(text, &app_config.wakeup_words) {
        if !welcome_already_spoken {
            return DetectAction::Welcome;
        }
        if auto_listen_active {
            return DetectAction::Silent;
        }
        return DetectAction::Llm;
    }
    DetectAction::Llm
}

pub fn random_greeting(app_config: &AppConfig) -> String {
    if app_config.greeting_list.is_empty() {
        "你好，有啥好玩的.".to_string()
    } else {
        use std::time::{SystemTime, UNIX_EPOCH};
        let idx = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as usize)
            .unwrap_or(0)
            % app_config.greeting_list.len();
        app_config.greeting_list[idx].clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiaozhi_config::AppConfig;

    #[test]
    fn ignore_auto_listen_start_while_welcome_playing() {
        assert!(should_ignore_listen_start_during_welcome("auto", true));
    }

    #[test]
    fn ignore_auto_listen_start_during_injected_speech() {
        assert!(should_ignore_listen_start_during_injected_speech("auto", true));
        assert!(!should_ignore_listen_start_during_injected_speech("realtime", true));
        assert!(!should_ignore_listen_start_during_injected_speech("auto", false));
    }

    #[test]
    fn ignore_empty_listen_start_during_llm_speak() {
        assert!(should_ignore_listen_start_during_speak("auto", true, true, false));
        assert!(!should_ignore_listen_start_during_speak("auto", true, true, true));
        assert!(!should_ignore_listen_start_during_speak("realtime", true, true, false));
        assert!(!should_ignore_listen_start_during_speak("auto", false, false, false));
    }

    #[test]
    fn ignore_detect_during_injected_speech() {
        assert!(should_ignore_detect_during_injected_speech(true));
        assert!(!should_ignore_detect_during_injected_speech(false));
    }

    #[test]
    fn detect_silent_during_welcome_playing() {
        let cfg = AppConfig {
            enable_greeting: true,
            wakeup_words: vec!["小智".into()],
            ..Default::default()
        };
        assert_eq!(
            resolve_detect_action("小智", &cfg, true, false, true),
            DetectAction::Silent
        );
    }
}
