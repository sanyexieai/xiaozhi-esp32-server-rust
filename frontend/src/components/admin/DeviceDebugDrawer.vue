<template>
  <el-drawer
    v-model="visible"
    :title="drawerTitle"
    direction="rtl"
    size="min(920px, 96vw)"
    destroy-on-close
    class="device-debug-drawer"
    @closed="handleClosed"
  >
    <div v-if="device" class="debug-root">
      <div class="device-meta">
        <el-tag :type="liveTagType">{{ liveLabel }}</el-tag>
        <el-tag v-if="caps.hasHardware" type="success" size="small">硬件端点</el-tag>
        <el-tag v-else type="info" size="small">无硬件</el-tag>
        <el-tag v-if="caps.hasWeb" type="success" size="small">Web端点</el-tag>
        <el-tag v-if="caps.mqttOnline" type="success" size="small">MQTT</el-tag>
        <el-tag v-else-if="endpointStatus?.mqtt_broker_online === false" type="danger" size="small">MQTT离线</el-tag>
        <el-tag v-if="caps.hasUdp" type="success" size="small">UDP</el-tag>
        <el-tag v-if="caps.ttsActive" type="warning" size="small">播放中</el-tag>
        <code class="device-mac">{{ deviceId }}</code>
      </div>

      <!-- 通信链路示意 -->
      <section class="pipeline-card">
        <div class="section-title">通信链路</div>
        <div class="pipeline">
          <template v-for="(node, idx) in PIPELINE" :key="node.key">
            <div
              class="pipeline-node"
              :class="{ active: activePhase === node.key, done: phaseIndex(activePhase) > idx }"
            >
              <span class="node-label">{{ node.label }}</span>
            </div>
            <div v-if="idx < PIPELINE.length - 1" class="pipeline-arrow">→</div>
          </template>
        </div>
        <p class="pipeline-hint">
          硬件冷启动：<code>speak_request</code> → <code>speak_ready</code>（固件未实现则超时）；
          热链路：UDP 已 hello 可直接 TTS；播完 <code>auto_listen:false</code> 会发 <code>goodbye</code> 回主页。
        </p>
      </section>

      <el-row :gutter="16" class="main-panels">
        <el-col :xs="24" :md="10">
          <section class="steps-card">
            <div class="section-title">硬件调试（MQTT + UDP）</div>
            <el-space direction="vertical" fill style="width: 100%">
              <el-tooltip content="刷新 Manager → Server 端点与播放状态" placement="left">
                <el-button
                  type="primary"
                  plain
                  :loading="busy"
                  style="width: 100%"
                  @click="handleRefresh"
                >
                  ① 检测连通与端点状态
                </el-button>
              </el-tooltip>

              <el-tooltip :content="caps.wakeHint" placement="left" :disabled="caps.canWake">
                <el-button
                  type="warning"
                  plain
                  :loading="busy"
                  :disabled="!caps.canWake"
                  style="width: 100%"
                  @click="handleWake"
                >
                  ② 远程唤醒（短播报）
                </el-button>
              </el-tooltip>
              <p v-if="caps.canColdWake && !caps.canHotSpeak" class="step-hint warn">
                {{ caps.wakeHint }}
              </p>

              <el-input
                v-model="speakText"
                type="textarea"
                :rows="2"
                placeholder="③ 要播报的文本（仅 TTS，不走 LLM）"
                :disabled="!caps.canSpeak"
              />
              <div class="inline-options">
                <el-checkbox v-model="speakAutoListen" :disabled="!caps.canSpeak">
                  播完继续听（不勾选自回主页）
                </el-checkbox>
              </div>
              <el-tooltip :content="caps.speakHint || '向硬件/Web 下发 TTS'" placement="left">
                <el-button
                  type="success"
                  plain
                  :loading="busy"
                  :disabled="!caps.canSpeak || !speakText.trim()"
                  style="width: 100%"
                  @click="handleSpeak"
                >
                  ③ 播报指定文本
                </el-button>
              </el-tooltip>

              <el-input
                v-model="chatText"
                type="textarea"
                :rows="2"
                placeholder="④ 对话内容（走 LLM + TTS）"
                :disabled="!caps.canChat"
              />
              <div class="inline-options">
                <el-checkbox v-model="chatAutoListen" :disabled="!caps.canChat">
                  播完继续听
                </el-checkbox>
              </div>
              <el-tooltip
                :content="caps.speakHint || '注入消息 → LLM → TTS'"
                placement="left"
              >
                <el-button
                  type="primary"
                  :loading="busy"
                  :disabled="!caps.canChat || !chatText.trim()"
                  style="width: 100%"
                  @click="handleChatApi"
                >
                  ④ API 对话（注入 → LLM → 播报）
                </el-button>
              </el-tooltip>

              <el-divider />

              <div class="section-title">播放控制</div>
              <el-tooltip :content="caps.abortHint" placement="left">
                <el-button
                  type="danger"
                  plain
                  :loading="busy"
                  :disabled="!caps.canAbortHw"
                  style="width: 100%"
                  @click="handleAbort"
                >
                  ⑤ 打断播放（abort）
                </el-button>
              </el-tooltip>
              <el-tooltip :content="caps.goodbyeHint" placement="left">
                <el-button
                  type="info"
                  plain
                  :loading="busy"
                  :disabled="!caps.canGoodbyeHw"
                  style="width: 100%"
                  @click="handleGoodbye"
                >
                  ⑥ 返回主页（goodbye）
                </el-button>
              </el-tooltip>
            </el-space>

            <el-divider />

            <div class="section-title">WebSocket 实时对话</div>
            <div class="ws-toolbar">
              <el-tag :type="wsTagType" size="small">{{ wsLabel }}</el-tag>
              <el-button
                v-if="!isConnected"
                size="small"
                type="primary"
                :loading="wsConnecting"
                @click="handleWsConnect"
              >
                连接并 hello
              </el-button>
              <el-button v-else size="small" @click="handleWsDisconnect">断开</el-button>
            </div>
            <el-input
              v-model="wsDraft"
              type="textarea"
              :rows="2"
              placeholder="listen.detect 文本（模拟 ASR 结果）"
              :disabled="!caps.canWsSend"
            />
            <el-button
              size="small"
              type="primary"
              :disabled="!caps.canWsSend || !wsDraft.trim()"
              style="margin-top: 8px; width: 100%"
              @click="handleWsSend"
            >
              发送 listen.detect
            </el-button>
            <div class="ws-actions">
              <el-tooltip :content="caps.wsAbortHint" placement="top">
                <el-button
                  size="small"
                  type="danger"
                  plain
                  :disabled="!caps.canWsAbort"
                  @click="handleWsAbort"
                >
                  WS 打断
                </el-button>
              </el-tooltip>
              <el-tooltip :content="caps.wsGoodbyeHint" placement="top">
                <el-button
                  size="small"
                  type="info"
                  plain
                  :disabled="!caps.canWsGoodbye"
                  @click="handleWsGoodbye"
                >
                  WS 回主页
                </el-button>
              </el-tooltip>
            </div>
          </section>
        </el-col>

        <el-col :xs="24" :md="14">
          <section class="log-card">
            <el-tabs v-model="logTab" class="log-tabs">
              <el-tab-pane label="信令记录" name="signals">
                <div class="log-header">
                  <span class="log-hint">设备 ↔ Server 全量信令（MQTT / WS / UDP，每秒刷新）</span>
                  <el-button size="small" text :disabled="!deviceDbId" @click="handleClearSignals">
                    清空
                  </el-button>
                </div>
                <div v-if="signalLog.length === 0" class="log-empty">
                  打开调试后自动采集；设备唤醒、对话、播报时这里会实时出现信令
                </div>
                <div v-else ref="signalListRef" class="timeline signal-timeline">
                  <div
                    v-for="item in signalLog"
                    :key="item.id"
                    class="timeline-item signal-item"
                    :class="signalItemClass(item)"
                  >
                    <div class="timeline-head">
                      <el-tag size="small" :type="item.direction === 'in' ? 'warning' : item.direction === 'internal' ? 'danger' : 'primary'">
                        {{ item.direction === 'in' ? '← 设备' : item.direction === 'internal' ? '⚙ 后台' : '→ 设备' }}
                      </el-tag>
                      <el-tag size="small" type="info">{{ channelLabel(item.channel) }}</el-tag>
                      <el-tag size="small" effect="plain">{{ item.msg_type }}</el-tag>
                      <span class="timeline-ts">{{ formatSignalTs(item.ts_ms) }}</span>
                    </div>
                    <div class="timeline-summary">{{ item.summary }}</div>
                    <details v-if="item.payload" class="timeline-detail">
                      <summary>payload</summary>
                      <pre>{{ formatJson(item.payload) }}</pre>
                    </details>
                  </div>
                </div>
              </el-tab-pane>

              <el-tab-pane label="操作流水" name="flow">
                <div class="log-header">
                  <span class="section-title">管理端操作</span>
                  <el-button size="small" text @click="clearFlow">清空</el-button>
                </div>
                <div v-if="mergedTimeline.length === 0" class="log-empty">执行上方步骤后，这里会显示请求与响应</div>
                <div v-else class="timeline">
                  <div
                    v-for="item in mergedTimeline"
                    :key="item.id"
                    class="timeline-item"
                    :class="{ fail: !item.ok, ws: item.step === 'chat_ws' }"
                  >
                    <div class="timeline-head">
                      <el-tag size="small" :type="item.ok ? 'success' : 'danger'">{{ stepLabel(item.step) }}</el-tag>
                      <span class="timeline-dir">{{ item.direction === 'in' ? '← 入站' : '→ 出站' }}</span>
                      <span class="timeline-ts">{{ formatTs(item.ts) }}</span>
                      <span v-if="item.durationMs" class="timeline-ms">{{ item.durationMs }}ms</span>
                    </div>
                    <div class="timeline-summary">{{ item.summary }}</div>
                    <details v-if="item.request || item.response" class="timeline-detail">
                      <summary>详情</summary>
                      <pre>{{ formatJson({ request: item.request, response: item.response }) }}</pre>
                    </details>
                  </div>
                </div>

                <div v-if="wsTranscript.length" class="ws-transcript">
                  <div class="section-title">WS 会话记录</div>
                  <div v-for="row in wsTranscript" :key="row.id" class="ws-line" :class="row.role">
                    <span class="ws-role">{{ row.title || row.role }}</span>
                    <span>{{ row.content }}</span>
                  </div>
                </div>
              </el-tab-pane>
            </el-tabs>
          </section>
        </el-col>
      </el-row>
    </div>
  </el-drawer>
