use std::time::Instant;

pub const SENTENCE_MIN_LEN: usize = 1;

#[derive(Debug, Clone)]
pub struct ResponseStreamState {
    pub device_id: String,
    pub buffer: String,
    pub emitted_text: String,
    pub pending_text: String,
    pub has_delta: bool,
    pub is_first: bool,
    pub last_seq: i64,
    pub created_at: Instant,
}

impl ResponseStreamState {
    pub fn new() -> Self {
        Self {
            device_id: String::new(),
            buffer: String::new(),
            emitted_text: String::new(),
            pending_text: String::new(),
            has_delta: false,
            is_first: true,
            last_seq: 0,
            created_at: Instant::now(),
        }
    }

    pub fn accounted_text(&self) -> String {
        normalize_openclaw_speech_text(format!(
            "{}{}",
            self.emitted_text.trim(),
            self.buffer.trim()
        ))
    }

    pub fn mark_emitted(&mut self, text: &str) {
        let normalized = normalize_openclaw_speech_text(text);
        if normalized.is_empty() {
            return;
        }
        self.emitted_text = normalize_openclaw_speech_text(format!(
            "{}{}",
            self.emitted_text, normalized
        ));
    }

    pub fn apply_snapshot_content(&mut self, content: &str) -> String {
        let normalized = normalize_openclaw_speech_text(content);
        if normalized.is_empty() {
            return String::new();
        }
        self.pending_text.clear();
        let mut snapshot_buffer = normalized.clone();
        let emitted = normalize_openclaw_speech_text(&self.emitted_text);
        if !emitted.is_empty() {
            if let Some(suffix) = trim_openclaw_canonical_prefix(&normalized, &emitted) {
                snapshot_buffer = suffix;
            }
        }
        self.buffer = normalize_openclaw_speech_text(&snapshot_buffer);
        self.buffer.clone()
    }

    pub fn to_incremental_content(&mut self, content: &str, stream_done: bool) -> String {
        let normalized = normalize_openclaw_speech_text(content);
        if normalized.is_empty() {
            if stream_done && !self.has_delta && !self.pending_text.is_empty() {
                let snapshot = self.pending_text.clone();
                self.pending_text.clear();
                return snapshot;
            }
            return String::new();
        }

        if self.has_delta {
            let accounted = self.accounted_text();
            if !accounted.is_empty() {
                if let Some(delta) = trim_openclaw_canonical_prefix(&normalized, &accounted) {
                    return delta;
                }
            }
            return normalized;
        }

        let accounted = self.accounted_text();
        if !accounted.is_empty() {
            self.has_delta = true;
            if let Some(delta) = trim_openclaw_canonical_prefix(&normalized, &accounted) {
                return delta;
            }
            return normalized;
        }

        if self.pending_text.is_empty() {
            self.pending_text = normalized;
            if stream_done {
                let snapshot = self.pending_text.clone();
                self.pending_text.clear();
                return snapshot;
            }
            return String::new();
        }

        if is_openclaw_canonical_growth(&self.pending_text, &normalized) {
            if open_claw_canonical_key(&normalized).len()
                >= open_claw_canonical_key(&self.pending_text).len()
            {
                self.pending_text = normalized;
            }
            if stream_done {
                let snapshot = self.pending_text.clone();
                self.pending_text.clear();
                return snapshot;
            }
            return String::new();
        }

        if is_openclaw_punctuation_only(&normalized) {
            self.pending_text =
                normalize_openclaw_speech_text(format!("{}{}", self.pending_text, normalized));
            if stream_done {
                let snapshot = self.pending_text.clone();
                self.pending_text.clear();
                return snapshot;
            }
            return String::new();
        }

        self.has_delta = true;
        let combined = normalize_openclaw_speech_text(format!(
            "{}{}",
            self.pending_text, normalized
        ));
        self.pending_text.clear();
        combined
    }
}

