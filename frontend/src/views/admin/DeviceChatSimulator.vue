<template>
  <div class="device-chat-simulator">
    <div class="page-toolbar">
      <div class="toolbar-copy">
        <p class="eyebrow">Debug Console</p>
        <h2>设备对话模拟</h2>
        <p class="subtitle">通过 WebSocket 代理模拟 ESP32 设备对话流程，文本模式可用；语音 / 多模态 / MCP Skill 已预留。</p>
      </div>
      <div class="toolbar-actions">
        <el-button @click="reloadAll" :loading="loadingDevices || loadingConfig">刷新</el-button>
        <el-tag :type="connectionTagType">{{ connectionLabel }}</el-tag>
      </div>
    </div>

    <el-row :gutter="16" class="layout-row">
      <el-col :xs="24" :lg="7">
        <el-card shadow="never" class="panel-card">
          <template #header>
            <span>连接配置</span>
          </template>
          <el-form label-position="top" class="connect-form">
            <el-form-item label="模拟设备">
              <el-select
                v-model="selectedDeviceId"
                filterable
                allow-create
                default-first-option
                placeholder="选择或输入 Device-Id"
                style="width: 100%"
                @change="handleDeviceChange"
              >
                <el-option
                  v-for="device in deviceOptions"
                  :key="device.id || device.device_name"
                  :label="getDeviceOptionLabel(device)"
                  :value="device.device_name || ''"
                >
                  <div class="device-option">
                    <span>{{ getDeviceNickName(device) }}</span>
                    <el-tag size="small" :type="isDeviceOnline(device) ? 'success' : 'info'">
                      {{ isDeviceOnline(device) ? '在线' : '离线' }}
                    </el-tag>
                  </div>
                </el-option>
              </el-select>
              <div class="field-help">
                选择真实设备可与硬件<strong>同时在线</strong>（共享对话）；默认「Web 模拟设备」无需硬件即可调试。
              </div>
            </el-form-item>

            <div v-if="selectedDeviceId" class="endpoint-status">
              <div class="endpoint-status-title">多端在线状态</div>
              <div class="endpoint-tags">
                <el-tag size="small" :type="endpointStatus?.has_hardware ? 'success' : 'info'">
                  硬件 {{ endpointStatus?.has_hardware ? '在线' : '离线' }}
                </el-tag>
                <el-tag size="small" :type="endpointStatus?.has_web ? 'success' : 'info'">
                  Web {{ endpointStatus?.has_web ? '在线' : '离线' }}
                </el-tag>
                <el-tag size="small" type="info">
                  端点 {{ endpointStatus?.endpoint_count ?? 0 }}
                </el-tag>
              </div>
              <div v-if="endpointStatus?.endpoints?.length" class="endpoint-list">
                <span
                  v-for="ep in endpointStatus.endpoints"
                  :key="ep.id"
                  class="endpoint-item"
                >
                  {{ ep.kind }} · {{ ep.id }}
                </span>
              </div>
            </div>

            <el-form-item label="协议版本">
              <el-input-number v-model="protocolVersion" :min="1" :max="3" style="width: 100%" />
            </el-form-item>

            <el-form-item label="WebSocket 地址（可选覆盖）">
              <el-input
                v-model="wsUrlOverride"
                placeholder="留空则使用 OTA 配置中的地址"
                clearable
              />
              <div v-if="simulatorConfig?.local_ws_url" class="field-help">
                本机调试推荐：{{ simulatorConfig.local_ws_url }}
                <el-tag size="small" type="success">默认</el-tag>
              </div>
              <div v-else-if="simulatorConfig?.ws_url" class="field-help">
                OTA 默认：{{ simulatorConfig.ws_url }}
                <el-tag size="small" type="info">{{ simulatorConfig.env }}</el-tag>
              </div>
            </el-form-item>

            <div class="connect-actions">
              <el-button
                v-if="!isConnected"
                type="primary"
                :loading="connecting"
                :disabled="!selectedDeviceId"
                @click="handleConnect"
              >
                连接并握手
              </el-button>
              <template v-else>
                <el-button type="danger" plain @click="handleDisconnect">断开</el-button>
                <el-button @click="handleGoodbye">结束会话</el-button>
              </template>
            </div>

            <el-alert
              v-if="lastError"
              type="error"
              :closable="false"
              show-icon
              class="error-alert"
              :title="lastError"
            />

            <el-divider />

            <div class="push-panel">
              <div class="push-panel-title">跨端推送</div>
              <el-input
                v-model="pushText"
                type="textarea"
                :rows="2"
                placeholder="向设备注入消息或直接播报（需设备在线）"
                maxlength="500"
              />
              <el-form-item label="音频路由" class="push-target">
                <el-select v-model="pushTarget" style="width: 100%">
                  <el-option label="优先硬件（默认）" value="hardware_first" />
                  <el-option label="仅硬件" value="hardware" />
                  <el-option label="仅 Web" value="web" />
                  <el-option label="全部端点" value="all" />
                </el-select>
              </el-form-item>
              <div class="push-actions">
                <el-checkbox v-model="pushSkipLlm">跳过 LLM（直接 TTS）</el-checkbox>
                <el-checkbox v-model="pushAutoListen">播报后自动监听</el-checkbox>
              </div>
              <div class="push-buttons">
                <el-button
                  size="small"
                  :disabled="!selectedDeviceId || !pushText.trim() || pushing"
                  :loading="pushing"
                  @click="handleInjectMessage"
                >
                  注入消息
                </el-button>
                <el-button
                  size="small"
                  type="primary"
                  :disabled="!selectedDeviceDbId || !pushText.trim() || pushing"
                  :loading="pushing"
                  @click="handleSpeak"
                >
                  指定播报
                </el-button>
              </div>
            </div>

            <el-divider />

            <div class="feature-badges">
              <el-tag size="small" type="success">文本对话</el-tag>
              <el-tag size="small" type="success">TTS 播放</el-tag>
              <el-tag size="small" type="info">语音上行（预留）</el-tag>
              <el-tag size="small" type="info">多模态（预留）</el-tag>
              <el-tag size="small" type="warning">后台流水</el-tag>
              <el-tag size="small" type="info">MCP Skill（预留）</el-tag>
            </div>

            <el-form-item label="TTS 播放">
              <div class="switch-field">
                <span class="switch-desc">解码 Opus 下行音频并通过扬声器播放</span>
                <el-switch v-model="ttsEnabled" @change="setTtsEnabled" />
              </div>
              <div v-if="binaryFrameCount > 0" class="field-help">
                已收到 {{ binaryFrameCount }} 帧音频
                <span v-if="ttsPlaying"> · 播放中</span>
              </div>
              <el-alert
                v-if="ttsPlayerError"
                type="warning"
                :closable="false"
                show-icon
                class="error-alert"
                :title="ttsPlayerError"
              />
            </el-form-item>
          </el-form>
        </el-card>
      </el-col>

      <el-col :xs="24" :lg="17">
        <el-card shadow="never" class="panel-card chat-panel">
          <template #header>
            <div class="chat-header">
              <span>对话面板</span>
              <div class="chat-header-actions">
                <el-button size="small" :disabled="!isConnected" @click="handleAbort">打断</el-button>
                <el-button size="small" @click="clearTranscript">清空记录</el-button>
              </div>
            </div>
          </template>

          <el-tabs v-model="activeTab" class="simulator-tabs">
            <el-tab-pane label="文本对话" name="chat">
              <div ref="transcriptRef" class="transcript" v-loading="connecting">
                <div v-if="transcript.length === 0" class="empty-transcript">
                  <el-empty description="连接设备后发送消息，将展示 STT / LLM / TTS 等协议事件" />
                </div>
                <div
                  v-for="item in transcript"
                  :key="item.id"
                  class="transcript-item"
                  :class="[`role-${item.role}`, `kind-${item.kind}`]"
                >
                  <div class="item-meta">
                    <span class="item-role">{{ roleLabel(item) }}</span>
                    <span class="item-time">{{ formatTime(item.ts) }}</span>
                  </div>
                  <div v-if="item.title" class="item-title">{{ item.title }}</div>
                  <div class="item-content">{{ item.content }}</div>
                </div>
              </div>

              <div class="composer">
                <el-input
                  v-model="draft"
                  type="textarea"
                  :rows="3"
                  placeholder="输入用户话术，将以 listen.detect 发送（模拟 ASR 结果）"
                  maxlength="2000"
                  show-word-limit
                  :disabled="!isConnected || !sessionId"
                  @keydown.ctrl.enter.prevent="handleSend"
                />
                <div class="composer-actions">
                  <span class="composer-hint">
                    {{ sessionId ? 'Ctrl + Enter 发送' : '等待 hello 握手完成…' }}
                  </span>
                  <el-button
                    type="primary"
                    :disabled="!isConnected || !sessionId || !draft.trim()"
                    @click="handleSend"
                  >
                    发送
                  </el-button>
                </div>
              </div>
            </el-tab-pane>

            <el-tab-pane label="语音（预留）" name="voice" lazy>
              <FeaturePlaceholder
                title="语音对话"
                description="模拟麦克风采集、Opus 上行与 TTS 音频下行播放，完整复现设备语音轮次。"
                :icon="Microphone"
                :planned="[
                  'listen.start / listen.stop 控制采集',
                  'Binary Opus 帧上行（Protocol-Version 1/2/3）',
                  'TTS 二进制帧解码与 Web Audio 播放',
                  'VAD / 打断模式联调'
                ]"
              />
            </el-tab-pane>

            <el-tab-pane label="多模态（预留）" name="multimodal" lazy>
              <FeaturePlaceholder
                title="多模态识图"
                description="模拟设备拍照识图流程，对接 Vision 配置与 /xiaozhi/api/vision 接口。"
                :icon="Picture"
                :planned="[
                  '图片上传与预览',
                  'Vision Provider 选择与参数',
                  '识图结果注入对话上下文',
                  '与 listen.detect 组合测试'
                ]"
              />
            </el-tab-pane>

            <el-tab-pane label="后台流水" name="backend">
              <div class="backend-log-header">
                <span class="section-title">服务端处理流水（含 MCP 工具调用）</span>
                <el-button
                  size="small"
                  text
                  :disabled="!canPollBackendSignals"
                  @click="handleClearBackendLog"
                >
                  清空
                </el-button>
              </div>
              <el-alert
                v-if="isConnected && !canPollBackendSignals"
                type="warning"
                :closable="false"
                show-icon
                class="error-alert"
                title="无法拉取后台流水：请先选择或填写设备 ID"
              />
              <el-alert
                v-else-if="isConnected && backendPollingActive"
                type="info"
                :closable="false"
                show-icon
                class="error-alert"
                title="已连接并轮询服务端流水；MCP 调用会同步显示在「文本对话」与下方列表"
              />
              <div v-if="!canPollBackendSignals" class="empty-transcript">
                <el-empty description="请选择设备并连接后开始记录后台流水" />
              </div>
              <div v-else-if="signalLog.length === 0" class="empty-transcript">
                <el-empty description="发送会触发 MCP 的对话（如查天气）后，这里会显示 mcp_tool_call / mcp_tool_result" />
              </div>
              <div v-else ref="backendLogRef" class="backend-log">
                <div
                  v-for="item in signalLog"
                  :key="item.id"
                  class="backend-item"
                  :class="backendItemClass(item)"
                >
                  <div class="backend-head">
                    <el-tag size="small" :type="backendDirectionTag(item)">
                      {{ backendDirectionLabel(item) }}
                    </el-tag>
                    <el-tag size="small" type="info">{{ backendChannelLabel(item.channel) }}</el-tag>
                    <el-tag size="small" effect="plain">{{ item.msg_type }}</el-tag>
                    <span class="backend-ts">{{ formatSignalTs(item.ts_ms) }}</span>
                  </div>
                  <div class="backend-summary">{{ item.summary }}</div>
                  <details v-if="item.payload" class="backend-detail">
                    <summary>详情</summary>
                    <pre>{{ formatJson(item.payload) }}</pre>
                  </details>
                </div>
              </div>
            </el-tab-pane>

            <el-tab-pane label="MCP Skill（预留）" name="mcp" lazy>
              <FeaturePlaceholder
                title="MCP Skill 工具链"
                description="模拟会话内 MCP 消息与 Skill 调用，联调全局 MCP / 市场导入服务 / 设备 MCP。"
                :icon="Connection"
                :planned="[
                  '解析服务端 mcp 类型 JSON 消息',
                  '工具列表与手动 invoke',
                  'Skill 编排与结果回显',
                  '与 AgentRuntimeDiagnostics 能力对齐'
                ]"
              />
            </el-tab-pane>
          </el-tabs>
        </el-card>
      </el-col>
    </el-row>
  </div>
