# 多端在线架构（Logical Device + Endpoint）

## 背景

同一账号下可能同时存在：

- ESP32 硬件（MQTT 信令 + UDP 音频）
- 管理台 Web 模拟器（WebSocket JSON + Opus 音频）
- 未来：手机 App、OpenAPI 客户端

旧模型 **一个 `device_id` 仅允许一条连接**，Web 模拟器会与真实设备互相踢会话，导致 TTS 走错通道。

## 目标模型

```
Logical Device（DB devices.device_id，绑定 user/agent）
    └── ChatManager（共享 dialogue / LLM / 历史入库）
            └── EndpointHub
                    ├── hardware  (MQTT+UDP)   音频主端
                    ├── web-{n}   (WebSocket)  文本同步 + 可选音频
                    └── tool-*    (MCP WS)     工具链
```

### 能力矩阵

| 能力 | 说明 |
|------|------|
| 多端同时在线 | 同一 `device_id` 可注册多个 endpoint，互不踢线 |
| 消息同步 | STT / LLM / TTS 信令 **广播** 到所有 endpoint |
| 音频路由 | Opus 音频按 `TtsAudioRoute` 投递（默认 HardwareFirst） |
| 跨端推送 | API 指定 `target=web|hardware|all` 注入文本或 TTS |
| 默认模拟设备 | 每用户 `web-sim-{user_id}`，无硬件也可调试 |

## 连接流程

1. **硬件 hello（MQTT）**  
   `ensure_chat_manager` → 创建/复用 ChatManager → `register_endpoint(hardware)`

2. **Web 模拟器连接（WS）**  
   不再 `remove_and_shutdown`；复用已有 ChatManager → `register_endpoint(web-{gen})`

3. **断开**  
   仅 `unregister_endpoint`；当 Hub 为空时才 `remove_and_shutdown`

## 默认模拟设备

- ID：`web-sim-{user_id}`（管理员调试时用 admin 的 user_id）
- 首次打开模拟器配置时自动创建并激活
- 可绑定智能体，与真实设备一样走 uconfig

## API

| 方法 | 路径 | 说明 |
|------|------|------|
| POST | `/api/user/devices/inject-message` | 向在线端注入；body 可选 `target`（web/hardware/all） |
| POST | `/api/admin/devices/{id}/speak` | 指定 endpoint 播报（`target` + `text`） |
| POST | `/api/user/devices/{id}/speak` | 用户侧指定播报 |
| GET | `/api/admin/devices/{id}/endpoints` | 列出当前在线 endpoint |
| GET | `/api/user/devices/{id}/endpoints` | 用户侧 endpoint 查询 |
| POST | `/api/admin/devices/live-status` | 批量查询会话端点（body: `device_ids`） |
| POST | `/api/user/devices/live-status` | 用户侧批量查询 |

`target` 取值：`hardware_first`（默认）、`hardware`、`web`、`all`。

## 分阶段落地

| 阶段 | 内容 | 状态 |
|------|------|------|
| P1 | EndpointHub + WS/MQTT 附着 + 默认模拟设备 | ✅ 已完成 |
| P2 | inject/speak 指定 target + endpoints 查询 API | ✅ 已完成 |
| P3 | 前端多端状态条 + 跨端推送 UI | ✅ 已完成 |
| P4 | 同账号多设备并行 + 设备列表多端状态展示 | ✅ 已完成 |

## 与 `sim:` 前缀的关系

早期用 `sim:{device_id}` 隔离会话。P1 起改为 **Endpoint 模型**；`sim:` 仍保留解析兼容，但模拟器默认使用独立 `web-sim-*` 设备或同设备多端附着。
