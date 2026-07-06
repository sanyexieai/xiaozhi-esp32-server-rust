//! LLM 响应队列（对齐 Go `LLMManager` + `HandleLLMResponseChannelSync`）

use std::sync::Arc;
use std::sync::Weak;

use tokio::sync::{mpsc, oneshot};
use xiaozhi_core::Result;
use xiaozhi_llm::{ChatMessage, ToolInfo};

use crate::mcp_tools::{food_delivery_tool_retry_hint, user_text_prefers_food_delivery_tools};
use xiaozhi_protocol::messages::ServerMessage;

use crate::llm_types::LlmResponseChunk;
use crate::manager::ChatManager;
use crate::outbound::SpeakDelivery;
use crate::sentence::SentenceBuffer;
use crate::tts_manager::TtsManager;

const LLM_RESPONSE_QUEUE_CAP: usize = 10;

struct LlmResponseQueueItem {
    dialogue: Vec<ChatMessage>,
    tools: Vec<ToolInfo>,
    result_tx: oneshot::Sender<Result<LlmTurnResult>>,
}

#[derive(Debug)]
pub struct LlmTurnResult {
    pub full_text: String,
    pub delivery: SpeakDelivery,
}

pub struct LlmManager {
    device_id: String,
    manager: Weak<ChatManager>,
    queue_tx: mpsc::Sender<LlmResponseQueueItem>,
}

impl LlmManager {
    pub fn new(device_id: String, manager: Weak<ChatManager>, tts: Arc<TtsManager>) -> Arc<Self> {
        let (queue_tx, queue_rx) = mpsc::channel(LLM_RESPONSE_QUEUE_CAP);
        let mgr = Arc::new(Self {
            device_id,
            manager,
            queue_tx,
        });

        let worker = Arc::clone(&mgr);
        tokio::spawn(async move {
            worker.process_llm_response_queue(queue_rx, tts).await;
        });

        mgr
    }

    /// 对齐 Go `DoLLmRequest` + `HandleLLMResponseChannelSync`（actionDoChat 主路径）
    pub async fn do_llm_request(
        &self,
        dialogue: Vec<ChatMessage>,
        tools: Vec<ToolInfo>,
    ) -> Result<LlmTurnResult> {
        let (result_tx, result_rx) = oneshot::channel();
        self.queue_tx
            .send(LlmResponseQueueItem {
                dialogue,
                tools,
                result_tx,
            })
            .await
            .map_err(|e| xiaozhi_core::Error::Session(format!("llmResponseQueue 已满: {e}")))?;
        result_rx
            .await
            .map_err(|_| xiaozhi_core::Error::Session("LLM 队列 worker 已退出".into()))?
    }

    /// 对齐 Go `HandleWelcome`：异步 goroutine 内 EnqueueTtsStart → TTS → EnqueueTtsStop
    pub async fn handle_welcome_tts(&self, text: &str) -> Result<()> {
        let manager = self
            .manager
            .upgrade()
            .ok_or_else(|| xiaozhi_core::Error::Session("ChatManager 已释放".into()))?;
        manager.clear_session_abort().await;
        let tts = manager
            .tts_manager()
            .await
            .ok_or_else(|| xiaozhi_core::Error::Session("TTS 管理器未初始化".into()))?;
        tts.enqueue_tts_start("HandleWelcome").await;
        tts.handle_text_response_sync_with_on_start(text, None).await?;
        tts.finish_tts_turn("HandleWelcome natural end").await;
        Ok(())
    }

    /// 对齐 Go `AddTextToTTSQueue`：跳过 LLM，直接 TTS 队列
    pub async fn add_text_to_tts_queue(&self, text: &str) -> Result<()> {
        self.add_text_to_tts_queue_with_on_start(text, None).await
    }

    pub async fn add_text_to_tts_queue_with_on_start(
        &self,
        text: &str,
        on_start: Option<crate::tts_manager::TtsPlaybackStartHook>,
    ) -> Result<()> {
        let manager = self
            .manager
            .upgrade()
            .ok_or_else(|| xiaozhi_core::Error::Session("ChatManager 已释放".into()))?;
        let tts = manager
            .tts_manager()
            .await
            .ok_or_else(|| xiaozhi_core::Error::Session("TTS 管理器未初始化".into()))?;
        // 对齐 Go HandleWelcome / HandleLLMResponseChannelSync：先向 session 音频队列投递
        // tts start，由 runSenderLoop 先发 MQTT 信令再播 UDP；handleTts 见 tts_active 后不再重复下发。
        tts.enqueue_tts_start("AddTextToTTSQueue").await;
        tts.handle_text_response_sync_with_on_start(text, on_start).await?;
        tts.finish_tts_turn("AddTextToTTSQueue").await;
        Ok(())
    }

