# xiaozhi-esp32-server-rust

面向 ESP32 小智设备的模块化 AI 语音后端。项目包含设备接入服务、管理控制台、对话管线、配置管理、MCP 工具、知识库、声纹与声音复刻等能力。

## 项目结构

```text
xiaozhi-esp32-server-rust/
├── crates/
│   ├── xiaozhi-core/            # 常量、错误类型
│   ├── xiaozhi-protocol/        # 设备通信协议
│   ├── xiaozhi-config/          # YAML 系统配置与 UConfig
│   ├── xiaozhi-transport/       # WebSocket / MQTT+UDP / UDP AES
│   ├── xiaozhi-auth/            # OTA 签名、JWT、设备激活
│   ├── xiaozhi-vad/             # VAD
│   ├── xiaozhi-asr/             # ASR
│   ├── xiaozhi-llm/             # LLM
│   ├── xiaozhi-tts/             # TTS
│   ├── xiaozhi-memory/          # 记忆
│   ├── xiaozhi-mcp/             # MCP 协议与工具
│   ├── xiaozhi-rag/             # 知识库检索
│   ├── xiaozhi-speaker/         # 声纹识别
│   ├── xiaozhi-openclaw/        # OpenClaw 智能体路由
│   ├── xiaozhi-pool/            # VAD/ASR/LLM/TTS 资源池
│   ├── xiaozhi-config-provider/ # Manager / Redis 配置源
│   ├── xiaozhi-chat/            # ChatManager / ChatSession 对话管线
│   ├── xiaozhi-mqtt-server/     # 内置 MQTT Broker
│   ├── xiaozhi-eventbus/        # 内部事件总线
│   ├── xiaozhi-hooks/           # 对话生命周期 Hook
│   ├── xiaozhi-music/           # 音乐播放工具
│   ├── xiaozhi-history/         # 聊天历史
│   ├── xiaozhi-manager/         # 管理控制台 API + 静态前端
│   └── xiaozhi-server/          # 设备接入服务入口
├── frontend/                    # Vue 3 管理控制台
├── config/                      # 配置文件
├── doc/                         # 项目文档
└── Cargo.toml
```

## 功能状态

| 功能 | 状态 | 说明 |
|------|------|------|
| WebSocket 设备接入 `/xiaozhi/v1/` | ✅ | 设备和 Web 模拟器可接入 |
| OTA + 设备激活 | ✅ | 支持设备版本检查和激活流程 |
| MQTT + UDP 传输 | ✅ | 支持 MQTT 信令与 UDP Opus 音频 |
| 内置 MQTT Broker | 🟡 | 可用于开发和联调，生产环境需按部署方案评估 |
| 对话管线 VAD → ASR → LLM → TTS | ✅ | 主路径可用 |
| VAD | 🟡 | WebRTC/Silero/TEN；Silero 需 ONNX 模型，TEN 需预编译库 |
| ASR | ✅ | 多 provider 流式识别 |
| LLM | ✅ | OpenAI 兼容族、Dify、Coze、Ollama 等 |
| TTS | 🟡 | 多 provider 可用，部分 provider 能力存在差异 |
| MCP 工具 | 🟡 | 本地工具、全局工具、设备侧工具链已接入 |
| 知识库 / RAG | 🟡 | Manager 侧同步与检索可用，对话侧依赖知识库配置 |
| 记忆系统 | 🟡 | 支持 no-memory、Mem0、Memobase、Memos 等配置 |
| 声纹识别 | ✅ | 声纹组、样本、验证流程可用 |
| 声音复刻 | ✅ | 支持异步任务、额度、重试、试听 |
| Manager 控制台 | ✅ | Web UI + API，支持设备、智能体、配置、日志、调试 |
| Web 设备对话模拟 | 🟡 | 文本对话可用，语音与多模态能力逐步补齐 |

详细边界见 [doc/IMPLEMENTATION_STATUS.md](doc/IMPLEMENTATION_STATUS.md)。

## 快速开始

### 环境要求

- Rust 1.75+
- Node.js 18+
- Windows / Linux / macOS

### 编译

```powershell
cd D:\code\xiaozhi-esp32-server-rust
cargo build --release

cd frontend
npm install
npm run build
cd ..
```

### 配置

编辑 `config/config.yaml`，至少配置可用的 LLM、TTS、ASR provider。示例：

```yaml
llm:
  provider: "qwen_72b"
  qwen_72b:
    type: "openai"
    model_name: "Qwen/Qwen2.5-72B-Instruct"
    api_key: "your-api-key"
    base_url: "https://api.siliconflow.cn/v1"

tts:
  provider: "edge"
  edge:
    voice: "zh-CN-XiaoxiaoNeural"

asr:
  provider: "funasr"
  funasr:
    host: "127.0.0.1"
    port: "10096"
```

### 运行

启动设备接入服务：

```powershell
cargo run --release --bin xiaozhi-server -- -c config/config.yaml
```

启动管理控制台：

```powershell
cargo run --release --bin xiaozhi-manager -- -c config/config.yaml
```

浏览器访问 `http://127.0.0.1:8080`。首次使用会进入初始化向导创建管理员账户。

开发前端时可以单独启动 Vite：

```powershell
# 终端 1：Manager API
cargo run --release --bin xiaozhi-manager -- -c config/config.yaml

# 终端 2：前端 dev server
cd frontend
npm run dev
```

## 默认服务地址

| 服务 | 地址 |
|------|------|
| 设备 WebSocket | `ws://host:8989/xiaozhi/v1/` |
| 设备 OTA | `POST http://host:8989/xiaozhi/ota/` |
| 管理控制台 | `http://host:8080` |
| 设备 MCP WebSocket | `ws://host:8989/xiaozhi/mcp/{device_id}` |

## 配置连通性测试

管理控制台的「配置测试」会通过 Manager 与 `xiaozhi-server` 的 Bridge WebSocket 转发到主服务执行。使用前请确认：

1. 先启动 `xiaozhi-server`，再启动 `xiaozhi-manager`。
2. `xiaozhi-server` 已配置 `manager` 段，并与 Manager 保持 WebSocket 长连接。
3. 如果提示「没有已连接的主服务客户端」，说明 Bridge 未连接，通常与 provider 密钥无关。

ASR、LLM、TTS 测试会调用真实 provider，可能产生费用。Silero VAD 测试需要本地存在 `config/models/vad/silero_vad.onnx`。

## 文档

| 文档 | 说明 |
|------|------|
| [doc/IMPLEMENTATION_STATUS.md](doc/IMPLEMENTATION_STATUS.md) | 当前功能状态与已知限制 |
| [doc/DEVICE_INTERACTION_FLOW.md](doc/DEVICE_INTERACTION_FLOW.md) | 设备交互流程、协议、调试链路 |
| [doc/MULTI_ENDPOINT.md](doc/MULTI_ENDPOINT.md) | 多端在线与 EndpointHub 架构 |

## 开发约定

- 前端源码提交到仓库。
- `node_modules/`、`frontend/dist/`、`data/*.db`、运行日志不提交。
- 新增 provider 时优先放在对应 crate 内，通过 factory 注册。
- 新增 API、配置项或调试流程后，同步更新 `README.md` 与 `doc/` 下相关文档。

## License

MIT
