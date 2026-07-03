//! OpenClaw 暖场（对齐 Go `openclaw_warmup.go`）

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex as SyncMutex;
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use xiaozhi_llm::{ChatMessage, LlmProvider};

use crate::detect::remove_punctuation;
use crate::llm_types::LlmResponseChunk;
use crate::manager::ChatManager;
use crate::tts_manager::TtsManager;

const WARMUP_PLAN_TIMEOUT: Duration = Duration::from_secs(8);
const WARMUP_PLAN_SIZE: usize = 11;

const WARMUP_SCHEDULE_MS: [u64; 11] = [
    1_000, 10_000, 20_000, 30_000, 40_000, 50_000, 60_000, 70_000, 80_000, 90_000, 100_000,
];

const WARMUP_SYSTEM_PROMPT: &str = r#"你是实时语音对话里的暖场助手，不是主回答者。

你的任务是：在主回复返回前，生成 11 条很短的中文接话，让等待过程听起来一直有人在回应。

硬性要求：
1. 只负责暖场，不能直接回答问题，不能给出事实、结论、建议、步骤、分析、解释或推测。
2. 语气要像真人在通话里轻声接话：简短、自然、口语化、有耐心。
3. 不要像客服，不要像系统提示，不要像通知播报，不要像写文案。
4. 禁止复述用户原话，尤其不要把“帮我查一下”“帮我看看”“帮我查询一下”“告诉我”这类用户指令原样拼进回复。
5. 如果需要提到主题，只能提炼成助手视角的名词短语，例如“北京后天的天气”“这个安排”；不要用命令句。
6. 前 1 到 2 条尽量更轻，不一定带主题词，例如“我看一下”“等我一下”；不要一上来就说很重的安慰话。
7. 后面的句子再逐步表达“我还在看”“我还在确认”，但要自然，不要机械重复。
8. 避免使用“正在为您处理”“请稍候”“持续跟进”“调取数据”“连接服务中”这类生硬说法。
9. 每条都必须是单句短中文，适合语音播报，长度控制在 4 到 16 个汉字。
10. 你会拿到实际播报时间点。11 条话术必须严格按这些时间点依次设计：
   - 第 1 秒：像刚接到问题，轻轻接一句。
   - 第 10 秒：自然补一句，语气仍然轻。
   - 第 20、30 秒：开始表达“我还在看”，但不要机械。
   - 第 40、50、60 秒：继续安抚，允许更明确地说“还在确认”。
   - 第 70、80、90、100 秒：承认时间有点久，但仍然自然、平静，不抱怨。
11. 只输出严格 JSON 数组，长度必须为 11。
12. JSON 每项格式必须为：{"text":"暖场语"}。
13. 禁止输出编号、Markdown、解释、代码块或 JSON 之外的任何内容。"#;

use tokio::sync::{Mutex, Notify};

struct WarmupTaskState {
    correlation_id: String,
    cancel: CancellationToken,
    lines: Mutex<Vec<String>>,
    plan_ready: Notify,
    plan_ready_at: SyncMutex<Option<Instant>>,
    plan_signaled: AtomicBool,
    speech_started: AtomicBool,
    speech_ended: AtomicBool,
    next_segment_is_start: AtomicBool,
    spoke_any: AtomicBool,
}

impl WarmupTaskState {
    fn new(correlation_id: String, cancel: CancellationToken) -> Self {
        Self {
            correlation_id,
            cancel,
            lines: Mutex::new(vec![String::new(); WARMUP_PLAN_SIZE]),
            plan_ready: Notify::new(),
            plan_ready_at: SyncMutex::new(None),
            plan_signaled: AtomicBool::new(false),
            speech_started: AtomicBool::new(false),
            speech_ended: AtomicBool::new(false),
            next_segment_is_start: AtomicBool::new(true),
            spoke_any: AtomicBool::new(false),
        }
    }

    fn mark_plan_ready(&self, ready_at: Option<Instant>) {
        if self.plan_signaled.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Some(at) = ready_at {
            *self.plan_ready_at.lock() = Some(at);
        }
        self.plan_ready.notify_waiters();
    }

    async fn wait_plan_ready(&self) -> Option<Instant> {
        if !self.plan_signaled.load(Ordering::SeqCst) {
            tokio::select! {
                _ = self.cancel.cancelled() => return None,
                _ = self.plan_ready.notified() => {}
            }
        }
        self.plan_ready_at.lock().clone()
    }

