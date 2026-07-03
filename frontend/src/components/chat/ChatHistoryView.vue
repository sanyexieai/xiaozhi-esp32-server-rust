<template>
  <div class="chat-history-page">
    <div class="page-header">
      <div class="header-left">
        <el-button v-if="showBack" @click="$router.back()" :icon="ArrowLeft" circle size="large" />
        <div class="header-context">
          <span class="context-label">{{ headerLabel }}</span>
          <strong class="context-value">{{ title }}</strong>
          <p class="context-meta" v-if="total > 0">共 {{ total }} 条消息</p>
        </div>
      </div>
      <div class="header-right">
        <el-button @click="goSessionsView">会话列表</el-button>
        <el-button v-if="scope === 'user'" @click="handleExport" :loading="exporting">
          <el-icon><Download /></el-icon>
          导出记录
        </el-button>
      </div>
    </div>

    <el-card class="filter-card" shadow="never">
      <el-form :model="filters" inline>
        <el-form-item v-if="scope === 'admin'" label="用户">
          <el-select
            v-model="filters.user_id"
            placeholder="全部"
            clearable
            filterable
            style="width: 150px"
          >
            <el-option
              v-for="user in users"
              :key="user.id"
              :label="user.username"
              :value="String(user.id)"
            />
          </el-select>
        </el-form-item>
        <el-form-item v-if="showAgentFilter" label="智能体">
          <el-select
            v-model="filters.agent_id"
            placeholder="全部"
            clearable
            filterable
            style="width: 160px"
            @change="onAgentFilterChange"
          >
            <el-option
              v-for="agent in agents"
              :key="agent.id"
              :label="agent.name"
              :value="String(agent.id)"
            />
          </el-select>
        </el-form-item>
        <el-form-item label="角色">
          <el-select v-model="filters.role" placeholder="全部" clearable style="width: 120px">
            <el-option label="全部" value="" />
            <el-option label="用户" value="user" />
            <el-option label="助手" value="assistant" />
          </el-select>
        </el-form-item>
        <el-form-item v-if="scope === 'user'" label="设备">
          <el-select v-model="filters.device_id" placeholder="全部" clearable style="width: 150px">
            <el-option label="全部" value="" />
            <el-option
              v-for="device in devices"
              :key="device.id"
              :label="device.device_name || device.device_code"
              :value="device.device_name"
            />
          </el-select>
        </el-form-item>
        <el-form-item v-if="scope === 'admin'" label="设备 ID">
          <el-input v-model="filters.device_id" placeholder="精确匹配" clearable style="width: 180px" />
        </el-form-item>
        <el-form-item label="开始日期">
          <el-date-picker
            v-model="filters.start_date"
            type="date"
            placeholder="选择日期"
            format="YYYY-MM-DD"
            value-format="YYYY-MM-DD"
            style="width: 150px"
            clearable
          />
        </el-form-item>
        <el-form-item label="结束日期">
          <el-date-picker
            v-model="filters.end_date"
            type="date"
            placeholder="选择日期"
            format="YYYY-MM-DD"
            value-format="YYYY-MM-DD"
            style="width: 150px"
            clearable
          />
        </el-form-item>
        <el-form-item>
          <el-button type="primary" @click="handleSearch">查询</el-button>
          <el-button @click="handleReset">重置</el-button>
        </el-form-item>
      </el-form>
    </el-card>

    <el-card class="messages-card" shadow="never" v-loading="loading">
      <div v-if="messages.length === 0" class="empty-state">
        <el-empty description="暂无聊天记录" />
      </div>
      <div v-else class="chat-container">
        <div class="chat-messages" ref="chatMessagesRef">
          <div
            v-for="(message, index) in messages"
            :key="message.id"
            class="message-wrapper"
            :class="{
              'message-right': message.role === 'user',
              'message-left': message.role === 'assistant'
            }"
          >
            <div v-if="shouldShowTime(message, index)" class="message-time-divider">
              {{ formatTimeShort(message.created_at) }}
            </div>

            <div v-if="scope === 'admin' && showMeta(message)" class="message-context-meta">
              <span v-if="message.username">用户：{{ message.username }}</span>
              <span v-if="message.agent_name">智能体：{{ message.agent_name }}</span>
              <span v-if="message.device_id">设备：{{ message.device_id }}</span>
            </div>

            <div class="message-bubble-wrapper">
              <div
                class="message-bubble"
                :class="message.role === 'user' ? 'message-bubble-right' : 'message-bubble-left'"
              >
                <div class="message-content-wrapper">
                  <div v-if="message.content" class="message-text">{{ message.content }}</div>
                  <div class="message-meta">
                    <span v-if="message.role === 'assistant'" class="message-time-small">
                      {{ formatTimeShort(message.created_at) }}
                    </span>
                    <el-dropdown trigger="click" @command="handleMessageAction">
                      <el-icon class="message-more"><MoreFilled /></el-icon>
                      <template #dropdown>
                        <el-dropdown-menu>
                          <el-dropdown-item :command="{ action: 'delete', id: message.id }">
                            删除
                          </el-dropdown-item>
                        </el-dropdown-menu>
                      </template>
                    </el-dropdown>
                    <span v-if="message.role === 'user'" class="message-time-small">
                      {{ formatTimeShort(message.created_at) }}
                    </span>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>

        <div class="pagination" v-if="total > 0">
          <el-pagination
            v-model:current-page="pagination.page"
            v-model:page-size="pagination.pageSize"
            :total="total"
            :page-sizes="[20, 50, 100]"
            layout="total, sizes, prev, pager, next, jumper"
            @size-change="handleSizeChange"
            @current-change="handlePageChange"
          />
        </div>
      </div>
    </el-card>
  </div>