</template>

<script setup>
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { ElMessage } from 'element-plus'
import { Connection, Microphone, Picture } from '@element-plus/icons-vue'
import api from '@/utils/api'
import { isDeviceOnline } from '@/utils/deviceStatus'
import { useDeviceChatSimulator } from '@/composables/useDeviceChatSimulator'
import { useDeviceDebug } from '@/composables/useDeviceDebug'
import FeaturePlaceholder from '@/components/admin/device-simulator/FeaturePlaceholder.vue'

const {
  config: simulatorConfig,
  connectionState,
  sessionId,
  transcript,
  lastError,
  binaryFrameCount,
  ttsEnabled,
  ttsPlaying,
  ttsPlayerError,
  isConnected,
  loadConfig,
  connect,
  disconnect,
  sendText,
  sendAbort,
  sendGoodbye,
  clearTranscript,
  setTtsEnabled,
  notifySystem
} = useDeviceChatSimulator()

const {
  signalLog,
  startSignalPolling,
  stopSignalPolling,
  clearSignals
} = useDeviceDebug({ scope: 'admin' })

const devices = ref([])
const loadingDevices = ref(false)
const loadingConfig = ref(false)
const connecting = ref(false)
const selectedDeviceId = ref('')
const protocolVersion = ref(1)
const wsUrlOverride = ref('')
const activeTab = ref('chat')
const draft = ref('')
const transcriptRef = ref(null)
const backendLogRef = ref(null)
const endpointStatus = ref(null)
const pushText = ref('')
const pushTarget = ref('hardware_first')
const pushSkipLlm = ref(false)
const pushAutoListen = ref(false)
const pushing = ref(false)
let endpointPollTimer = null
const backendPollingActive = ref(false)
const mirroredBackendSignalIds = new Set()
let pollingLogicalDeviceId = ''

