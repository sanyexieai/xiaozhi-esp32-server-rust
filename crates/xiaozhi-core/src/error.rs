use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("配置错误: {0}")]
    Config(String),

    #[error("认证失败: {0}")]
    Auth(String),

    #[error("协议错误: {0}")]
    Protocol(String),

    #[error("Provider 错误 [{provider}]: {message}")]
    Provider { provider: String, message: String },

    #[error("传输层错误: {0}")]
    Transport(String),

    #[error("资源池错误: {0}")]
    Pool(String),

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON 错误: {0}")]
    Json(#[from] serde_json::Error),

    #[error("HTTP 错误: {0}")]
    Http(String),

    #[error("WebSocket 错误: {0}")]
    WebSocket(String),

    #[error("MQTT 错误: {0}")]
    Mqtt(String),

    #[error("音频处理错误: {0}")]
    Audio(String),

    #[error("会话错误: {0}")]
    Session(String),

    #[error("未找到: {0}")]
    NotFound(String),

    #[error("不支持: {0}")]
    Unsupported(String),

    #[error("超时")]
    Timeout,

    #[error("已取消")]
    Cancelled,

    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn provider(provider: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Provider {
            provider: provider.into(),
            message: message.into(),
        }
    }
}