</template>

<script setup>
import { computed, nextTick, ref, watch } from 'vue'
import { ElMessage } from 'element-plus'
import api from '@/utils/api'
import { deviceOnlineLabel, deviceOnlineTagType } from '@/utils/deviceStatus'
import { useDeviceDebug, deriveDebugCapabilities } from '@/composables/useDeviceDebug'
import { useDeviceChatSimulator } from '@/composables/useDeviceChatSimulator'

const props = defineProps({
  modelValue: { type: Boolean, default: false },
  device: { type: Object, default: null },
  live: { type: Object, default: null },
  /** admin | user — 决定 API 与 WS 配置路径 */
  scope: { type: String, default: 'admin' }
})

const emit = defineEmits(['update:modelValue'])

const debugScope = computed(() => (props.scope === 'user' ? 'user' : 'admin'))
const chatConfigPath = computed(() =>
  debugScope.value === 'user' ? '/user/device-chat/config' : '/admin/device-simulator/config'
)

const visible = computed({
  get: () => props.modelValue,
  set: (v) => emit('update:modelValue', v)
})

const speakText = ref('你好，我是小智')
const chatText = ref('今天天气怎么样？')
const speakAutoListen = ref(false)
const chatAutoListen = ref(true)
const wsDraft = ref('')
const wsConnecting = ref(false)
const configLoaded = ref(false)
const logTab = ref('signals')
const signalListRef = ref(null)

