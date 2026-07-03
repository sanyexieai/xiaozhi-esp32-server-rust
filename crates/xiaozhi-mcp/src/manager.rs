use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use dashmap::DashMap;
use serde_json::{json, Value};
use xiaozhi_config::{LocalMcpConfig, McpGlobalConfig, McpServerEntry};

use crate::global_hub::GlobalMcpHub;
use crate::types::{McpRequest, McpResponse, McpTool, McpToolEntry, McpToolGroup};

pub const BUILTIN_MCP_SERVER: &str = "内置服务";
pub const DEVICE_MCP_SERVER: &str = "设备 MCP";

pub type ToolHandler = Arc<dyn Fn(&str, Value) -> Result<Value, String> + Send + Sync>;

struct GlobalMcpState {
    servers: Vec<McpServerEntry>,
    hub: Option<Arc<GlobalMcpHub>>,
}

pub struct McpManager {
    global: RwLock<GlobalMcpState>,
    local_mcp: RwLock<LocalMcpConfig>,
    local_tools: DashMap<String, ToolHandler>,
    transport_resolver: DashMap<String, String>,
}

impl McpManager {
    pub fn new(global_servers: Vec<McpServerEntry>, global_hub: Option<Arc<GlobalMcpHub>>) -> Self {
        Self {
            global: RwLock::new(GlobalMcpState {
                servers: global_servers,
                hub: global_hub,
            }),
            local_mcp: RwLock::new(LocalMcpConfig::default()),
            local_tools: DashMap::new(),
            transport_resolver: DashMap::new(),
        }
    }

    pub fn reload_local_mcp(&self, config: &LocalMcpConfig) {
        let mut guard = self.local_mcp.write().unwrap_or_else(|e| e.into_inner());
        *guard = config.clone();
        tracing::debug!("本地 MCP 工具开关已重载");
    }

    pub fn is_local_tool_enabled(&self, name: &str) -> bool {
        self.local_mcp
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .is_tool_enabled(name)
    }

    pub fn reload_global(&self, config: &McpGlobalConfig) {
        let hub = if config.enabled {
            Some(GlobalMcpHub::start(config))
        } else {
            None
        };
        let mut global = self.global.write().unwrap_or_else(|e| e.into_inner());
        if let Some(old) = global.hub.take() {
            old.shutdown();
        }
        global.servers = config.servers.clone();
        global.hub = hub;
        tracing::info!(
            "全局 MCP 已重载: enabled={}, servers={}",
            config.enabled,
            global.servers.len()
        );
    }

    pub fn register_local_tool(&self, name: &str, handler: ToolHandler) {
        self.local_tools.insert(name.to_string(), handler);
    }

    pub fn register_transport_resolver(
        &self,
        device_id: &str,
        transport_type: &str,
    ) {
        self.transport_resolver
            .insert(device_id.to_string(), transport_type.to_string());
    }