pub fn is_openclaw_snapshot_frame(phase: &str, content_type: &str) -> bool {
    let phase = phase.trim().to_lowercase();
    let content_type = content_type.trim().to_lowercase();
    phase == "snapshot" || content_type == "snapshot"
}

pub fn extract_openclaw_sentences(text: &str, min_len: usize, is_first: bool) -> (Vec<String>, String) {
    let trimmed = normalize_openclaw_speech_text(text);
    if trimmed.is_empty() {
        return (Vec::new(), String::new());
    }
    let runes: Vec<char> = trimmed.chars().collect();
    let mut start = 0usize;
    let mut sentences = Vec::new();

    for i in 0..runes.len() {
        if !is_openclaw_sentence_separator(runes[i], is_first) {
            continue;
        }
        let segment = trim_openclaw_segment(&runes[start..=i].iter().collect::<String>());
        if segment.is_empty() {
            start = skip_openclaw_delimiters(&runes, i + 1);
            continue;
        }
        if segment.chars().count() < min_len {
            continue;
        }
        sentences.push(segment);
        start = skip_openclaw_delimiters(&runes, i + 1);
    }

    let remaining = trim_openclaw_segment(&runes[start..].iter().collect::<String>());
    (sentences, remaining)
}

pub fn normalize_openclaw_speech_text(text: impl AsRef<str>) -> String {
    let trimmed = text.as_ref().trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let mut text = trimmed
        .replace('\r', "")
        .replace('\t', " ")
        .replace("```", "")
        .replace('`', "")
        .replace("**", "")
        .replace("__", "")
        .replace("###", "")
        .replace("##", "")
        .replace('#', "")
        .replace("\n- ", "，")
        .replace("\n* ", "，")
        .replace("\n• ", "，")
        .replace('\n', "，")
        .replace('|', "，");

    let mut out: Vec<char> = Vec::new();
    for ch in text.chars() {
        match ch {
            c if c.is_whitespace() => {
                if out.is_empty() || *out.last().unwrap() == ' ' || is_open_claw_pause_rune(*out.last().unwrap()) {
                    continue;
                }
                out.push(' ');
            }
            '*' | '_' | '`' | '#' => {}
            c if is_open_claw_soft_separator(c) => {
                trim_openclaw_trailing_space(&mut out);
                if out.is_empty() || is_open_claw_pause_rune(*out.last().unwrap()) {
                    continue;
                }
                out.push('，');
            }
            c if is_openclaw_sentence_separator(c, false) => {
                trim_openclaw_trailing_space(&mut out);
                if out.is_empty() {
                    continue;
                }
                out.push(c);
            }
            '：' | ':' => {
                trim_openclaw_trailing_space(&mut out);
                if out.is_empty() {
                    continue;
                }
                out.push('：');
            }
            '-' | '•' => {
                if out.is_empty() || is_open_claw_pause_rune(*out.last().unwrap()) {
                    continue;
                }
                out.push(ch);
            }
            c => out.push(c),
        }
    }

    trim_openclaw_segment(&out.iter().collect::<String>())
}

fn is_openclaw_sentence_separator(ch: char, _is_first: bool) -> bool {
    matches!(ch, '。' | '？' | '！' | ';' | '；' | '.' | '?' | '!')
}

fn is_open_claw_soft_separator(ch: char) -> bool {
    matches!(ch, '，' | ',' | '、')
}

fn is_open_claw_pause_rune(ch: char) -> bool {
    matches!(
        ch,
        ' ' | '，' | ',' | '、' | '。' | '！' | '？' | '!' | '?' | '；' | ';' | '：' | ':'
    )
}

fn skip_openclaw_delimiters(runes: &[char], mut start: usize) -> usize {
    while start < runes.len() {
        let r = runes[start];
        if r.is_whitespace() || is_open_claw_soft_separator(r) {
            start += 1;
            continue;
        }
        break;
    }
    start
}