const {
  PIPELINE,
  flowLog,
  signalLog,
  endpointStatus,
  busy,
  activePhase,
  clearFlow,
  refreshStatus,
  wakeDevice,
  playText,
  chatViaApi,
  abortPlayback,
  returnToHome,
  logWsEvent,
  logWsSend,
  startSignalPolling,
  stopSignalPolling,
  clearSignals
} = useDeviceDebug({ scope: props.scope === 'user' ? 'user' : 'admin' })

const {
  config: simConfig,
  connectionState,
  sessionId,
  transcript: wsTranscript,
  isConnected,
  ttsPlaying,
  loadConfig,
  connect,
  disconnect,
  sendText,
  sendAbort,
  sendGoodbye
} = useDeviceChatSimulator({
  quietConnect: true,
  configPath: chatConfigPath.value
})

const caps = computed(() =>
  deriveDebugCapabilities(endpointStatus.value, {
    connected: isConnected.value,
    ttsPlaying: ttsPlaying.value
  })
)

const deviceId = computed(() => props.device?.device_name || '')
const deviceDbId = computed(() => props.device?.id)
const drawerTitle = computed(() => {
  const name = props.device?.nick_name || props.device?.device_name || '设备'
  return `设备调试 · ${name}`
})

const liveLabel = computed(() => deviceOnlineLabel(props.device, props.live))
const liveTagType = computed(() => deviceOnlineTagType(props.device, props.live))