const selectedDeviceDbId = computed(() => {
  if (
    simulatorConfig.value?.default_sim_device_id &&
    selectedDeviceId.value === simulatorConfig.value.default_sim_device_id
  ) {
    return simulatorConfig.value.default_sim_db_id || null
  }
  const device = devices.value.find(
    (d) => (d.device_name || '') === selectedDeviceId.value
  )
  return device?.id || null
})

/** 后台流水轮询目标：优先 DB id，否则用 device_id 直连 server */
const signalsPollTarget = computed(() => {
  if (selectedDeviceDbId.value) {
    return { deviceDbId: selectedDeviceDbId.value }
  }
  const deviceId = selectedDeviceId.value?.trim()
  if (deviceId) {
    return { deviceId }
  }
  return null
})

const canPollBackendSignals = computed(() => !!signalsPollTarget.value)

const deviceOptions = computed(() => {
  const opts = [...devices.value]
  const simId = simulatorConfig.value?.default_sim_device_id
  if (simId && !opts.some((d) => (d.device_name || '') === simId)) {
    opts.unshift({
      id: simulatorConfig.value?.default_sim_db_id,
      device_name: simId,
      nick_name: simulatorConfig.value?.default_sim_device_name || 'Web 模拟设备',
      is_simulator: true
    })
  }
  return opts
})

