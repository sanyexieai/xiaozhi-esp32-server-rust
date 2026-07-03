<template>
  <div class="session-detail-page" v-loading="pageLoading">
    <div class="page-header">
      <div class="header-left">
        <el-button @click="goBack" :icon="ArrowLeft" circle size="large" />
        <div class="header-context">
          <span class="context-label">{{ scope === 'admin' ? '管理端 · 会话详情' : '会话详情' }}</span>
          <strong class="context-value">{{ sessionTitle }}</strong>
          <p class="context-meta">
            <span v-if="sessionMeta.device_id">设备 {{ sessionMeta.device_id }}</span>
            <span v-if="sessionMeta.message_count"> · {{ sessionMeta.message_count }} 条历史</span>
            <span v-if="activeSessionId"> · 会话 {{ shortSessionId }}</span>
          </p>
        </div>
      </div>
      <div class="header-right" v-if="canContinueChat">
        <el-tag :type="connectionTagType" size="small">{{ connectionLabel }}</el-tag>
        <el-button v-if="isConnected" @click="stopContinueChat">结束连接</el-button>
        <el-button v-else type="primary" :loading="connecting" @click="startContinueChat">
          重新连接
        </el-button>
      </div>
    </div>

    <el-card class="chat-card" shadow="never">
      <div v-if="displayMessages.length === 0 && !pageLoading" class="empty-state">
        <el-empty description="该会话暂无消息" />
      </div>

      <div v-else class="chat-messages" ref="chatRef">
        <div
          v-for="(message, index) in displayMessages"
          :key="message.id"
          class="message-wrapper"
          :class="message.role === 'user' ? 'message-right' : 'message-left'"
        >
          <div
            v-if="message.created_at && shouldShowTime(message, index)"
            class="message-time-divider"
          >
            {{ formatTimeShort(message.created_at) }}
          </div>
          <div class="message-bubble-wrapper">
            <div
              class="message-bubble"
              :class="message.role === 'user' ? 'message-bubble-right' : 'message-bubble-left'"
            >
              <div class="message-text">{{ message.content }}</div>
            </div>
          </div>
        </div>

        <div v-if="connecting" class="connecting-hint">正在恢复会话上下文…</div>
      </div>

      <div v-if="canContinueChat" class="chat-input-bar">
        <el-input
          v-model="inputText"
          type="textarea"
          :rows="2"
          :disabled="connecting"
          :placeholder="inputPlaceholder"
          @keydown.enter.exact.prevent="sendMessage"
        />
        <el-button
          type="primary"
          :loading="connecting || sending"
          :disabled="!inputText.trim()"
          @click="sendMessage"
        >
          发送
        </el-button>
      </div>
    </el-card>
  </div>
</template>