const wsLabel = computed(() => {
  if (connectionState.value === 'connecting') return 'WS 连接中…'
  if (isConnected.value) {
    const play = ttsPlaying.value ? ' · 播放中' : ''
    return sessionId.value
      ? `WS 已连接 · ${sessionId.value.slice(0, 8)}…${play}`
      : `WS 已连接${play}`
  }
  if (connectionState.value === 'error') return 'WS 失败'
  return 'WS 未连接'
})

const wsTagType = computed(() => {
  if (isConnected.value) return 'success'
  if (connectionState.value === 'connecting') return 'warning'
  if (connectionState.value === 'error') return 'danger'
  return 'info'
})

const mergedTimeline = computed(() => {
  return [...flowLog.value].sort((a, b) => a.id - b.id)
})

function phaseIndex(phase) {
  return PIPELINE.value.findIndex((p) => p.key === phase)
}

function stepLabel(step) {
  const map = {
    status: '状态检测',
    wake: '唤醒',
    speak: '播报',
    chat_api: 'API对话',
    abort: '打断',
    goodbye: '回主页',
    chat_ws: 'WS对话'
  }
  return map[step] || step
}

function formatTs(iso) {
  try {
    return new Date(iso).toLocaleTimeString()
  } catch {
    return iso
  }
}

function formatJson(obj) {
  try {
    return JSON.stringify(obj, null, 2)
  } catch {
    return String(obj)
  }
}

function formatSignalTs(tsMs) {
  if (!tsMs) return ''
  try {
    return new Date(tsMs).toLocaleTimeString()
  } catch {
    return String(tsMs)
  }
}

function channelLabel(channel) {
  const map = { mqtt: 'MQTT', ws: 'WebSocket', udp: 'UDP', llm: 'LLM' }
  return map[channel] || channel || '?'
}

function signalItemClass(item) {
  return {
    'signal-in': item.direction === 'in',
    'signal-out': item.direction === 'out',
    'signal-internal': item.direction === 'internal',
    'signal-audio': item.msg_type === 'audio',
    'signal-mcp-call': item.msg_type === 'mcp_tool_call',
    'signal-mcp-result': item.msg_type === 'mcp_tool_result'
  }
}

async function handleClearSignals() {
  if (!deviceDbId.value) return
  try {
    await clearSignals(deviceDbId.value)
  } catch (e) {
    ElMessage.error(e?.message || '清空信令失败')
  }
}

async function ensureConfig() {
  if (configLoaded.value && simConfig.value) return
  await loadConfig(api)
  configLoaded.value = true
}

async function refreshAfterAction() {
  if (!deviceDbId.value) return
  try {
    await refreshStatus(deviceDbId.value)
  } catch {
    /* 刷新失败不阻断主流程 */
  }
}

watch(
  () => props.modelValue,
  async (open) => {
    if (!open) {
      stopSignalPolling()
      return
    }
    if (!deviceDbId.value) return
    clearFlow()
    logTab.value = 'signals'
    wsDraft.value = ''
    try {
      await ensureConfig()
      await refreshStatus(deviceDbId.value)
      await clearSignals(deviceDbId.value)
      startSignalPolling(deviceDbId.value)
    } catch (e) {
      ElMessage.warning(e?.message || '加载调试环境失败')
    }
  }
)