    async fn set_lines(&self, lines: Vec<String>) {
        let mut slot = self.lines.lock().await;
        for (idx, line) in lines.into_iter().enumerate().take(WARMUP_PLAN_SIZE) {
            if let Some(text) = sanitize_warmup_text(&line) {
                slot[idx] = text;
            }
        }
    }

    async fn line_at(&self, index: usize) -> String {
        self.lines.lock().await.get(index).cloned().unwrap_or_default()
    }

    fn try_mark_speech_started(&self) -> bool {
        if self.speech_started.load(Ordering::SeqCst) || self.speech_ended.load(Ordering::SeqCst) {
            return false;
        }
        self.speech_started.store(true, Ordering::SeqCst);
        true
    }

    fn mark_speech_ended(&self) -> bool {
        if !self.speech_started.load(Ordering::SeqCst) || self.speech_ended.load(Ordering::SeqCst) {
            return false;
        }
        self.speech_ended.store(true, Ordering::SeqCst);
        true
    }

    fn take_segment_start(&self) -> bool {
        self.next_segment_is_start
            .swap(false, Ordering::SeqCst)
    }

    fn has_spoken_any(&self) -> bool {
        self.spoke_any.load(Ordering::SeqCst)
    }
}

pub struct OpenClawWarmupController {
    active: Mutex<Option<Arc<WarmupTaskState>>>,
    runner: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Default for OpenClawWarmupController {
    fn default() -> Self {
        Self {
            active: Mutex::new(None),
            runner: Mutex::new(None),
        }
    }
}

impl OpenClawWarmupController {
    pub async fn start(
        &self,
        manager: Arc<ChatManager>,
        correlation_id: String,
        user_text: String,
        session_id: String,
        llm: Arc<dyn LlmProvider>,
        tts: Arc<TtsManager>,
    ) {
        let correlation_id = correlation_id.trim().to_string();
        if correlation_id.is_empty() {
            return;
        }

        self.finish("", true, &manager, &tts).await;

        let cancel = CancellationToken::new();
        let task = Arc::new(WarmupTaskState::new(correlation_id.clone(), cancel.child_token()));
        {
            let mut guard = self.active.lock().await;
            *guard = Some(Arc::clone(&task));
        }

        let parent_cancel = cancel.clone();
        let runner_task = Arc::clone(&task);
        let device_id = manager.device_id.clone();
        let device_id_for_log = device_id.clone();
        let manager_for_task = Arc::clone(&manager);
        let handle = tokio::spawn(async move {
            run_warmup_task(
                manager_for_task,
                runner_task,
                parent_cancel,
                user_text,
                session_id,
                llm,
                tts,
                device_id,
            )
            .await;
        });
        *self.runner.lock().await = Some(handle);

        info!(
            device_id = %device_id_for_log,
            correlation_id = %correlation_id,
            "OpenClaw warmup started"
        );
    }

    pub async fn has_task(&self, correlation_id: &str) -> bool {
        let correlation_id = correlation_id.trim();
        let guard = self.active.lock().await;
        guard.as_ref().is_some_and(|t| {
            correlation_id.is_empty() || t.correlation_id == correlation_id
        })
    }

    pub async fn cancel(&self, correlation_id: &str, interrupt: bool, manager: &ChatManager, tts: &TtsManager) {
        let Some(task) = self.get_task(correlation_id).await else {
            return;
        };
        if task.cancel.is_cancelled() {
            return;
        }
        task.cancel.cancel();
        if interrupt && task.has_spoken_any() {
            tts.interrupt_and_stop_sync(true, &format!(
                "OpenClaw warmup canceled correlation_id={}",
                task.correlation_id
            ))
            .await;
        }
        info!(
            device_id = %manager.device_id,
            correlation_id = %task.correlation_id,
            interrupt,
            spoke_any = task.has_spoken_any(),
            "OpenClaw warmup canceled"
        );
    }

    pub async fn finish(
        &self,
        correlation_id: &str,
        interrupt: bool,
        manager: &ChatManager,
        tts: &TtsManager,
    ) {
        let Some(task) = self.take_task(correlation_id).await else {
            return;
        };
        task.cancel.cancel();
        if let Some(handle) = self.runner.lock().await.take() {
            handle.abort();
        }
        if interrupt && task.has_spoken_any() {
            tts.interrupt_and_stop_sync(true, &format!(
                "OpenClaw warmup finished correlation_id={} interrupt",
                task.correlation_id
            ))
            .await;
        }
        let _ = task.mark_speech_ended();
        info!(
            device_id = %manager.device_id,
            correlation_id = %task.correlation_id,
            interrupt,
            spoke_any = task.has_spoken_any(),
            "OpenClaw warmup finished"
        );
    }