    pub fn clear_llm_response_queue(&self) {
        // 新任务会通过 generation/abort 丢弃；队列项仍在飞行中由 worker 检查 abort
    }

    async fn process_llm_response_queue(
        &self,
        mut rx: mpsc::Receiver<LlmResponseQueueItem>,
        tts: Arc<TtsManager>,
    ) {
        while let Some(item) = rx.recv().await {
            let result = self
                .handle_llm_request_sync(item.dialogue, item.tools, &tts)
                .await;
            let _ = item.result_tx.send(result);
        }
    }

    async fn handle_llm_request_sync(
        &self,
        mut dialogue: Vec<ChatMessage>,
        tools: Vec<ToolInfo>,
        tts: &TtsManager,
    ) -> Result<LlmTurnResult> {
        let manager = self
            .manager
            .upgrade()
            .ok_or_else(|| xiaozhi_core::Error::Session("ChatManager 已释放".into()))?;
        manager.clear_session_abort().await;
        let mut delivery = SpeakDelivery::default();

        if tools.is_empty() {
            return self
                .run_streaming_llm_turn(&manager, &mut dialogue, &tools, tts, &mut delivery)
                .await;
        }

        const MAX_TOOL_ROUNDS: usize = 5;
        let last_user_text = dialogue
            .iter()
            .rev()
            .find(|m| m.role == xiaozhi_llm::MessageRole::User)
            .map(|m| m.content.clone())
            .unwrap_or_default();
        for round in 0..MAX_TOOL_ROUNDS {
            if manager.is_session_aborted().await {
                break;
            }

            let media = manager
                .session_media()
                .await
                .ok_or_else(|| xiaozhi_core::Error::Session("SessionMedia 未初始化".into()))?;

            let completion = media
                .llm
                .complete_with_context(&media.session_id, &dialogue, &tools)
                .await?;

            if completion.has_tool_calls() && round + 1 < MAX_TOOL_ROUNDS {
                dialogue.push(completion.to_assistant_message());
                let mut stop_llm = false;
                for call in &completion.tool_calls {
                    let outcome = manager
                        .execute_mcp_tool_for_llm(
                            &call.name,
                            call.arguments.clone(),
                            &call.id,
                        )
                        .await;
                    if outcome.stop_llm {
                        stop_llm = true;
                    }
                    dialogue.push(ChatMessage::tool(outcome.text, &call.id));
                }
                if stop_llm {
                    return Ok(LlmTurnResult {
                        full_text: String::new(),
                        delivery,
                    });
                }
                continue;
            }

            if !completion.content.is_empty() {
                if round == 0
                    && !completion.has_tool_calls()
                    && user_text_prefers_food_delivery_tools(&last_user_text, &tools)
                {
                    tracing::info!(
                        device_id = %manager.device_id(),
                        user_text = %last_user_text,
                        "外卖/点餐意图未走 MCP，追加提示后重试"
                    );
                    dialogue.push(ChatMessage::system(food_delivery_tool_retry_hint()));
                    continue;
                }
                return self
                    .run_tts_for_text(
                        &manager,
                        &mut dialogue,
                        &completion.content,
                        tts,
                        &mut delivery,
                    )
                    .await;
            }
            break;
        }

        tracing::warn!(
            device_id = %manager.device_id(),
            tool_count = tools.len(),
            "LLM 工具路径无文本回复，回退流式对话"
        );
        manager.clear_session_abort().await;
        let streaming = self
            .run_streaming_llm_turn(&manager, &mut dialogue, &[], tts, &mut delivery)
            .await?;
        if !streaming.full_text.trim().is_empty() {
            return Ok(streaming);
        }

        const FALLBACK: &str = "抱歉，我刚刚没想好怎么说，请再说一遍。";
        tracing::warn!(
            device_id = %manager.device_id(),
            "LLM 流式兜底仍为空，播报默认提示"
        );
        self.run_tts_for_text(&manager, &mut dialogue, FALLBACK, tts, &mut delivery)
            .await
    }

