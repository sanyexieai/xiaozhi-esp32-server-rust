//! LLM 流式输出分句（对齐 Go streamtransform 的句级 TTS 触发）

const SENTENCE_BREAKS: &[char] = &['。', '！', '？', '.', '!', '?', '\n', '；', ';'];

pub struct SentenceBuffer {
    pending: String,
}

impl SentenceBuffer {
    pub fn new() -> Self {
        Self {
            pending: String::new(),
        }
    }

    pub fn push_delta(&mut self, delta: &str) -> Vec<String> {
        self.pending.push_str(delta);
        let mut out = Vec::new();
        loop {
            let Some((idx, ch)) = self
                .pending
                .char_indices()
                .find(|(_, c)| SENTENCE_BREAKS.contains(c))
            else {
                break;
            };
            let end = idx + ch.len_utf8();
            let sentence = self.pending[..end].trim().to_string();
            self.pending = self.pending[end..].to_string();
            if sentence.len() >= 2 {
                out.push(sentence);
            }
        }
        out
    }

    pub fn flush(&mut self) -> Option<String> {
        let tail = self.pending.trim().to_string();
        self.pending.clear();
        if tail.len() >= 1 {
            Some(tail)
        } else {
            None
        }
    }
}
