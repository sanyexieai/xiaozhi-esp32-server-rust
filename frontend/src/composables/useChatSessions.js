import { computed, reactive, ref } from 'vue'
import { ElMessage } from 'element-plus'
import api from '@/utils/api'

export function useChatSessions(options = {}) {
  const scope = options.scope || 'user'
  const fixedAgentId = options.agentId ?? null

  const loading = ref(false)
  const sessions = ref([])
  const total = ref(0)
  const agents = ref([])
  const devices = ref([])
  const users = ref([])

  const filters = reactive({
    device_id: '',
    agent_id: fixedAgentId ? String(fixedAgentId) : '',
    user_id: '',
    start_date: '',
    end_date: ''
  })

  const pagination = reactive({
    page: 1,
    pageSize: 20
  })

  const listUrl = computed(() => {
    if (scope === 'admin') {
      return '/admin/history/sessions'
    }
    return '/user/history/sessions'
  })

  const sessionMessagesUrl = (sessionId) => {
    if (scope === 'admin') {
      return `/admin/history/sessions/${encodeURIComponent(sessionId)}/messages`
    }
    return `/user/history/sessions/${encodeURIComponent(sessionId)}/messages`
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

  async function loadSessions() {
    loading.value = true
    try {
      const response = await api.get(listUrl.value, { params: buildListParams() })
      sessions.value = response.data?.data || []
      total.value = response.data?.total || 0
    } catch (error) {
      if (error.response?.status === 404) {
        ElMessage.error('会话接口不存在(404)，请重新编译并重启 xiaozhi-manager')
      } else {
        ElMessage.error('加载会话列表失败: ' + (error.response?.data?.error || error.message))
      }
      sessions.value = []
      total.value = 0
    } finally {
      loading.value = false
    }
  }

  async function loadSessionMessages(sessionId, page = 1, pageSize = 100) {
    if (!sessionId) {
      return { messages: [], total: 0 }
    }
    const response = await api.get(sessionMessagesUrl(sessionId), {
      params: { page, page_size: pageSize }
    })
    const data = response.data?.data || []
    return {
      messages: [...data].reverse(),
      total: response.data?.total || 0
    }
  }

  async function handleSearch() {
    pagination.page = 1
    await loadSessions()
  }

  async function handleReset() {
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
    await loadSessions()
  }

  async function handlePageChange(page) {
    pagination.page = page
    await loadSessions()
  }

  async function handleSizeChange(size) {
    pagination.pageSize = size
    pagination.page = 1
    await loadSessions()
  }

  async function onAgentFilterChange() {
    filters.device_id = ''
    await loadDevices()
  }

  async function init() {
    await Promise.all([loadAgents(), loadUsers(), loadDevices(), loadSessions()])
  }

  return {
    scope,
    fixedAgentId,
    loading,
    sessions,
    total,
    agents,
    devices,
    users,
    filters,
    pagination,
    loadSessions,
    loadSessionMessages,
    handleSearch,
    handleReset,
    handlePageChange,
    handleSizeChange,
    onAgentFilterChange,
    init
  }
}

export function truncatePreview(text, max = 48) {
  const s = String(text || '').trim()
  if (!s) return '（无内容）'
  return s.length > max ? `${s.slice(0, max)}…` : s
}
