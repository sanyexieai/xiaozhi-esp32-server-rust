use reqwest::Client;
use serde_json::json;
use xiaozhi_config_provider::build_http_client;
use xiaozhi_core::Result;

#[derive(Debug, Clone)]
pub struct HistoryMessageInput {
    pub device_id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub agent_id: Option<i64>,
    pub user_id: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct DialogueMessage {
    pub role: String,
    pub content: String,
}

pub struct HistoryClient {
    base_url: String,
    auth_token: String,
    client: Client,
    enabled: bool,
}

impl HistoryClient {
    pub fn new(base_url: String, auth_token: String, enabled: bool) -> Self {
        Self {
            base_url: base_url.clone(),
            auth_token,
            client: build_http_client(&base_url),
            enabled,
        }
    }

    fn effective_base_url(&self) -> String {
        xiaozhi_config::resolve_manager_backend_url(&self.base_url)
    }

    pub async fn report_chat(
        &self,
        device_id: &str,
        session_id: &str,
        role: &str,
        content: &str,
    ) -> Result<()> {
        self.save_message(HistoryMessageInput {
            device_id: device_id.to_string(),
            session_id: session_id.to_string(),
            role: role.to_string(),
            content: content.to_string(),
            agent_id: None,
            user_id: None,
        })
        .await
    }

    pub async fn save_message(&self, msg: HistoryMessageInput) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let content = msg.content.trim();
        if content.is_empty() {
            return Ok(());
        }

        let url = format!(
            "{}/api/internal/history/messages",
            self.effective_base_url().trim_end_matches('/')
        );
        let message_id = uuid::Uuid::new_v4().to_string();
        let mut body = json!({
            "message_id": message_id,
            "device_id": msg.device_id,
            "session_id": msg.session_id,
            "role": msg.role,
            "content": content,
        });
        if let Some(agent_id) = msg.agent_id.filter(|id| *id > 0) {
            body["agent_id"] = json!(agent_id);
        }
        if let Some(user_id) = msg.user_id.filter(|id| *id > 0) {
            body["user_id"] = json!(user_id);
        }

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.auth_token))
            .json(&body)
            .send()
            .await;

        match resp {
            Ok(response) if response.status().is_success() => {}
            Ok(response) => {
                tracing::warn!(
                    device_id = %msg.device_id,
                    role = %msg.role,
                    status = %response.status(),
                    "聊天记录上报失败"
                );
            }
            Err(e) => {
                tracing::warn!(
                    device_id = %msg.device_id,
                    role = %msg.role,
                    error = %e,
                    "聊天记录上报请求失败"
                );
            }
        }

        Ok(())
    }

    pub async fn fetch_session_dialogue(
        &self,
        session_id: &str,
    ) -> Result<Vec<DialogueMessage>> {
        if !self.enabled || session_id.trim().is_empty() {
            return Ok(Vec::new());
        }
        let url = format!(
            "{}/api/internal/history/sessions/{}/dialogue",
            self.effective_base_url().trim_end_matches('/'),
            session_id.trim()
        );
        let resp = match self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.auth_token))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(session_id = %session_id, error = %e, "拉取会话上下文请求失败");
                return Ok(Vec::new());
            }
        };
        if !resp.status().is_success() {
            tracing::warn!(
                session_id = %session_id,
                status = %resp.status(),
                "拉取会话上下文失败"
            );
            return Ok(Vec::new());
        }
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        let messages = body
            .get("messages")
            .or_else(|| body.get("data").and_then(|d| d.get("messages")))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        Some(DialogueMessage {
                            role: item.get("role")?.as_str()?.to_string(),
                            content: item.get("content")?.as_str()?.to_string(),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Ok(messages)
    }

    pub async fn report_pool_stats(&self, stats: &serde_json::Value) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let url = format!(
            "{}/api/internal/pool/stats",
            self.effective_base_url().trim_end_matches('/')
        );
        let _ = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.auth_token))
            .json(stats)
            .send()
            .await;
        Ok(())
    }
}