    async fn run_streaming_llm_turn(
        &self,
        manager: &Arc<ChatManager>,
        dialogue: &mut Vec<ChatMessage>,
        tools: &[ToolInfo],
        tts: &TtsManager,
        delivery: &mut SpeakDelivery,
    ) -> Result<LlmTurnResult> {
        let media = manager
            .session_media()
            .await
            .ok_or_else(|| xiaozhi_core::Error::Session("SessionMedia 未初始化".into()))?;

        let (response_tx, response_rx) = mpsc::channel::<LlmResponseChunk>(32);
        let session_id = media.session_id.clone();
        let llm = Arc::clone(&media.llm);
        let tools = tools.to_vec();
        let dialogue_for_llm = dialogue.clone();

        tokio::spawn(async move {
            let Ok(mut rx) = llm
                .response_with_context(&session_id, &dialogue_for_llm, &tools)
                .await
            else {
                let _ = response_tx.send(LlmResponseChunk::end()).await;
                return;
            };
            let mut sentence_buf = SentenceBuffer::new();
            let mut first = true;
            while let Some(msg) = rx.recv().await {
                if msg.is_error() {
                    if let Some(err) = msg.error_text() {
                        tracing::warn!("LLM 流式错误: {err}");
                    }
                    break;
                }
                if msg.content.is_empty() {
                    continue;
                }
                for sentence in sentence_buf.push_delta(&msg.content) {
                    let _ = response_tx
                        .send(LlmResponseChunk {
                            text: sentence,
                            is_start: first,
                            is_end: false,
                        })
                        .await;
                    first = false;
                }
            }
            if let Some(tail) = sentence_buf.flush() {
                let _ = response_tx
                    .send(LlmResponseChunk {
                        text: tail,
                        is_start: first,
                        is_end: false,
                    })
                    .await;
            }
            let _ = response_tx.send(LlmResponseChunk::end()).await;
        });

        tts.enqueue_tts_start("LLMManager.HandleLLMResponseChannelSync start")
            .await;
        let full_text = self
            .handle_llm_response(response_rx, tts, Arc::clone(&manager))
            .await?;
        tts.finish_tts_turn("LLMManager.HandleLLMResponseChannelSync end")
            .await;

        if !full_text.is_empty() {
            delivery.messages.push(ServerMessage::llm(
                full_text.clone(),
                Some(media.session_id.clone()),
            ));
            manager
                .persist_assistant_reply(&full_text, dialogue)
                .await;
        }

        Ok(LlmTurnResult {
            full_text,
            delivery: std::mem::take(delivery),
        })
    }

    async fn run_tts_for_text(
        &self,
        manager: &Arc<ChatManager>,
        dialogue: &mut Vec<ChatMessage>,
        text: &str,
        tts: &TtsManager,
        delivery: &mut SpeakDelivery,
    ) -> Result<LlmTurnResult> {
        let media = manager
            .session_media()
            .await
            .ok_or_else(|| xiaozhi_core::Error::Session("SessionMedia 未初始化".into()))?;

        tts.enqueue_tts_start("LLMManager.complete_with_context").await;
        if let Err(e) = tts.handle_text_response_sync(text).await {
            tracing::warn!(
                device_id = %manager.device_id(),
                "TTS 处理完整 LLM 回复失败，保留文本回复继续对话: {e:#}"
            );
        }
        tts.finish_tts_turn("LLMManager.complete_with_context").await;

        delivery.messages.push(ServerMessage::llm(
            text.to_string(),
            Some(media.session_id.clone()),
        ));
        manager.persist_assistant_reply(text, dialogue).await;

        Ok(LlmTurnResult {
            full_text: text.to_string(),
            delivery: std::mem::take(delivery),
        })
    }

    /// 对齐 Go `handleLLMResponse`：从 channel 读分片 → `handleTextResponse`
    async fn handle_llm_response(
        &self,
        mut response_rx: mpsc::Receiver<LlmResponseChunk>,
        tts: &TtsManager,
        manager: Arc<ChatManager>,
    ) -> Result<String> {
        let mut full_text = String::new();
        while let Some(chunk) = response_rx.recv().await {
            if manager.is_session_aborted().await {
                break;
            }
            if !chunk.text.is_empty() {
                full_text.push_str(&chunk.text);
                if let Err(e) = tts.handle_text_response(chunk, None, None).await {
                    tracing::warn!(
                        device_id = %manager.device_id(),
                        "TTS 处理 LLM 分句失败，保留文本回复继续对话: {e:#}"
                    );
                }
            } else if chunk.is_end {
                break;
            }
        }
        Ok(full_text)
    }
}