watch(signalLog, async () => {
  if (logTab.value !== 'signals') return
  await nextTick()
  const el = signalListRef.value
  if (el) {
    el.scrollTop = el.scrollHeight
  }
}, { deep: true })

async function handleRefresh() {
  if (!deviceDbId.value) return
  try {
    await refreshStatus(deviceDbId.value)
    ElMessage.success('状态已刷新')
  } catch {
    ElMessage.error('状态检测失败')
  }
}

async function handleWake() {
  if (!deviceDbId.value) return
  try {
    await wakeDevice(deviceDbId.value)
    ElMessage.success('唤醒指令已发送')
    await refreshAfterAction()
  } catch {
    ElMessage.error('唤醒失败')
  }
}

async function handleSpeak() {
  if (!deviceDbId.value) return
  try {
    await playText(deviceDbId.value, speakText.value, {
      autoListen: speakAutoListen.value
    })
    ElMessage.success('播报已发送')
    await refreshAfterAction()
  } catch (e) {
    ElMessage.error(e?.message || '播报失败')
  }
}

async function handleChatApi() {
  if (!deviceId.value) return
  try {
    await chatViaApi(deviceId.value, chatText.value, {
      skipLlm: false,
      autoListen: chatAutoListen.value
    })
    ElMessage.success('对话已提交')
    await refreshAfterAction()
  } catch (e) {
    ElMessage.error(e?.message || '对话失败')
  }
}

async function handleAbort() {
  if (!deviceDbId.value) return
  try {
    await abortPlayback(deviceDbId.value)
    ElMessage.success('打断指令已发送')
    await refreshAfterAction()
  } catch {
    ElMessage.error('打断失败')
  }
}

async function handleGoodbye() {
  if (!deviceDbId.value) return
  try {
    await returnToHome(deviceDbId.value)
    ElMessage.success('goodbye 已发送')
    await refreshAfterAction()
  } catch {
    ElMessage.error('回主页失败')
  }
}

async function handleWsConnect() {
  if (!deviceId.value) return
  wsConnecting.value = true
  try {
    await ensureConfig()
    logWsSend('WebSocket 连接 + hello', { device_id: deviceId.value })
    await connect({ deviceId: deviceId.value, protocolVersion: 1 })
    logWsEvent('hello 握手完成', { session_id: sessionId.value })
    ElMessage.success('WebSocket 已连接')
  } catch (e) {
    logWsEvent('WebSocket 连接失败', { error: e?.message }, false)
    ElMessage.error(e?.message || '连接失败')
  } finally {
    wsConnecting.value = false
  }
}

function handleWsDisconnect() {
  disconnect()
  logWsEvent('WebSocket 已断开', {})
  ElMessage.info('已断开 WebSocket')
}

async function handleWsSend() {
  const text = wsDraft.value.trim()
  if (!text) return
  try {
    logWsSend('listen.detect', { type: 'listen', state: 'detect', text })
    await sendText(text)
    wsDraft.value = ''
  } catch (e) {
    ElMessage.error(e?.message || '发送失败')
  }
}

function handleWsAbort() {
  logWsSend('abort', { type: 'abort' })
  if (sendAbort()) {
    ElMessage.success('WS abort 已发送')
  } else {
    ElMessage.error('发送失败')
  }
}

function handleWsGoodbye() {
  logWsSend('goodbye', { type: 'goodbye' })
  if (sendGoodbye()) {
    logWsEvent('WebSocket goodbye 已发送', {})
    ElMessage.success('WS goodbye 已发送')
  } else {
    ElMessage.error('发送失败')
  }
}

watch(wsTranscript, (rows) => {
  const last = rows[rows.length - 1]
  if (!last || last._logged) return
  last._logged = true
  logWsEvent(`WS ${last.title || last.kind || 'message'}`, {
    role: last.role,
    content: last.content,
    type: last.rawType
  })
}, { deep: true })