    pub async fn begin_openclaw_speech(&self, correlation_id: &str, tts: &TtsManager) {
        let Some(task) = self.get_task(correlation_id).await else {
            return;
        };
        if task.try_mark_speech_started() {
            tts.enqueue_tts_start(&format!(
                "OpenClaw warmup start correlation_id={}",
                task.correlation_id
            ))
            .await;
        }
    }

    pub async fn openclaw_speech_started(&self, correlation_id: &str) -> bool {
        self.get_task(correlation_id)
            .await
            .is_some_and(|t| t.speech_started.load(Ordering::SeqCst))
    }

    async fn get_task(&self, correlation_id: &str) -> Option<Arc<WarmupTaskState>> {
        let correlation_id = correlation_id.trim();
        let guard = self.active.lock().await;
        guard.as_ref().and_then(|task| {
            if !correlation_id.is_empty() && task.correlation_id != correlation_id {
                None
            } else {
                Some(Arc::clone(task))
            }
        })
    }

    async fn take_task(&self, correlation_id: &str) -> Option<Arc<WarmupTaskState>> {
        let correlation_id = correlation_id.trim();
        let mut guard = self.active.lock().await;
        let task = guard.as_ref()?;
        if !correlation_id.is_empty() && task.correlation_id != correlation_id {
            return None;
        }
        guard.take()
    }
}

async fn run_warmup_task(
    _manager: Arc<ChatManager>,
    task: Arc<WarmupTaskState>,
    cancel: CancellationToken,
    user_text: String,
    session_id: String,
    llm: Arc<dyn LlmProvider>,
    tts: Arc<TtsManager>,
    device_id: String,
) {
    let plan_task = Arc::clone(&task);
    let llm_for_plan = Arc::clone(&llm);
    let user_for_plan = user_text.clone();
    let session_for_plan = build_warmup_session_id(&session_id, &task.correlation_id);
    let plan_cancel = cancel.child_token();
    tokio::spawn(async move {
        let result = tokio::time::timeout(
            WARMUP_PLAN_TIMEOUT,
            generate_warmup_plan(&llm_for_plan, &session_for_plan, &user_for_plan),
        )
        .await;
        match result {
            Ok(Ok(lines)) => {
                plan_task.set_lines(lines).await;
                plan_task.mark_plan_ready(Some(Instant::now()));
                info!(
                    device_id = %device_id,
                    correlation_id = %plan_task.correlation_id,
                    "OpenClaw warmup plan ready"
                );
            }
            Ok(Err(e)) if plan_cancel.is_cancelled() => {
                plan_task.mark_plan_ready(None);
            }
            Ok(Err(e)) => {
                warn!(
                    device_id = %device_id,
                    correlation_id = %plan_task.correlation_id,
                    "OpenClaw warmup plan generation failed: {e}"
                );
                plan_task.mark_plan_ready(None);
            }
            Err(_) => {
                plan_task.mark_plan_ready(None);
            }
        }
    });

    let base_at = match task.wait_plan_ready().await {
        Some(at) => at,
        None => return,
    };

    for (idx, delay_ms) in WARMUP_SCHEDULE_MS.iter().enumerate() {
        if !wait_until(&cancel, base_at + Duration::from_millis(*delay_ms)).await {
            return;
        }
        let text = task.line_at(idx).await;
        if text.is_empty() {
            continue;
        }
        if speak_warmup_line(&task, &tts, &text).await.is_err() {
            return;
        }
        task.spoke_any.store(true, Ordering::SeqCst);
    }
}

async fn wait_until(cancel: &CancellationToken, deadline: Instant) -> bool {
    let wait = deadline.saturating_duration_since(Instant::now());
    if wait.is_zero() {
        return !cancel.is_cancelled();
    }
    tokio::select! {
        _ = cancel.cancelled() => false,
        _ = tokio::time::sleep(wait) => !cancel.is_cancelled(),
    }
}

async fn speak_warmup_line(
    task: &WarmupTaskState,
    tts: &TtsManager,
    text: &str,
) -> xiaozhi_core::Result<()> {
    let text = sanitize_warmup_text(text).unwrap_or_default();
    if text.is_empty() {
        return Ok(());
    }
    if task.try_mark_speech_started() {
        tts.enqueue_tts_start(&format!(
            "OpenClaw warmup start correlation_id={}",
            task.correlation_id
        ))
        .await;
    }
    let is_start = task.take_segment_start();
    tts.handle_text_response(
        LlmResponseChunk {
            text,
            is_start,
            is_end: true,
        },
        None,
        None,
    )
    .await?;
    Ok(())
}

async fn generate_warmup_plan(
    llm: &Arc<dyn LlmProvider>,
    session_id: &str,
    user_text: &str,
) -> xiaozhi_core::Result<Vec<String>> {
    let dialogue = vec![
        ChatMessage::system(WARMUP_SYSTEM_PROMPT),
        ChatMessage::user(build_warmup_user_prompt(user_text)),
    ];
    let mut rx = llm
        .response_with_context(session_id, &dialogue, &[])
        .await?;
    let mut raw = String::new();
    while let Some(msg) = rx.recv().await {
        if !msg.content.is_empty() {
            raw.push_str(&msg.content);
        }
    }
    let lines = parse_warmup_plan(&raw);
    if count_warmup_lines(&lines) == 0 {
        return Err(xiaozhi_core::Error::Session("empty warmup plan".into()));
    }
    Ok(lines)
}

fn build_warmup_user_prompt(user_text: &str) -> String {
    let trimmed = user_text.trim();
    let topic = format_warmup_topic(&build_warmup_hint(user_text));
    let topic_line = if topic.is_empty() {
        "不要复述“帮我查一下”这类用户指令。".to_string()
    } else {
        format!(
            "如果需要提到主题，只能提炼成名词短语“{topic}”，不要复述“帮我查一下”这类用户指令。"
        )
    };
    format!(
        "用户本轮任务：\n{trimmed}\n\n{topic_line}\n\n实际播报时间点依次为：第1秒、第10秒、第20秒、第30秒、第40秒、第50秒、第60秒、第70秒、第80秒、第90秒、第100秒。\n请输出 11 条暖场语，并按上述 11 个时间点一一对应。"
    )
}

fn build_warmup_session_id(session_id: &str, correlation_id: &str) -> String {
    let base = session_id.trim();
    let base = if base.is_empty() { "openclaw" } else { base };
    let correlation_id = correlation_id.trim();
    let short = if correlation_id.chars().count() > 12 {
        correlation_id.chars().take(12).collect::<String>()
    } else {
        correlation_id.to_string()
    };
    if short.is_empty() {
        format!("{base}:warmup")
    } else {
        format!("{base}:warmup:{short}")
    }
}

pub fn parse_warmup_plan(raw: &str) -> Vec<String> {
    let mut lines = vec![String::new(); WARMUP_PLAN_SIZE];
    let raw = raw.trim();
    if raw.is_empty() {
        return lines;
    }
    let start = raw.find('[').unwrap_or(0);
    let end = raw.rfind(']').map(|i| i + 1).unwrap_or(raw.len());
    let candidate = &raw[start..end];

    if let Ok(value) = serde_json::from_str::<Value>(candidate) {
        if let Some(arr) = value.as_array() {
            let items: Vec<String> = arr
                .iter()
                .filter_map(|item| {
                    if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                        Some(text.to_string())
                    } else if let Some(text) = item.as_str() {
                        Some(text.to_string())
                    } else {
                        None
                    }
                })
                .collect();
            if !items.is_empty() {
                return build_warmup_plan_lines(items);
            }
        }
    }