<script setup>
import { computed, nextTick, onMounted, onUnmounted, reactive, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { ArrowLeft } from '@element-plus/icons-vue'
import { ElMessage } from 'element-plus'
import api from '@/utils/api'
import {
  formatTimeShort,
  shouldShowMessageTime
} from '@/composables/useChatHistory'
import { truncatePreview, useChatSessions } from '@/composables/useChatSessions'
import { useDeviceChatSimulator } from '@/composables/useDeviceChatSimulator'
import { useAuthStore } from '@/stores/auth'

const props = defineProps({
  scope: { type: String, default: 'user' },
  agentId: { type: [String, Number], default: null }
})

const route = useRoute()
const router = useRouter()
const sessionId = computed(() => route.params.sessionId)

const pageLoading = ref(false)
const historyMessages = ref([])
const chatRef = ref(null)
const inputText = ref('')
const connecting = ref(false)
const sending = ref(false)
const sessionMeta = reactive({
  device_id: '',
  message_count: 0,
  preview: ''
})

const canContinueChat = computed(() => Boolean(sessionMeta.device_id))

const authStore = useAuthStore()

const { loadSessionMessages } = useChatSessions({
  scope: props.scope,
  agentId: props.agentId
})

const {
  connectionState,
  sessionId: activeSessionId,
  transcript,
  lastError,
  isConnected,
  loadConfig,
  connect,
  disconnect,
  setTtsEnabled,
  sendText,
  clearTranscript
} = useDeviceChatSimulator({
  quietConnect: true
})

function resolveChatConfigPath() {
  // 管理员统一走已稳定的 device-simulator 代理（可连任意设备）
  if (props.scope === 'admin' || authStore.isAdmin) {
    return '/admin/device-simulator/config'
  }
  return '/user/device-chat/config'
}

const sessionTitle = computed(() => truncatePreview(sessionMeta.preview, 60))
const shortSessionId = computed(() => {
  const id = activeSessionId.value || sessionId.value || ''
  return id.length > 8 ? `${id.slice(0, 8)}…` : id
})

const liveMessages = computed(() =>
  transcript.value
    .filter((entry) => entry.kind === 'text' && ['user', 'assistant'].includes(entry.role))
    .map((entry) => ({
      id: entry.id,
      role: entry.role,
      content: entry.content,
      created_at: entry.ts,
      live: true
    }))
)

const displayMessages = computed(() => {
  const history = historyMessages.value
    .filter((message) => message && message.content != null)
    .map((message, index) => ({
      ...message,
      id: `history-${message.id ?? index}`,
      _index: index
    }))
  return [...history, ...liveMessages.value]
})

const connectionLabel = computed(() => {
  const map = {
    idle: '未连接',
    connecting: '连接中',
    connected: '已连接',
    error: '连接异常'
  }
  return map[connectionState.value] || connectionState.value
})

const connectionTagType = computed(() => {
  if (connectionState.value === 'connected') return 'success'
  if (connectionState.value === 'error') return 'danger'
  if (connectionState.value === 'connecting') return 'warning'
  return 'info'
})

const inputPlaceholder = computed(() => {
  if (connecting.value) return '正在连接…'
  if (!isConnected.value) return '发送时将自动连接并继续本会话'
  return '输入消息，Enter 发送'
})

const shouldShowTime = (message, index) =>
  shouldShowMessageTime(message, index, displayMessages.value)

function listBackPath() {
  if (props.scope === 'admin') {
    return '/admin/chat-history'
  }
  const agentId = props.agentId || route.params.id
  if (agentId) {
    return `/user/agents/${agentId}/history`
  }
  return '/user/history'
}

function goBack() {
  router.push(listBackPath())
}

async function loadHistory() {
  pageLoading.value = true
  try {
    const { messages, total } = await loadSessionMessages(sessionId.value, 1, 200)
    historyMessages.value = messages
    sessionMeta.message_count = total
    const firstUser = messages.find((m) => m.role === 'user')
    const first = messages[0]
    sessionMeta.preview = firstUser?.content || first?.content || ''
    sessionMeta.device_id = first?.device_id || messages[messages.length - 1]?.device_id || ''
  } catch (error) {
    ElMessage.error('加载会话消息失败: ' + (error.response?.data?.error || error.message))
  } finally {
    pageLoading.value = false
  }
}

async function startContinueChat() {
  if (!canContinueChat.value) {
    ElMessage.warning('无法确定会话关联设备')
    return false
  }
  if (isConnected.value) return true

  connecting.value = true
  try {
    const configPath = resolveChatConfigPath()
    try {
      await loadConfig(api, configPath)
    } catch (error) {
      if (error.response?.status === 404 && configPath === '/user/device-chat/config') {
        throw new Error(
          '用户端对话接口未找到(404)，请重新编译并重启 xiaozhi-manager：cargo build -p xiaozhi-manager'
        )
      }
      throw error
    }
    await connect({
      deviceId: sessionMeta.device_id,
      resumeSessionId: sessionId.value
    })
    setTtsEnabled(true)
    if (!isConnected.value) {
      throw new Error(lastError.value || '连接未就绪')
    }
    return true
  } catch (error) {
    const msg =
      error.response?.status === 404
        ? `接口不存在(404): ${error.config?.url || '未知'}，请确认 xiaozhi-manager 已重新编译并运行在 8080 端口`
        : error.message || lastError.value || '连接失败'
    ElMessage.error(msg)
    return false
  } finally {
    connecting.value = false
  }
}

function stopContinueChat() {
  disconnect()
  clearTranscript()
}

async function sendMessage() {
  const text = inputText.value.trim()
  if (!text || !canContinueChat.value) return

  sending.value = true
  try {
    if (!isConnected.value) {
      const ok = await startContinueChat()
      if (!ok) return
    }
    if (sendText(text)) {
      inputText.value = ''
      await scrollToBottom()
      const assistantBefore = transcript.value.filter(
        (m) => m.role === 'assistant' && m.kind === 'text'
      ).length
      for (let i = 0; i < 60; i++) {
        await new Promise((resolve) => setTimeout(resolve, 500))
        const assistantNow = transcript.value.filter(
          (m) => m.role === 'assistant' && m.kind === 'text'
        ).length
        if (assistantNow > assistantBefore) return
      }
      ElMessage.warning(
        '暂未收到 AI 回复。硬件若未进入唤醒状态，请先在设备上唤醒或检查 speak_ready 日志'
      )
    } else {
      ElMessage.error(lastError.value || '发送失败，请检查 WebSocket 连接')
    }
  } finally {
    sending.value = false
  }
}

async function scrollToBottom() {
  await nextTick()
  if (chatRef.value) {
    chatRef.value.scrollTop = chatRef.value.scrollHeight
  }
}

watch(displayMessages, scrollToBottom, { deep: true })

onMounted(() => {
  loadHistory()
})

onUnmounted(() => {
  disconnect()
})
</script>

<style scoped>
.session-detail-page {
  min-height: 100%;
}

.page-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 20px;
  padding: 20px;
  background: rgba(255, 255, 255, 0.88);
  border-radius: var(--apple-radius-lg);
  box-shadow: var(--apple-shadow-md);
}

