# 实现状态说明

本文档记录当前项目的功能状态、已知限制和维护注意事项。状态描述以本仓库当前实现为准，不包含与其他项目的对比。

**审计日期**：2026-07-03

## 状态图例

| 标记 | 含义 |
|------|------|
| ✅ 完成 | 主路径可用，可作为当前项目能力使用 |
| 🟡 部分 | 有实现，但依赖额外服务、配置或存在能力边界 |
| 🔴 占位 | 固定返回、空实现或仅日志，不应视为已交付 |
| ⚪ 未接入 | 代码存在，但运行时尚未接入主路径 |
| ❌ 缺失 | 前端或配置有入口，但后端尚无对应能力 |

## 一、总览

| 领域 | 状态 | 说明 |
|------|------|------|
| 设备 WebSocket 对话主链路 | ✅ | VAD → ASR → LLM → TTS 主路径可用 |
| MQTT + UDP 设备接入 | ✅ | 支持 MQTT 信令、UDP 加密音频、设备 hello/goodbye |
| Manager 控制台 | ✅ | 用户、设备、智能体、多数配置 CRUD 可用 |
| 配置连通性测试 | 🟡 | 需 `xiaozhi-server` 与 Manager Bridge 在线 |
| Web 设备对话模拟 | 🟡 | 文本对话可用；语音、多模态仍在扩展 |
| 知识库 | ✅ | multipart 上传、Dify/RAGFlow/WeKnora 同步与检索已接线 |
| 声音复刻 | ✅ | 多 provider 异步任务、额度、重试、试听可用 |
| 声纹组 | ✅ | CRUD、样本、验证、远程清理可用 |
| MCP 工具 | 🟡 | 本地工具、全局工具、设备侧 MCP 均已接入，远程服务质量取决于配置 |
| VAD | 🟡 | Silero 需 ONNX 模型；TEN 需预编译库 |
| ASR | ✅ | 多 provider 真流式识别 |
| LLM | ✅ | OpenAI 兼容族主路径可用；Dify/Coze 为 SSE 文本流 |
| TTS | 🟡 | 多 provider 可合成；运行时切换和音频格式能力因 provider 而异 |
| Memory | 🟡 | 支持多个 provider；实际效果取决于外部服务配置 |
| RAG crate | 🟡 | Manager 内部检索已接入；本地检索仍为关键词路径 |

## 二、Manager API

### 2.1 知识库

| API | 状态 | 说明 |
|-----|------|------|
| `GET /user/knowledge-bases` | ✅ | 含全局知识库配置 |
| `POST .../sync` | ✅ | 异步触发知识库同步 |
| `POST .../documents/{id}/sync` | ✅ | 单文档外部同步与轮询 |
| `POST .../documents/upload` | ✅ | multipart 上传与扩展名校验 |
| `POST .../test-search` | ✅ | 走统一知识检索 |
| `create` / `update` / `delete` | ✅ | 写入 DB；非 local 可触发外部同步 |

### 2.2 声音复刻

| API | 状态 | 说明 |
|-----|------|------|
| `POST /user/voice-clones` | ✅ | 支持 minimax、cosyvoice、aliyun_qwen、indextts_vllm、doubao 等任务 |
| `POST .../retry` | ✅ | 仅失败任务可重试 |
| `GET /user/voice-clone/capabilities` | ✅ | 按 provider 返回能力 |
| `GET/PUT /admin/users/{id}/voice-clone-quotas` | ✅ | 额度持久化 |
| `GET .../preview` | ✅ | 已成功音色可试听 |
| `POST .../append-audio` | ✅ | IndexTTS 成功后可追加参考音频 |

启动时会恢复 `queued` / `processing` 的声音复刻任务。

### 2.3 声纹组

| API | 状态 | 说明 |
|-----|------|------|
| `GET /user/speaker-groups` | ✅ | 含 `agent_name` 与分页 |
| `GET /user/speaker-groups/{id}` | ✅ | 详情含样本 |
| `POST` / `PUT` | ✅ | 智能体归属校验与同用户重名校验 |
| 样本增删 / 验证 | ✅ | 远程声纹服务可选 |
| `DELETE` 声纹组 | ✅ | 远程整组删除、本地样本清理、DB 级联 |

### 2.4 MCP 与配置辅助

| API | 状态 | 说明 |
|-----|------|------|
| `POST /admin/mcp-configs/discover-tools` | ✅ | 探测 HTTP / SSE MCP 服务工具列表 |
| `POST /admin/knowledge-search-configs/weknora/models` | ✅ | 拉取 WeKnora 模型并分类 |
| `POST /admin/configs/test` | 🟡 | 部分测试需转发到已连接的 `xiaozhi-server` |

### 2.5 OpenClaw 对话测试

| API | 状态 | 说明 |
|-----|------|------|
| `GET .../openclaw-endpoint` | ✅ | 经 Bridge 查询服务状态 |
| `POST .../openclaw-chat-test?stream=1` | ✅ | SSE 流式返回 |
| 非流式 fallback | ✅ | 单次请求返回 JSON |
| 参数校验 | ✅ | `message` 必填；校验智能体归属 |