    pub fn list_local_tools(&self) -> Vec<McpTool> {
        let all = vec![
            McpTool {
                name: "get_current_datetime".into(),
                description: "获取当前日期时间".into(),
                input_schema: json!({"type": "object", "properties": {}}),
            },
            McpTool {
                name: "exit_conversation".into(),
                description: "退出当前对话".into(),
                input_schema: json!({"type": "object", "properties": {}}),
            },
            McpTool {
                name: "clear_conversation_history".into(),
                description: "清空对话历史".into(),
                input_schema: json!({"type": "object", "properties": {}}),
            },
            McpTool {
                name: "switch_device_role".into(),
                description: "切换设备角色".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {"role_name": {"type": "string"}},
                    "required": ["role_name"]
                }),
            },
            McpTool {
                name: "restore_device_default_role".into(),
                description: "恢复设备默认角色".into(),
                input_schema: json!({"type": "object", "properties": {}}),
            },
            McpTool {
                name: "search_knowledge".into(),
                description: "搜索知识库".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "要检索的查询内容"},
                        "top_k": {"type": "integer", "description": "返回条数，默认5"},
                        "knowledge_base_ids": {
                            "type": "array",
                            "items": {"type": "integer"},
                            "description": "可选：仅在这些知识库ID内检索"
                        }
                    },
                    "required": ["query"]
                }),
            },
            McpTool {
                name: "play_music".into(),
                description: "当用户想听歌时使用，播放指定名称的音乐".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {"name": {"type": "string", "description": "音乐名称"}},
                    "required": ["name"]
                }),
            },
            McpTool {
                name: "control_music_playback".into(),
                description: "控制音乐播放：resume/pause/stop/prev/next".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "description": "resume、pause、stop、prev、next、play_playlist"
                        }
                    },
                    "required": ["action"]
                }),
            },
        ];
        all.into_iter()
            .filter(|tool| self.is_local_tool_enabled(&tool.name))
            .collect()
    }

    pub async fn handle_request(&self, req: McpRequest) -> McpResponse {
        if req.method == "tools/list" {
            return McpResponse {
                jsonrpc: "2.0".into(),
                id: req.id,
                result: Some(json!({"tools": self.list_local_tools()})),
                error: None,
            };
        }

        if req.method == "tools/call" {
            let name = req.params["name"].as_str().unwrap_or("");
            let args = req.params["arguments"].clone();

            if !self.is_local_tool_enabled(name) {
                return McpResponse {
                    jsonrpc: "2.0".into(),
                    id: req.id,
                    result: None,
                    error: Some(crate::types::McpError {
                        code: -32601,
                        message: format!("本地工具已禁用: {name}"),
                    }),
                };
            }

            if let Some(handler) = self.local_tools.get(name) {
                match handler(name, args) {
                    Ok(result) => {
                        return McpResponse {
                            jsonrpc: "2.0".into(),
                            id: req.id,
                            result: Some(json!({"content": [{"type": "text", "text": result.to_string()}]})),
                            error: None,
                        };
                    }
                    Err(e) => {
                        return McpResponse {
                            jsonrpc: "2.0".into(),
                            id: req.id,
                            result: None,
                            error: Some(crate::types::McpError {
                                code: -32000,
                                message: e,
                            }),
                        };
                    }
                }
            }
        }

        McpResponse {
            jsonrpc: "2.0".into(),
            id: req.id,
            result: None,
            error: Some(crate::types::McpError {
                code: -32601,
                message: format!("Method not found: {}", req.method),
            }),
        }
    }

    pub fn enabled_global_servers(&self) -> Vec<McpServerEntry> {
        self.global
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .servers
            .clone()
    }

    pub async fn list_all_tools(&self) -> Vec<McpTool> {
        self.list_all_tool_entries()
            .await
            .into_iter()
            .map(|entry| McpTool {
                name: entry.name,
                description: entry.description,
                input_schema: entry.input_schema,
            })
            .collect()
    }

    pub async fn list_all_tool_entries(&self) -> Vec<McpToolEntry> {
        let mut entries: Vec<McpToolEntry> = self
            .list_local_tools()
            .into_iter()
            .map(|tool| tool_entry_from_mcp_tool(tool, BUILTIN_MCP_SERVER))
            .collect();
        let local_names: std::collections::HashSet<_> =
            entries.iter().map(|t| t.name.clone()).collect();
        let hub = self
            .global
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .hub
            .clone();
        if let Some(hub) = hub {
            for (server_name, tool) in hub.list_tools_with_server() {
                if local_names.contains(&tool.name) {
                    continue;
                }
                entries.push(tool_entry_from_mcp_tool(tool, &server_name));
            }
        }
        entries
    }

    pub fn group_tool_entries(entries: &[McpToolEntry]) -> Vec<McpToolGroup> {
        use std::collections::HashMap;

        let mut order: Vec<String> = Vec::new();
        let mut grouped: HashMap<String, Vec<McpTool>> = HashMap::new();
        for entry in entries {
            if !grouped.contains_key(&entry.server_name) {
                order.push(entry.server_name.clone());
                grouped.insert(entry.server_name.clone(), Vec::new());
            }
            grouped
                .get_mut(&entry.server_name)
                .expect("server group exists")
                .push(McpTool {
                    name: entry.name.clone(),
                    description: entry.description.clone(),
                    input_schema: entry.input_schema.clone(),
                });
        }
        order
            .into_iter()
            .filter_map(|server_name| {
                grouped.remove(&server_name).map(|tools| McpToolGroup {
                    server_name,
                    tools,
                })
            })
            .collect()
    }

    pub async fn global_tool_count(&self) -> usize {
        self.global
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .hub
            .as_ref()
            .map(|h| h.tool_count())
            .unwrap_or(0)
    }

    pub async fn call_global_tool(&self, name: &str, arguments: Value) -> Result<String, String> {
        let hub = self.global_hub()?;
        hub.call_tool(name, arguments).await
    }

    pub async fn call_global_tool_raw(&self, name: &str, arguments: Value) -> Result<Value, String> {
        let hub = self.global_hub()?;
        hub.call_tool_raw(name, arguments).await
    }

    pub async fn read_global_resource(
        &self,
        tool_name: &str,
        uri: &str,
        arguments: Value,
    ) -> Result<Value, String> {
        let hub = self.global_hub()?;
        hub.read_resource(tool_name, uri, arguments).await
    }

    fn global_hub(&self) -> Result<std::sync::Arc<GlobalMcpHub>, String> {
        self.global
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .hub
            .clone()
            .ok_or_else(|| "全局 MCP 未启用".to_string())
    }

    pub fn has_global_tool(&self, name: &str) -> bool {
        self.global
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .hub
            .as_ref()
            .is_some_and(|h| h.has_tool(name))
    }
}

fn tool_entry_from_mcp_tool(tool: McpTool, server_name: &str) -> McpToolEntry {
    McpToolEntry {
        name: tool.name,
        description: tool.description,
        input_schema: tool.input_schema,
        server_name: server_name.to_string(),
    }
}

#[async_trait]
pub trait McpClient: Send + Sync {
    async fn list_tools(&self) -> Result<Vec<McpTool>, String>;
    async fn call_tool(&self, name: &str, args: Value) -> Result<Value, String>;
}
