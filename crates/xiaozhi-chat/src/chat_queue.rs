//! 用户文本异步队列（对齐 Go `chatTextQueue` + `processChatText`）

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::manager::ChatManager;
use crate::outbound::SpeakDelivery;
use crate::state::ListenPhase;

const CHAT_TEXT_QUEUE_CAP: usize = 10;

#[derive(Debug, Clone)]
pub struct ChatTextJob {
    pub text: String,
    pub send_stt: bool,
}

pub struct ChatTextQueue {
    tx: mpsc::Sender<ChatTextJob>,
}

impl ChatTextQueue {
    pub fn new(manager: Arc<ChatManager>) -> Self {
        let (tx, mut rx) = mpsc::channel(CHAT_TEXT_QUEUE_CAP);
        tokio::spawn(async move {
            while let Some(job) = rx.recv().await {
                let _ = process_chat_job(&manager, job).await;
            }
        });
        Self { tx }
    }

    pub fn try_enqueue(&self, job: ChatTextJob) -> bool {
        self.tx.try_send(job).is_ok()
    }
}

async fn process_chat_job(manager: &Arc<ChatManager>, job: ChatTextJob) -> xiaozhi_core::Result<()> {
    if let Err(e) = process_chat_job_inner(manager, job).await {
        tracing::error!(device_id = %manager.device_id(), "处理对话队列失败: {e:#}");
        let delivery = {
            let mut guard = manager.session.lock().await;
            if let Some(session) = guard.as_mut() {
                session
                    .empty_listen_recovery_session("对话处理失败，请再说一遍")
                    .await
                    .unwrap_or_default()
            } else {
                SpeakDelivery::default()
            }
        };
        let _ = manager.push_delivery(&delivery).await;
        return Ok(());
    }
    Ok(())
}

async fn process_chat_job_inner(
    manager: &Arc<ChatManager>,
    job: ChatTextJob,
) -> xiaozhi_core::Result<()> {
    let stt_delivery = {
        let mut guard = manager.session.lock().await;
        let session = guard
            .as_mut()
            .ok_or_else(|| xiaozhi_core::Error::Session("会话未初始化".into()))?;
        if session.state().listen_phase == ListenPhase::Listening && !session.state().is_realtime() {
            session.state_mut().listen_phase = ListenPhase::Processing;
        }
        if job.send_stt {
            let mut early = SpeakDelivery::default();
            early.messages.push(xiaozhi_protocol::messages::ServerMessage::stt(
                job.text.clone(),
                Some(session.state().session_id.clone()),
            ));
            Some(early)
        } else {
            None
        }
    };
        if let Some(early) = stt_delivery {
        manager.push_delivery(&early).await?;
    }

    let outcome = {
        let mut guard = manager.session.lock().await;
        let session = guard
            .as_mut()
            .ok_or_else(|| xiaozhi_core::Error::Session("会话未初始化".into()))?;
        session.prepare_chat_turn(job.text, false).await?
    };

    let delivery = match outcome {
        crate::session::ChatTurnOutcome::Complete(delivery) => delivery,
        crate::session::ChatTurnOutcome::RunLlm {
            dialogue,
            tools,
            mut delivery,
        } => {
            manager.clear_session_abort().await;
            match manager.run_llm_turn(dialogue, tools).await {
                Ok(turn) => {
                    delivery.messages.extend(turn.delivery.messages);
                }
                Err(e) => {
                    tracing::error!(
                        device_id = %manager.device_id(),
                        "LLM 对话轮处理失败，返回文本兜底: {e:#}"
                    );
                    let fallback = "抱歉，我刚刚没想好怎么说，请再说一遍。";
                    let session_id = manager.session_media().await.map(|media| media.session_id);
                    delivery
                        .messages
                        .push(xiaozhi_protocol::messages::ServerMessage::llm(
                            fallback,
                            session_id,
                        ));
                    manager.persist_chat_message("assistant", fallback).await;
                }
            }
            delivery
        }
    };
    manager.push_delivery(&delivery).await?;
    manager.ensure_voice_response_after_chat_turn().await;
    Ok(())
}