const connectionLabel = computed(() => {
  switch (connectionState.value) {
    case 'connecting':
      return '连接中…'
    case 'connected':
      return sessionId.value ? `已连接 · ${sessionId.value.slice(0, 8)}…` : '已连接'
    case 'error':
      return '连接失败'
    default:
      return '未连接'
  }
})

const connectionTagType = computed(() => {
  if (connectionState.value === 'connected') return 'success'
  if (connectionState.value === 'error') return 'danger'
  if (connectionState.value === 'connecting') return 'warning'
  return 'info'
})

watch(transcript, async () => {
  await nextTick()
  const el = transcriptRef.value
  if (el) el.scrollTop = el.scrollHeight
}, { deep: true })

function getDeviceNickName(device) {
  return device.nick_name || device.device_name || device.device_code || '未命名设备'
}

function getDeviceOptionLabel(device) {
  const nick = getDeviceNickName(device)
  const id = device.device_name || '-'
  return `${nick} (${id})`
}

function roleLabel(item) {
  if (item.role === 'user') return '用户'
  if (item.role === 'assistant') return '助手'
  return '系统'
}

function formatTime(ts) {
  if (!ts) return ''
  return new Date(ts).toLocaleTimeString()
}

function formatSignalTs(tsMs) {
  if (!tsMs) return ''
  try {
    return new Date(tsMs).toLocaleTimeString()
  } catch {
    return String(tsMs)
  }
}