    if let Ok(string_items) = serde_json::from_str::<Vec<String>>(candidate) {
        return build_warmup_plan_lines(string_items);
    }
    warn!("OpenClaw warmup plan parse failed, ignored: raw={raw:?}");
    lines
}

fn build_warmup_plan_lines(items: Vec<String>) -> Vec<String> {
    let mut lines = vec![String::new(); WARMUP_PLAN_SIZE];
    for (idx, item) in items.into_iter().enumerate().take(WARMUP_PLAN_SIZE) {
        if let Some(text) = sanitize_warmup_text(&item) {
            lines[idx] = text;
        }
    }
    lines
}

fn count_warmup_lines(lines: &[String]) -> usize {
    lines.iter().filter(|line| !line.trim().is_empty()).count()
}

pub fn sanitize_warmup_text(text: &str) -> Option<String> {
    let mut text = text.replace('\n', " ").trim().to_string();
    text = text.trim_matches(|c| "\"'`[]{}".contains(c)).to_string();
    text = text.trim_start_matches(|c: char| c.is_ascii_digit() || "、.- ".contains(c)).to_string();
    text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() || is_invalid_warmup_text(&text) {
        return None;
    }
    let chars: Vec<char> = text.chars().collect();
    if chars.len() > 16 {
        return None;
    }
    Some(text)
}