function handleClosed() {
  stopSignalPolling()
  disconnect()
  clearFlow()
  configLoaded.value = false
}
</script>

<style scoped>
.debug-root {
  display: flex;
  flex-direction: column;
  gap: 16px;
  padding-bottom: 24px;
}

.device-meta {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
}

.device-mac {
  font-size: 12px;
  color: #64748b;
  background: #f1f5f9;
  padding: 2px 8px;
  border-radius: 4px;
}

.section-title {
  font-weight: 600;
  font-size: 14px;
  margin-bottom: 10px;
  color: #1e293b;
}

.pipeline-card,
.steps-card,
.log-card {
  background: #f8fafc;
  border: 1px solid #e2e8f0;
  border-radius: 10px;
  padding: 14px;
}

.pipeline {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
}

.pipeline-node {
  padding: 8px 12px;
  border-radius: 8px;
  background: #fff;
  border: 1px solid #cbd5e1;
  font-size: 12px;
  transition: all 0.2s;
}

.pipeline-node.active {
  border-color: #3b82f6;
  background: #eff6ff;
  box-shadow: 0 0 0 2px rgba(59, 130, 246, 0.2);
}

.pipeline-node.done {
  border-color: #22c55e;
  background: #f0fdf4;
}

.pipeline-arrow {
  color: #94a3b8;
  font-weight: 700;
}

.pipeline-hint {
  margin: 10px 0 0;
  font-size: 12px;
  color: #64748b;
  line-height: 1.5;
}

.pipeline-hint code {
  font-size: 11px;
}

.step-hint {
  margin: -4px 0 4px;
  font-size: 12px;
  line-height: 1.4;
}

.step-hint.warn {
  color: #b45309;
}

.inline-options {
  width: 100%;
  font-size: 12px;
}

.log-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 8px;
}

.log-hint {
  font-size: 12px;
  color: #64748b;
}

.log-tabs :deep(.el-tabs__header) {
  margin-bottom: 8px;
}

.signal-timeline {
  max-height: 480px;
}

.signal-item.signal-in {
  border-left-color: #f59e0b;
}

.signal-item.signal-out {
  border-left-color: #3b82f6;
}

.signal-item.signal-audio {
  border-left-color: #94a3b8;
  opacity: 0.92;
}

.signal-item.signal-internal,
.signal-item.signal-mcp-call,
.signal-item.signal-mcp-result {
  border-left-color: #ef4444;
  background: #fef2f2;
}

.log-empty {
  color: #94a3b8;
  font-size: 13px;
  padding: 24px 0;
  text-align: center;
}

.timeline {
  max-height: 420px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.timeline-item {
  background: #fff;
  border: 1px solid #e2e8f0;
  border-left: 3px solid #22c55e;
  border-radius: 8px;
  padding: 10px 12px;
}

.timeline-item.fail {
  border-left-color: #ef4444;
}

.timeline-item.ws {
  border-left-color: #3b82f6;
}

.timeline-head {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  margin-bottom: 6px;
}

.timeline-dir {
  font-size: 12px;
  color: #64748b;
}

.timeline-ts,
.timeline-ms {
  font-size: 11px;
  color: #94a3b8;
  margin-left: auto;
}

.timeline-summary {
  font-size: 13px;
  color: #334155;
  line-height: 1.45;
}

.timeline-detail {
  margin-top: 8px;
  font-size: 12px;
}

.timeline-detail pre {
  margin: 6px 0 0;
  padding: 8px;
  background: #0f172a;
  color: #e2e8f0;
  border-radius: 6px;
  overflow: auto;
  max-height: 180px;
  font-size: 11px;
}

.ws-toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
}

.ws-actions {
  display: flex;
  gap: 8px;
  margin-top: 8px;
}

.ws-transcript {
  margin-top: 16px;
  border-top: 1px dashed #cbd5e1;
  padding-top: 12px;
}

.ws-line {
  font-size: 13px;
  margin-bottom: 6px;
  line-height: 1.4;
}

.ws-line.user { color: #0369a1; }
.ws-line.assistant { color: #15803d; }
.ws-line.system { color: #64748b; }

.ws-role {
  font-weight: 600;
  margin-right: 6px;
}
</style>