function formatJson(obj) {
  try {
    return JSON.stringify(obj, null, 2)
  } catch {
    return String(obj)
  }
}

function backendChannelLabel(channel) {
  const map = { mqtt: 'MQTT', ws: 'WebSocket', udp: 'UDP', llm: 'LLM' }
  return map[channel] || channel || '?'
}

function backendDirectionLabel(item) {
  if (item.direction === 'in') return '← 设备'
  if (item.direction === 'internal') return '⚙ 后台'
  return '→ 设备'
}

function backendDirectionTag(item) {
  if (item.direction === 'in') return 'warning'
  if (item.direction === 'internal') return 'danger'
  return 'primary'
}

function backendItemClass(item) {
  return {
    'backend-in': item.direction === 'in',
    'backend-out': item.direction === 'out',
    'backend-internal': item.direction === 'internal',
    'backend-mcp-call': item.msg_type === 'mcp_tool_call',
    'backend-mcp-result': item.msg_type === 'mcp_tool_result'
  }
}

async function handleClearBackendLog() {
  if (!signalsPollTarget.value) return
  try {
    await clearSignals(signalsPollTarget.value)
  } catch (e) {
    ElMessage.error(e?.message || '清空后台流水失败')
  }
}

function startBackendPolling({ reset = false } = {}) {
  const target = signalsPollTarget.value
  if (!target) {
    backendPollingActive.value = false
    pollingLogicalDeviceId = ''
    return
  }
  const logicalId = selectedDeviceId.value?.trim() || ''
  const sameDevice = backendPollingActive.value && logicalId && pollingLogicalDeviceId === logicalId
  stopSignalPolling()
  if (reset || !sameDevice) {
    mirroredBackendSignalIds.clear()
    void clearSignals(target)
  }
  startSignalPolling(target, 800)
  backendPollingActive.value = true
  pollingLogicalDeviceId = logicalId
}

function stopBackendPolling() {
  stopSignalPolling()
  backendPollingActive.value = false
  pollingLogicalDeviceId = ''
}

async function loadEndpoints() {
  if (!selectedDeviceDbId.value) {
    endpointStatus.value = null
    return
  }
  try {
    const res = await api.get(`/admin/devices/${selectedDeviceDbId.value}/endpoints`, {
      timeout: 15000,
      silentError: true
    })
    endpointStatus.value = res.data?.data || null
  } catch {
    endpointStatus.value = null
  }
}

function startEndpointPolling() {
  stopEndpointPolling()
  void loadEndpoints()
  endpointPollTimer = setInterval(() => {
    void loadEndpoints()
  }, 5000)
}

function stopEndpointPolling() {
  if (endpointPollTimer) {
    clearInterval(endpointPollTimer)
    endpointPollTimer = null
  }
}

function handleDeviceChange() {
  void loadEndpoints()
}

async function handleInjectMessage() {
  if (!selectedDeviceId.value?.trim() || !pushText.value.trim()) return
  pushing.value = true
  try {
    const res = await api.post('/admin/devices/inject-message', {
      device_id: selectedDeviceId.value.trim(),
      message: pushText.value.trim(),
      skip_llm: pushSkipLlm.value,
      auto_listen: pushAutoListen.value,
      target: pushTarget.value
    })
    if (res.data?.data?.success || res.data?.success) {
      ElMessage.success('消息已注入')
      if (signalsPollTarget.value) {
        startBackendPolling({ reset: false })
      }
    } else {
      ElMessage.error(res.data?.data?.error || res.data?.error || '注入失败')
    }
  } catch (e) {
    ElMessage.error(e?.response?.data?.error || e?.message || '注入失败')
  } finally {
    pushing.value = false
    void loadEndpoints()
  }
}

