//! LLM 流式输出分片（对齐 Go `llm_common.LLMResponseStruct`）

#[derive(Debug, Clone, Default)]
pub struct LlmResponseChunk {
    pub text: String,
    pub is_start: bool,
    pub is_end: bool,
}

impl LlmResponseChunk {
    pub fn segment(text: impl Into<String>, is_start: bool) -> Self {
        Self {
            text: text.into(),
            is_start,
            is_end: false,
        }
    }

    pub fn end() -> Self {
        Self {
            text: String::new(),
            is_start: false,
            is_end: true,
        }
    }
}
