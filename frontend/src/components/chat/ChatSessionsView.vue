<template>
  <div class="chat-sessions-page">
    <div class="page-header">
      <div class="header-left">
        <el-button v-if="showBack" @click="$router.back()" :icon="ArrowLeft" circle size="large" />
        <div class="header-context">
          <span class="context-label">{{ headerLabel }}</span>
          <strong class="context-value">{{ title }}</strong>
          <p class="context-meta" v-if="total > 0">共 {{ total }} 个会话</p>
        </div>
      </div>
      <div class="header-right">
        <el-button @click="goFlatHistory">全部消息</el-button>
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

    <el-card class="sessions-card" shadow="never" v-loading="loading">
      <div v-if="sessions.length === 0" class="empty-state">
        <el-empty description="暂无聊天会话" />
      </div>
      <div v-else class="session-list">
        <div
          v-for="session in sessions"
          :key="session.session_id"
          class="session-item"
          @click="openSession(session)"
        >
          <div class="session-main">
            <div class="session-title">{{ truncatePreview(session.preview) }}</div>
            <div class="session-sub">{{ truncatePreview(session.last_preview, 64) }}</div>
          </div>
          <div class="session-meta">
            <span>{{ session.message_count }} 条</span>
            <span>{{ formatTimeShort(session.updated_at) }}</span>
            <span v-if="session.device_id" class="device-tag">{{ session.device_id }}</span>
            <el-button
              v-if="session.device_id"
              type="primary"
              link
              size="small"
              @click.stop="continueSession(session)"
            >
              继续聊
            </el-button>
          </div>
        </div>
      </div>

      <div class="pagination" v-if="total > 0">
        <el-pagination
          v-model:current-page="pagination.page"
          v-model:page-size="pagination.pageSize"
          :total="total"
          :page-sizes="[10, 20, 50]"
          layout="total, sizes, prev, pager, next, jumper"
          @size-change="handleSizeChange"
          @current-change="handlePageChange"
        />
      </div>
    </el-card>
  </div>
</template>

<script setup>
import { computed, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { ArrowLeft } from '@element-plus/icons-vue'
import { formatTimeShort } from '@/composables/useChatHistory'
import { truncatePreview, useChatSessions } from '@/composables/useChatSessions'

const props = defineProps({
  scope: { type: String, default: 'user' },
  agentId: { type: [String, Number], default: null },
  showBack: { type: Boolean, default: false },
  pageTitle: { type: String, default: '' }
})

const router = useRouter()

const {
  scope,
  loading,
  sessions,
  total,
  agents,
  devices,
  users,
  filters,
  pagination,
  handleSearch,
  handleReset,
  handlePageChange,
  handleSizeChange,
  onAgentFilterChange,
  init
} = useChatSessions({
  scope: props.scope,
  agentId: props.agentId
})

const headerLabel = computed(() => (props.scope === 'admin' ? '管理端' : '对话记录'))
const title = computed(() => {
  if (props.pageTitle) return props.pageTitle
  if (props.agentId) return '智能体会话'
  return props.scope === 'admin' ? '全平台会话' : '我的会话'
})
const showAgentFilter = computed(() => props.scope === 'user' && !props.agentId)

function sessionDetailPath(sessionId) {
  if (props.scope === 'admin') {
    return `/admin/history/sessions/${encodeURIComponent(sessionId)}`
  }
  if (props.agentId) {
    return `/user/agents/${props.agentId}/history/sessions/${encodeURIComponent(sessionId)}`
  }
  return `/user/history/sessions/${encodeURIComponent(sessionId)}`
}

function openSession(session) {
  router.push(sessionDetailPath(session.session_id))
}

function continueSession(session) {
  router.push(sessionDetailPath(session.session_id))
}

function goFlatHistory() {
  if (props.scope === 'admin') {
    router.push('/admin/chat-history/messages')
    return
  }
  if (props.agentId) {
    router.push(`/user/agents/${props.agentId}/history/messages`)
    return
  }
  router.push('/user/history/messages')
}

onMounted(() => {
  init()
})
</script>

<style scoped>
.chat-sessions-page {
  padding: 0;
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
}

.context-meta {
  margin: 0;
  color: var(--apple-text-secondary);
  font-size: 14px;
}

.filter-card,
.sessions-card {
  margin-bottom: 20px;
}

.empty-state {
  padding: 60px 0;
  text-align: center;
}

.session-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.session-item {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 16px;
  padding: 14px 16px;
  border-radius: 14px;
  border: 1px solid rgba(229, 229, 234, 0.8);
  background: rgba(255, 255, 255, 0.92);
  cursor: pointer;
  transition: background 0.15s, box-shadow 0.15s;
}

.session-item:hover {
  background: rgba(0, 122, 255, 0.06);
  box-shadow: 0 4px 12px rgba(15, 23, 42, 0.06);
}

.session-main {
  min-width: 0;
  flex: 1;
}

.session-title {
  font-weight: 600;
  color: var(--apple-text);
  margin-bottom: 4px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.session-sub {
  font-size: 13px;
  color: var(--apple-text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.session-meta {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 4px;
  font-size: 12px;
  color: var(--apple-text-tertiary);
  flex-shrink: 0;
}

.device-tag {
  max-width: 140px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.pagination {
  margin-top: 20px;
  display: flex;
  justify-content: center;
}
</style>