fn is_invalid_warmup_text(text: &str) -> bool {
    [
        "帮我", "给我", "告诉我", "请帮", "麻烦帮", "能帮我", "可以帮我", "帮忙查", "帮忙看", "帮忙问",
    ]
    .iter()
    .any(|bad| text.contains(bad))
}

pub fn build_warmup_hint(user_text: &str) -> String {
    let trimmed = user_text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut normalized = remove_punctuation(trimmed);
    if normalized.is_empty() {
        return String::new();
    }
    normalized = trim_warmup_command_prefix(&normalized);
    normalized = trim_warmup_question_suffix(&normalized);
    if normalized.is_empty() {
        return String::new();
    }
    for keyword in ["天气", "气温", "温度", "预报"] {
        if let Some(idx) = normalized.find(keyword) {
            let limit = idx + keyword.chars().count();
            let chars: Vec<char> = normalized.chars().collect();
            normalized = chars.into_iter().take(limit).collect();
            break;
        }
    }
    let mut runes: Vec<char> = normalized.chars().take(10).collect();
    while let Some(&last) = runes.last() {
        if matches!(last, '的' | '了' | '呢') {
            runes.pop();
        } else {
            break;
        }
    }
    runes.into_iter().collect()
}

fn trim_warmup_command_prefix(text: &str) -> String {
    let mut trimmed = text.trim().to_string();
    loop {
        let mut changed = false;
        for prefix in [
            "麻烦帮我查询一下",
            "麻烦帮我查一下",
            "麻烦帮我看一下",
            "请帮我查询一下",
            "请帮我查一下",
            "请帮我看一下",
            "帮我查询一下",
            "帮我查一下",
            "帮我看一下",
            "帮我问一下",
            "给我查询一下",
            "给我查一下",
            "给我看一下",
            "可以帮我查一下",
            "可以帮我看一下",
            "能帮我查一下",
            "能帮我看一下",
            "我想知道",
            "我想问一下",
            "我想问",
            "请问一下",
            "请问",
            "查询一下",
            "查一下",
            "看一下",
            "问一下",
            "帮我查询",
            "帮我查",
            "帮我看",
            "帮我问",
            "给我查询",
            "给我查",
            "给我看",
            "查询",
            "查",
            "看",
            "问",
        ] {
            if trimmed.starts_with(prefix) {
                trimmed = trimmed[prefix.len()..].trim().to_string();
                changed = true;
                break;
            }
        }
        if !changed {
            break;
        }
    }
    trimmed
}

fn trim_warmup_question_suffix(text: &str) -> String {
    let mut trimmed = text.trim().to_string();
    for suffix in ["怎么样", "如何", "多少", "是什么", "是啥", "吗", "呢", "呀", "吧"] {
        if trimmed.ends_with(suffix) {
            trimmed = trimmed[..trimmed.len() - suffix.len()].trim().to_string();
        }
    }
    trimmed
}

pub fn format_warmup_topic(hint: &str) -> String {
    let hint = hint.trim();
    if hint.is_empty() {
        return String::new();
    }
    for keyword in ["天气", "气温", "温度", "预报"] {
        if let Some(idx) = hint.find(keyword) {
            if idx > 0 {
                let prefix = hint[..idx].trim();
                if prefix.is_empty() || prefix.ends_with('的') {
                    return hint.to_string();
                }
                return format!("{prefix}的{}", &hint[idx..]);
            }
        }
    }
    hint.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_warmup_plan_objects() {
        let got = parse_warmup_plan(
            r#"[{"text":"我先看一下天气。"},{"text":"天气情况我继续跟进。"},{"text":"这个问题我还在处理。"}]"#,
        );
        assert_eq!(got[0], "我先看一下天气。");
        assert_eq!(got[1], "天气情况我继续跟进。");
    }

    #[test]
    fn parses_invalid_json_returns_empty_slots() {
        let got = parse_warmup_plan("not-json");
        assert!(got.iter().all(|line| line.is_empty()));
    }

    #[test]
    fn builds_weather_hint() {
        let got = build_warmup_hint("天津后天的天气怎么样？");
        assert!(got.contains("天气"));
    }

    #[test]
    fn rejects_user_command_echo() {
        assert!(sanitize_warmup_text("我先看看帮我查询一下。").is_none());
    }
}
