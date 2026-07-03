import { computed, nextTick, reactive, ref } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import api from '@/utils/api'

export function useChatHistory(options = {}) {
  const scope = options.scope || 'user' // user | admin
  const fixedAgentId = options.agentId ?? null

  const loading = ref(false)
  const exporting = ref(false)
  const messages = ref([])
  const total = ref(0)
  const agents = ref([])
  const devices = ref([])
  const users = ref([])
  const agentName = ref('')

  const filters = reactive({
    role: '',
    device_id: '',
    agent_id: fixedAgentId ? String(fixedAgentId) : '',
    user_id: '',
    start_date: '',
    end_date: ''
  })

  const pagination = reactive({
    page: 1,
    pageSize: 50
  })

  const listUrl = computed(() => {
    if (scope === 'admin') {
      return '/admin/history/messages'
    }
    if (fixedAgentId) {
      return `/user/history/agents/${fixedAgentId}/messages`
    }
    return '/user/history/messages'
  })

  const deleteUrl = (id) => {
    if (scope === 'admin') {
      return `/admin/history/messages/${id}`
    }
    return `/user/history/messages/${id}`
  }

  async function loadAgents() {
    if (scope !== 'user' || fixedAgentId) return
    try {
      const res = await api.get('/user/agents')
      agents.value = res.data?.data || []
    } catch (error) {
      console.error('加载智能体列表失败:', error)
    }
  }

  async function loadUsers() {
    if (scope !== 'admin') return
    try {
      const res = await api.get('/admin/users')
      users.value = res.data?.data || []
    } catch (error) {
      console.error('加载用户列表失败:', error)
    }
  }

  async function loadAgentMeta() {
    if (!fixedAgentId) return
    try {
      const res = await api.get(`/user/agents/${fixedAgentId}`)
      agentName.value = res.data?.data?.name || '智能体'
    } catch (error) {
      console.error('加载智能体信息失败:', error)
    }
  }

  async function loadDevices() {
    if (scope !== 'user') return
    try {
      if (fixedAgentId) {
        const res = await api.get(`/user/agents/${fixedAgentId}/devices`)
        devices.value = res.data?.data || []
      } else if (filters.agent_id) {
        const res = await api.get(`/user/agents/${filters.agent_id}/devices`)
        devices.value = res.data?.data || []
      } else {
        devices.value = []
      }
    } catch (error) {
      console.error('加载设备列表失败:', error)
    }
  }

  function buildListParams() {
    const params = {
      page: pagination.page,
      page_size: pagination.pageSize
    }
    if (filters.role) params.role = filters.role
    if (filters.device_id) params.device_id = filters.device_id
    if (filters.start_date) params.start_date = filters.start_date
    if (filters.end_date) params.end_date = filters.end_date
    if (scope === 'admin') {
      if (filters.user_id) params.user_id = Number(filters.user_id)
      if (filters.agent_id) params.agent_id = Number(filters.agent_id)
    } else if (fixedAgentId) {
      params.agent_id = Number(fixedAgentId)
    } else if (filters.agent_id) {
      params.agent_id = Number(filters.agent_id)
    }
    return params
  }

  async function loadMessages() {
    loading.value = true
    try {
      const response = await api.get(listUrl.value, { params: buildListParams() })
      const data = response.data?.data || []
      messages.value = [...data].reverse()
      total.value = response.data?.total || 0
    } catch (error) {
      ElMessage.error('加载消息列表失败: ' + (error.response?.data?.error || error.message))
      messages.value = []
      total.value = 0
    } finally {
      loading.value = false
    }
  }

  async function handleSearch() {
    pagination.page = 1
    await loadMessages()
  }

  async function handleReset() {
    filters.role = ''
    filters.device_id = ''
    filters.start_date = ''
    filters.end_date = ''
    if (!fixedAgentId) {
      filters.agent_id = ''
    }
    if (scope === 'admin') {
      filters.user_id = ''
    }
    pagination.page = 1
    await loadDevices()
    await loadMessages()
  }

  async function handlePageChange(page) {
    pagination.page = page
    await loadMessages()
  }

  async function handleSizeChange(size) {
    pagination.pageSize = size
    pagination.page = 1
    await loadMessages()
  }

  async function handleDelete(messageId) {
    try {
      await ElMessageBox.confirm('确定要删除这条消息吗？', '提示', {
        confirmButtonText: '确定',
        cancelButtonText: '取消',
        type: 'warning'
      })
      await api.delete(deleteUrl(messageId))
      ElMessage.success('删除成功')
      await loadMessages()
    } catch (error) {
      if (error !== 'cancel') {
        ElMessage.error('删除失败')
      }
    }
  }

  async function handleExport() {
    if (scope === 'admin') {
      ElMessage.info('管理端导出功能即将支持，请先在用户工作台导出')
      return
    }
    exporting.value = true
    try {
      const params = {}
      const agentId = fixedAgentId || filters.agent_id
      if (agentId) params.agent_id = agentId
      if (filters.role) params.role = filters.role
      if (filters.device_id) params.device_id = filters.device_id
      if (filters.start_date) params.start_date = filters.start_date
      if (filters.end_date) params.end_date = filters.end_date

      const response = await api.get('/user/history/export', {
        params,
        responseType: 'blob'
      })
      const url = window.URL.createObjectURL(new Blob([response.data]))
      const link = document.createElement('a')
      link.href = url
      link.setAttribute('download', `chat_history_${new Date().toISOString().slice(0, 10)}.json`)
      document.body.appendChild(link)
      link.click()
      link.remove()
      window.URL.revokeObjectURL(url)
      ElMessage.success('导出成功')
    } catch (error) {
      ElMessage.error('导出失败')
    } finally {
      exporting.value = false
    }
  }

  async function onAgentFilterChange() {
    filters.device_id = ''
    await loadDevices()
  }

  async function init() {
    await Promise.all([
      loadAgentMeta(),
      loadAgents(),
      loadUsers(),
      loadDevices(),
      loadMessages()
    ])
  }

  return {
    scope,
    fixedAgentId,
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
    listUrl,
    loadMessages,
    handleSearch,
    handleReset,
    handlePageChange,
    handleSizeChange,
    handleDelete,
    handleExport,
    onAgentFilterChange,
    init
  }
}

export function formatTimeShort(dateString) {
  const date = new Date(dateString)
  const now = new Date()
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate())
  const msgDate = new Date(date.getFullYear(), date.getMonth(), date.getDate())

  if (msgDate.getTime() === today.getTime()) {
    return date.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })
  }

  const yesterday = new Date(today)
  yesterday.setDate(yesterday.getDate() - 1)
  if (msgDate.getTime() === yesterday.getTime()) {
    return `昨天 ${date.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })}`
  }

  if (date.getFullYear() === now.getFullYear()) {
    return `${date.getMonth() + 1}月${date.getDate()}日 ${date.toLocaleTimeString('zh-CN', {
      hour: '2-digit',
      minute: '2-digit'
    })}`
  }

  return date.toLocaleString('zh-CN', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit'
  })
}

export function shouldShowMessageTime(message, index, messages) {
  if (!message?.created_at) return false
  if (index == null || index <= 0) return true
  const prev = messages[index - 1]
  if (!prev?.created_at) return true
  const currentTime = new Date(message.created_at).getTime()
  const prevTime = new Date(prev.created_at).getTime()
  return currentTime - prevTime > 5 * 60 * 1000
}

export async function scrollTranscriptToBottom(containerRef) {
  await nextTick()
  const el = containerRef.value
  if (el) {
    el.scrollTop = el.scrollHeight
  }
}