fn trim_openclaw_segment(text: &str) -> String {
    let mut text = text.trim().to_string();
    text = text.trim_start_matches(['-', '•', '*', '，', ',', '、', ';', '；', ':', '：', ' '])
        .to_string();
    text = text
        .replace(" ，", "，")
        .replace(" 。", "。")
        .replace(" ！", "！")
        .replace(" ？", "？")
        .replace(" ；", "；")
        .replace(" ：", "：")
        .replace("( ", "(")
        .replace("（ ", "（")
        .replace(" )", ")")
        .replace(" ）", "）");
    text.trim().to_string()
}

fn trim_openclaw_trailing_space(out: &mut Vec<char>) {
    while out.last() == Some(&' ') {
        out.pop();
    }
}

fn open_claw_canonical_key(text: &str) -> String {
    normalize_openclaw_speech_text(text)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

fn open_claw_comparable_key(text: &str) -> String {
    normalize_openclaw_speech_text(text)
        .chars()
        .filter(|c| !c.is_whitespace() && !is_open_claw_pause_rune(*c))
        .collect()
}

fn is_openclaw_canonical_growth(base: &str, candidate: &str) -> bool {
    let base_key = open_claw_comparable_key(base);
    let candidate_key = open_claw_comparable_key(candidate);
    if base_key.is_empty() || candidate_key.is_empty() {
        return false;
    }
    candidate_key.starts_with(&base_key) || base_key.starts_with(&candidate_key)
}

fn is_openclaw_punctuation_only(text: &str) -> bool {
    let normalized = normalize_openclaw_speech_text(text);
    if normalized.is_empty() {
        return false;
    }
    normalized
        .chars()
        .all(|c| c.is_whitespace() || is_open_claw_pause_rune(c))
}

fn trim_openclaw_canonical_prefix(text: &str, prefix: &str) -> Option<String> {
    let normalized_text = normalize_openclaw_speech_text(text);
    let normalized_prefix = normalize_openclaw_speech_text(prefix);
    if normalized_prefix.is_empty() {
        return Some(normalized_text.trim().to_string());
    }

    let text_key = open_claw_comparable_key(&normalized_text);
    let prefix_key = open_claw_comparable_key(&normalized_prefix);
    if prefix_key.is_empty() {
        return Some(normalized_text.trim().to_string());
    }
    if !text_key.starts_with(&prefix_key) {
        return None;
    }
    if text_key == prefix_key {
        return Some(String::new());
    }

    let text_runes: Vec<char> = normalized_text.chars().collect();
    let prefix_runes: Vec<char> = normalized_prefix.chars().collect();
    let mut matched = 0usize;
    let mut advance_prefix = |matched: &mut usize| {
        while *matched < prefix_runes.len() && is_open_claw_comparable_ignorable(prefix_runes[*matched])
        {
            *matched += 1;
        }
    };
    advance_prefix(&mut matched);
    for (idx, r) in text_runes.iter().enumerate() {
        if is_open_claw_comparable_ignorable(*r) {
            continue;
        }
        if matched >= prefix_runes.len() || *r != prefix_runes[matched] {
            return None;
        }
        matched += 1;
        advance_prefix(&mut matched);
        if matched == prefix_runes.len() {
            let mut suffix_start = idx + 1;
            while suffix_start < text_runes.len()
                && is_open_claw_comparable_ignorable(text_runes[suffix_start])
            {
                suffix_start += 1;
            }
            return Some(text_runes[suffix_start..].iter().collect());
        }
    }
    None
}

fn is_open_claw_comparable_ignorable(ch: char) -> bool {
    ch.is_whitespace() || is_open_claw_pause_rune(ch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_sentences_on_punctuation() {
        let (sentences, remaining) =
            extract_openclaw_sentences("你好。世界", SENTENCE_MIN_LEN, true);
        assert_eq!(sentences, vec!["你好。"]);
        assert_eq!(remaining, "世界");
    }

    #[test]
    fn normalizes_markdown_noise() {
        let got = normalize_openclaw_speech_text("**你好**\n- 世界");
        assert!(got.contains("你好"));
        assert!(!got.contains('*'));
    }
}
