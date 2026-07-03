//! Realtime 模式下音乐播放 ASR 门控（对齐 Go `realtime_media_gate.go`）

use crate::detect::remove_punctuation;

struct ControlRule {
    action: &'static str,
    keywords: &'static [&'static str],
}

const CONTROL_RULES: &[ControlRule] = &[
    ControlRule {
        action: "play_playlist",
        keywords: &[
            "播放歌单",
            "播放歌单里的歌曲",
            "播放播放列表",
            "播放列表",
        ],
    },
    ControlRule {
        action: "enqueue_current",
        keywords: &[
            "加入歌单",
            "加入播放列表",
            "添加到歌单",
            "添加到播放列表",
        ],
    },
    ControlRule {
        action: "resume",
        keywords: &["继续播放", "恢复播放", "继续听", "接着放", "接着播"],
    },
    ControlRule {
        action: "pause",
        keywords: &["暂停", "先暂停", "先停一下"],
    },
    ControlRule {
        action: "stop",
        keywords: &["停止播放", "停止", "停播", "别播了"],
    },
    ControlRule {
        action: "next",
        keywords: &["下一首", "下首", "切到下一首", "切歌"],
    },
    ControlRule {
        action: "prev",
        keywords: &["上一首", "上首", "切到上一首"],
    },
];

const EXIT_KEYWORDS: &[&str] = &[
    "再见",
    "拜拜",
    "拜了",
    "回见",
    "退出",
    "退出对话",
    "退下吧",
];

fn normalize_gate_text(text: &str) -> String {
    remove_punctuation(text.trim()).to_lowercase()
}

pub fn detect_media_control_action(text: &str) -> Option<&'static str> {
    let normalized = normalize_gate_text(text);
    if normalized.is_empty() {
        return None;
    }
    for rule in CONTROL_RULES {
        for keyword in rule.keywords {
            let kw = normalize_gate_text(keyword);
            if !kw.is_empty() && normalized.contains(&kw) {
                return Some(rule.action);
            }
        }
    }
    None
}

pub fn is_media_exit_command(text: &str) -> bool {
    let normalized = normalize_gate_text(text);
    if normalized.is_empty() {
        return false;
    }
    EXIT_KEYWORDS.iter().any(|kw| {
        let kw = normalize_gate_text(kw);
        !kw.is_empty() && normalized.contains(&kw)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_full_control_actions() {
        let cases = [
            ("给我继续播放", Some("resume")),
            ("先暂停一下。", Some("pause")),
            ("停止播放吧", Some("stop")),
            ("下一首", Some("next")),
            ("上一首", Some("prev")),
            ("播放歌单里的歌曲", Some("play_playlist")),
            ("把当前播放加入歌单", Some("enqueue_current")),
            ("帮我讲个笑话", None),
        ];
        for (text, want) in cases {
            assert_eq!(
                detect_media_control_action(text),
                want,
                "text={text}"
            );
        }
    }

    #[test]
    fn detects_exit_commands() {
        let cases = [
            ("再见", true),
            ("那就退出对话", true),
            ("拜拜啦", true),
            ("继续播放", false),
            ("今天天气怎么样", false),
        ];
        for (text, want) in cases {
            assert_eq!(is_media_exit_command(text), want, "text={text}");
        }
    }

    #[test]
    fn detects_pause_and_next() {
        assert_eq!(detect_media_control_action("先暂停一下"), Some("pause"));
        assert_eq!(detect_media_control_action("下一首"), Some("next"));
    }

    #[test]
    fn detects_exit() {
        assert!(is_media_exit_command("拜拜"));
    }
}