依赖：`xiaozhi-server` 已连接 Manager Bridge，且 OpenClaw 客户端已连接对应智能体。

## 三、运行时引擎

### 3.1 VAD

| Provider | 状态 | 说明 |
|----------|------|------|
| `webrtc_vad` | ✅ | WebRTC VAD |
| `ten_vad` | ✅ | FFI 绑定，需预编译库 |
| `silero_vad` | ✅ | ONNX Runtime，需模型文件 |

### 3.2 ASR

| Provider | 状态 | 说明 |
|----------|------|------|
| `funasr` / `aliyun_funasr` / `aliyun_qwen3` / `doubao` / `xunfei` | ✅ | WebSocket 流式识别，中间结果与 final |

### 3.3 LLM

| Provider | 对话 | 工具调用 |
|----------|------|----------|
| OpenAI 兼容族 | ✅ 流式 | 🟡 工具主路径走非流式上下文请求 |
| `dify` / `coze` | 🟡 SSE 文本流 | ❌ 不支持 tool calls |
| `ollama` / `eino` | 🟡 OpenAI 兼容客户端 | 同 OpenAI 兼容族 |

### 3.4 TTS

| 项 | 状态 | 说明 |
|----|------|------|
| provider 创建 | 🟡 | 多数 provider 可实例化并合成 |
| 运行时切换音色 | 🟡 | edge、doubao_ws、zhipu、minimax、aliyun_qwen、openai、doubao_http、cosyvoice、xunfei、edge_offline 已支持 |
| Opus 下行 | 🟡 | 设备侧播放依赖传输通道和 provider 输出格式 |

### 3.5 Memory

| Provider | 状态 | 说明 |
|----------|------|------|
| `nomemo` | ✅ | 不启用记忆 |
| `mem0` / `memos` | 🟡 | 依赖外部服务 API |
| `memobase` | 🟡 | 依赖外部服务 API |

### 3.6 RAG

| 组件 | 状态 | 说明 |
|------|------|------|
| `create_searcher` | 🟡 | 支持 local、dify、ragflow、weknora |
| `KnowledgeClient` | 🟡 | 调 Manager 检索 |
| 本地检索 | 🟡 | 当前为 SQLite 关键词路径 |

### 3.7 MCP

| 能力 | 状态 | 说明 |
|------|------|------|
| 本地工具 | ✅ | 时间、退出对话、清空历史等 |
| 全局 MCP | 🟡 | 后台同步工具列表；远程服务不可用会产生告警 |
| 设备侧 MCP | 🟡 | 支持设备工具初始化与调用；Web 模拟器默认不启用设备侧 MCP |
| 知识库工具 | 🟡 | 有可用知识库时向 LLM 暴露 |
| 音乐播放工具 | ✅ | 支持搜索、播放、暂停、恢复、停止、切歌 |

## 四、前端能力

| 页面 | 状态 | 说明 |
|------|------|------|
| 设备管理 | ✅ | 设备列表、状态、调试抽屉 |
| 智能体管理 | ✅ | 配置、角色、工具和知识库绑定 |
| 配置页 | 🟡 | 多数配置可保存；测试能力依赖 Bridge |
| 知识库 | ✅ | 文件上传、外部同步、检索测试 |
| 声音复刻 | ✅ | 任务、重试、额度、试听 |
| 声纹组 | ✅ | CRUD、样本、验证 |
| MCP 配置 | 🟡 | 服务探测、导入、工具发现 |
| OpenClaw 测试 | ✅ | SSE 流式调试 |
| 设备对话模拟 | 🟡 | 文本对话可用；语音、多模态为后续能力 |

## 五、配置测试说明

| 类型 | 执行位置 | 说明 |
|------|----------|------|
| `ota` | Manager | 配置格式与部分连通性 |
| `vad` / `asr` / `llm` / `tts` / `memory` | Server | 通过 Bridge 转发执行 |
| `knowledge_search` / `mcp` | Manager | 本地探测外部服务 |

无 Bridge 连接时，Manager 会返回「没有已连接的主服务客户端」。

## 六、维护说明

- 新增 API、配置或前端入口后，请更新本文对应状态。
- 修复占位实现后，将状态从 🔴 调整为 🟡 或 ✅，并写明实际限制。
- 定期搜索 `占位`、`stub`、`TODO`、`unimplemented`、`Vec::new()`、`json!({})`，避免页面存在但后端无能力。
- 涉及外部 provider 的能力应注明依赖项，例如模型文件、动态库、API key、远程服务地址。

## 附录：Provider 支持列表

| Crate | 已注册 provider（节选） |
|-------|-------------------------|
| ASR | funasr, aliyun_funasr, aliyun_qwen3, doubao, xunfei |
| TTS | edge, openai, doubao, doubao_ws, cosyvoice, edge_offline, xiaozhi, xunfei, xunfei_super_tts, zhipu, minimax, aliyun_qwen, indextts_vllm |
| LLM | openai 兼容族, dify, coze, ollama |
| Memory | nomemo, memobase, mem0, memos |
| RAG | local, dify, ragflow, weknora |
| VAD | webrtc_vad, silero_vad, ten_vad |