async function handleSpeak() {
  if (!selectedDeviceDbId.value || !pushText.value.trim()) return
  pushing.value = true
  try {
    const res = await api.post(`/admin/devices/${selectedDeviceDbId.value}/speak`, {
      text: pushText.value.trim(),
      target: pushTarget.value,
      auto_listen: pushAutoListen.value
    })
    const data = res.data?.data
    if (data?.success) {
      ElMessage.success('播报已发送')
    } else {
      ElMessage.error(data?.error || '播报失败')
    }
  } catch (e) {
    ElMessage.error(e?.response?.data?.error || e?.message || '播报失败')
  } finally {
    pushing.value = false
    void loadEndpoints()
  }
}

async function loadDevices() {
  loadingDevices.value = true
  try {
    const res = await api.get('/admin/devices')
    devices.value = res.data?.data || []
  } finally {
    loadingDevices.value = false
  }
}

async function reloadAll() {
  loadingConfig.value = true
  try {
    await Promise.all([loadDevices(), loadConfig(api)])
    const defaultSimId = simulatorConfig.value?.default_sim_device_id
    if (defaultSimId && !selectedDeviceId.value) {
      selectedDeviceId.value = defaultSimId
    }
    if (!wsUrlOverride.value && simulatorConfig.value?.local_ws_url) {
      wsUrlOverride.value = simulatorConfig.value.local_ws_url
    }
    await loadEndpoints()
    startEndpointPolling()
  } catch {
    ElMessage.error('加载模拟器配置失败')
  } finally {
    loadingConfig.value = false
  }
}

async function handleConnect() {
  connecting.value = true
  try {
    await connect({
      deviceId: selectedDeviceId.value,
      protocolVersion: protocolVersion.value,
      wsUrlOverride: wsUrlOverride.value
    })
    ElMessage.success('已连接并完成 hello 握手')
    void loadEndpoints()
    startBackendPolling({ reset: true })
  } catch (e) {
    ElMessage.error(e?.message || '连接失败')
  } finally {
    connecting.value = false
  }
}

function handleDisconnect() {
  disconnect()
  stopBackendPolling()
  void loadEndpoints()
  ElMessage.info('已断开连接')
}

function handleGoodbye() {
  sendGoodbye()
  ElMessage.info('已发送 goodbye')
}

function handleSend() {
  if (!draft.value.trim()) return
  if (sendText(draft.value)) {
    draft.value = ''
  } else {
    ElMessage.warning('发送失败，请确认已连接')
  }
}

function handleAbort() {
  if (sendAbort()) {
    ElMessage.info('已发送 abort')
  }
}

onMounted(() => {
  reloadAll()
})

onBeforeUnmount(() => {
  stopEndpointPolling()
  stopBackendPolling()
  disconnect()
})

watch(activeTab, async (tab) => {
  if (tab !== 'backend') return
  if (isConnected.value && signalsPollTarget.value && !backendPollingActive.value) {
    startBackendPolling({ reset: false })
  }
  await nextTick()
  const el = backendLogRef.value
  if (el) el.scrollTop = el.scrollHeight
})

watch(signalLog, async (log) => {
  for (const item of log) {
    if (!item?.id || mirroredBackendSignalIds.has(item.id)) continue
    if (item.msg_type === 'mcp_tool_call' || item.msg_type === 'mcp_tool_result') {
      mirroredBackendSignalIds.add(item.id)
      const title = item.msg_type === 'mcp_tool_call' ? 'MCP 调用' : 'MCP 结果'
      notifySystem(item.summary || item.msg_type, title)
    }
  }
  if (activeTab.value !== 'backend') return
  await nextTick()
  const el = backendLogRef.value
  if (el) el.scrollTop = el.scrollHeight
}, { deep: true })

watch([isConnected, signalsPollTarget], ([connected, target], [wasConnected]) => {
  if (connected && target) {
    const reset = !wasConnected
    startBackendPolling({ reset })
  } else if (!connected) {
    stopBackendPolling()
    mirroredBackendSignalIds.clear()
  }
})
</script>

