//! 注册可在 MCP 协议层直接调用的本地工具。
//! `exit_conversation`、`clear_conversation_history` 等由 `xiaozhi-chat::execute_tool` 拦截处理。

use chrono::Local;
use serde_json::json;

use crate::manager::McpManager;

pub fn register_default_tools(manager: &McpManager) {    manager.register_local_tool(
        "get_current_datetime",
        std::sync::Arc::new(|_, _| Ok(json!(Local::now().format("%Y-%m-%d %H:%M:%S").to_string()))),
    );

    manager.register_local_tool(
        "exit_conversation",
        std::sync::Arc::new(|_, _| Ok(json!("好的，再见！"))),
    );

    manager.register_local_tool(
        "clear_conversation_history",
        std::sync::Arc::new(|_, _| Ok(json!("对话历史已清空"))),
    );
}