</template>

<script setup>
import { computed, onMounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { ArrowLeft, Download, MoreFilled } from '@element-plus/icons-vue'
import {
  formatTimeShort,
  scrollTranscriptToBottom,
  shouldShowMessageTime,
  useChatHistory
} from '@/composables/useChatHistory'

const props = defineProps({
  scope: {
    type: String,
    default: 'user'
  },
  agentId: {
    type: [String, Number],
    default: null
  },
  showBack: {
    type: Boolean,
    default: false
  },
  pageTitle: {
    type: String,
    default: ''
  }
})

const {
  scope,
  loading,
  exporting,
  messages,
  total,
  agents,
  devices,
  users,
  agentName,
  filters,
  pagination,
  handleSearch,
  handleReset,
  handlePageChange,
  handleSizeChange,
  handleDelete,
  handleExport,
  onAgentFilterChange,
  init
} = useChatHistory({
  scope: props.scope,
  agentId: props.agentId
})

const chatMessagesRef = ref(null)
const router = useRouter()

function goSessionsView() {
  if (props.scope === 'admin') {
    router.push('/admin/chat-history')
    return
  }
  if (props.agentId) {
    router.push(`/user/agents/${props.agentId}/history`)
    return
  }
  router.push('/user/history')
}

const headerLabel = computed(() => (props.scope === 'admin' ? '管理端' : '对话记录'))
const title = computed(() => {
  if (props.pageTitle) return props.pageTitle
  if (props.agentId) return agentName.value || '智能体对话'
  return props.scope === 'admin' ? '全平台对话记录' : '我的对话记录'
})
const showAgentFilter = computed(() => props.scope === 'user' && !props.agentId)

const shouldShowTime = (message, index) => shouldShowMessageTime(message, index, messages.value)
const showMeta = (message) => message.username || message.agent_name || message.device_id

const handleMessageAction = (command) => {
  if (command.action === 'delete') {
    handleDelete(command.id)
  }
}

watch(messages, async () => {
  await scrollTranscriptToBottom(chatMessagesRef)
}, { deep: true })

onMounted(() => {
  init()
})
</script>

<style scoped>
.chat-history-page,
.agent-history-page {
  padding: 0;
  background: transparent;
  min-height: 100%;
}

.page-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 20px;
  padding: 20px;
  background: rgba(255, 255, 255, 0.88);
  border: 1px solid rgba(255, 255, 255, 0.9);
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
  color: var(--apple-text-secondary);
  font-size: 12px;
  font-weight: 600;
}

.context-value {
  color: var(--apple-text);
  font-size: 16px;
  line-height: 1.3;
}

.context-meta {
  margin: 0;
  color: var(--apple-text-secondary);
  font-size: 14px;
}

.filter-card,
.messages-card {
  margin-bottom: 20px;
}

.empty-state {
  padding: 60px 0;
  text-align: center;
}

.chat-container {
  background: rgba(248, 250, 252, 0.92);
  border: 1px solid rgba(229, 229, 234, 0.72);
  min-height: 500px;
  border-radius: 22px;
  overflow: hidden;
}

.chat-messages {
  padding: 20px;
  max-height: 70vh;
  overflow-y: auto;
}

.message-wrapper {
  display: flex;
  flex-direction: column;
  margin-bottom: 16px;
}

.message-context-meta {
  display: flex;
  flex-wrap: wrap;
  gap: 8px 16px;
  justify-content: center;
  margin-bottom: 6px;
  font-size: 12px;
  color: var(--apple-text-tertiary);
}

.message-time-divider {
  text-align: center;
  margin: 16px 0;
  font-size: 12px;
  color: var(--apple-text-tertiary);
}

.message-bubble-wrapper {
  display: flex;
  align-items: flex-start;
  max-width: 75%;
}

.message-right {
  margin-left: auto;
  justify-content: flex-end;
  width: 100%;
  display: flex;
}

.message-left {
  margin-right: auto;
  justify-content: flex-start;
  width: 100%;
  display: flex;
}

.message-bubble {
  position: relative;
  padding: 10px 14px;
  border-radius: 18px;
  word-wrap: break-word;
  word-break: break-word;
  box-shadow: 0 8px 16px rgba(15, 23, 42, 0.05);
  max-width: 100%;
}

.message-bubble-left {
  background: rgba(255, 255, 255, 0.94);
  border-top-left-radius: 8px;
}

.message-bubble-right {
  background: rgba(0, 122, 255, 0.12);
  border: 1px solid rgba(0, 122, 255, 0.16);
  border-top-right-radius: 8px;
  margin-left: auto;
}

.message-content-wrapper {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.message-text {
  color: var(--apple-text);
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
  font-size: 14px;
}

.message-meta {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-top: 4px;
  opacity: 0.7;
}

.message-meta:hover {
  opacity: 1;
}

.message-time-small {
  font-size: 11px;
  color: var(--apple-text-tertiary);
}

.message-bubble-right .message-time-small {
  color: var(--apple-primary-pressed);
}

.message-more {
  font-size: 14px;
  color: var(--apple-text-tertiary);
  cursor: pointer;
  padding: 2px;
  border-radius: 8px;
}

.pagination {
  margin-top: 20px;
  padding: 20px;
  display: flex;
  justify-content: center;
  background: rgba(255, 255, 255, 0.88);
  border-top: 1px solid rgba(229, 229, 234, 0.72);
}
</style>