.header-left {
  display: flex;
  align-items: center;
  gap: 16px;
}

.header-context {
  display: grid;
  gap: 4px;
}

.context-label {
  font-size: 12px;
  color: var(--apple-text-secondary);
}

.context-value {
  font-size: 16px;
}

.context-meta {
  margin: 0;
  font-size: 13px;
  color: var(--apple-text-secondary);
}

.header-right {
  display: flex;
  align-items: center;
  gap: 8px;
}

.chat-card {
  margin-bottom: 20px;
  display: flex;
  flex-direction: column;
}

.chat-card :deep(.el-card__body) {
  display: flex;
  flex-direction: column;
  padding: 0;
}

.empty-state {
  padding: 48px 0;
}

.chat-messages {
  flex: 1;
  min-height: 420px;
  max-height: calc(100vh - 280px);
  overflow-y: auto;
  padding: 16px;
  background: rgba(248, 250, 252, 0.9);
}

.message-wrapper {
  display: flex;
  flex-direction: column;
  margin-bottom: 12px;
}

.message-time-divider {
  text-align: center;
  font-size: 12px;
  color: var(--apple-text-tertiary);
  margin: 8px 0;
}

.message-bubble-wrapper {
  max-width: 75%;
}

.message-right {
  align-items: flex-end;
  margin-left: auto;
}

.message-left {
  align-items: flex-start;
}

.message-bubble {
  padding: 10px 14px;
  border-radius: 16px;
  font-size: 14px;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
}

.message-bubble-right {
  background: rgba(0, 122, 255, 0.12);
}

.message-bubble-left {
  background: rgba(255, 255, 255, 0.94);
  border: 1px solid rgba(229, 229, 234, 0.8);
}

.connecting-hint {
  text-align: center;
  font-size: 13px;
  color: var(--apple-text-tertiary);
  padding: 8px 0;
}

.chat-input-bar {
  display: flex;
  gap: 8px;
  align-items: flex-end;
  padding: 16px;
  border-top: 1px solid rgba(229, 229, 234, 0.72);
  background: rgba(255, 255, 255, 0.96);
}

.chat-input-bar .el-button {
  flex-shrink: 0;
}
</style>