<style scoped>
.device-chat-simulator {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.page-toolbar {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: 16px;
  flex-wrap: wrap;
}

.toolbar-copy .eyebrow {
  margin: 0;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

.toolbar-copy h2 {
  margin: 4px 0;
  font-size: 24px;
}

.subtitle {
  margin: 0;
  color: var(--el-text-color-secondary);
  max-width: 640px;
  line-height: 1.6;
}

.toolbar-actions {
  display: flex;
  align-items: center;
  gap: 12px;
}

.panel-card {
  height: 100%;
}

.connect-form .field-help {
  margin-top: 6px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  line-height: 1.5;
}

.connect-actions {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}

.error-alert {
  margin-top: 12px;
}

.feature-badges {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-bottom: 12px;
}

.switch-field {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  width: 100%;
}

.switch-desc {
  font-size: 13px;
  color: var(--el-text-color-regular);
}

.device-option {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 8px;
}

.endpoint-status {
  margin-bottom: 12px;
  padding: 10px 12px;
  border-radius: 8px;
  background: var(--el-fill-color-lighter);
}

.endpoint-status-title,
.push-panel-title {
  font-size: 13px;
  font-weight: 600;
  margin-bottom: 8px;
}

.endpoint-tags {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.endpoint-list {
  margin-top: 8px;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.endpoint-item {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  font-family: ui-monospace, monospace;
}

.push-panel {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.push-target {
  margin-bottom: 0;
}

.push-target :deep(.el-form-item__label) {
  padding-bottom: 4px;
}

.push-actions {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.push-buttons {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}

.chat-panel :deep(.el-card__body) {
  display: flex;
  flex-direction: column;
  min-height: 560px;
}

.chat-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.simulator-tabs {
  flex: 1;
  display: flex;
  flex-direction: column;
}

.simulator-tabs :deep(.el-tabs__content) {
  flex: 1;
  display: flex;
  flex-direction: column;
}

.simulator-tabs :deep(.el-tab-pane) {
  flex: 1;
  display: flex;
  flex-direction: column;
}

.transcript {
  flex: 1;
  min-height: 320px;
  max-height: 420px;
  overflow-y: auto;
  padding: 12px;
  background: var(--el-fill-color-lighter);
  border-radius: 8px;
  margin-bottom: 12px;
}

.empty-transcript {
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
}

.transcript-item {
  margin-bottom: 12px;
  padding: 10px 12px;
  border-radius: 10px;
  background: #fff;
  border: 1px solid var(--el-border-color-lighter);
}

.transcript-item.role-user {
  border-left: 3px solid var(--el-color-primary);
}

.transcript-item.role-assistant {
  border-left: 3px solid var(--el-color-success);
}

.transcript-item.role-system {
  border-left: 3px solid var(--el-color-info);
  background: var(--el-fill-color-blank);
}

.item-meta {
  display: flex;
  justify-content: space-between;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  margin-bottom: 4px;
}

.item-title {
  font-weight: 600;
  margin-bottom: 4px;
}

.item-content {
  white-space: pre-wrap;
  word-break: break-word;
  line-height: 1.6;
}

.composer-actions {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-top: 8px;
}

.composer-hint {
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.backend-log-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 12px;
}

.section-title {
  font-size: 13px;
  font-weight: 600;
}

.backend-log {
  max-height: 520px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.backend-item {
  padding: 10px 12px;
  border-radius: 10px;
  border: 1px solid var(--el-border-color-lighter);
  background: #fff;
}

.backend-item.backend-in {
  border-left: 3px solid var(--el-color-warning);
}

.backend-item.backend-out {
  border-left: 3px solid var(--el-color-primary);
}

.backend-item.backend-internal,
.backend-item.backend-mcp-call,
.backend-item.backend-mcp-result {
  border-left: 3px solid var(--el-color-danger);
  background: var(--el-color-danger-light-9);
}

.backend-head {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
  margin-bottom: 6px;
}

.backend-ts {
  margin-left: auto;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.backend-summary {
  line-height: 1.6;
  word-break: break-word;
}

.backend-detail {
  margin-top: 8px;
}

.backend-detail pre {
  margin: 8px 0 0;
  padding: 8px;
  border-radius: 8px;
  background: var(--el-fill-color-light);
  font-size: 12px;
  overflow-x: auto;
  white-space: pre-wrap;
  word-break: break-word;
}

@media (max-width: 992px) {
  .layout-row .el-col {
    margin-bottom: 16px;
  }
}
</style>
