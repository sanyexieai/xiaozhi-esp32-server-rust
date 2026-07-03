<template>
  <div class="mcp-config">
    <el-form ref="formRef" :model="form" :rules="rules" class="config-form" v-loading="loading">
      <div class="config-layout">
        <el-card class="config-card config-card-main" shadow="never">
          <template #header>
            <div class="card-head">
              <div>
                <p class="card-kicker">Global MCP</p>
                <h3>全局 MCP 服务</h3>
                <p class="card-description">维护服务端统一可用的 MCP 服务器、重连策略和允许工具范围。</p>
              </div>
              <div class="card-head-tags">
                <el-tag
                  v-if="testResult"
                  :type="testResult.ok ? 'success' : 'danger'"
                  effect="plain"
                  round
                >
                  {{ formatTestResultLabel(testResult, { successPrefix: '连通' }) }}
                </el-tag>
                <el-tag :type="form.mcp.global.enabled ? 'success' : 'info'" effect="plain" round>
                  {{ form.mcp.global.enabled ? `${enabledServerCount} 个启用服务` : '全局 MCP 已停用' }}
                </el-tag>
              </div>
            </div>
          </template>

          <div class="field-grid field-grid-main">
            <el-form-item label="启用全局 MCP" prop="mcp.global.enabled">
              <div class="switch-field">
                <div>
                  <div class="switch-title">允许服务端统一连接 MCP</div>
                  <div class="field-help">关闭后不会主动建立全局 MCP 连接，但本地 MCP 仍可单独控制。</div>
                </div>
                <el-switch v-model="form.mcp.global.enabled" />
              </div>
            </el-form-item>

            <el-form-item label="重连间隔（秒）" prop="mcp.global.reconnect_interval">
              <el-input-number
                v-model="form.mcp.global.reconnect_interval"
                :min="1"
                :max="3600"
                controls-position="right"
                style="width: 100%"
              />
            </el-form-item>

            <el-form-item label="最大重连次数" prop="mcp.global.max_reconnect_attempts">
              <el-input-number
                v-model="form.mcp.global.max_reconnect_attempts"
                :min="1"
                :max="100"
                controls-position="right"
                style="width: 100%"
              />
            </el-form-item>
          </div>

          <div class="local-mcp-section">
            <div class="section-head">
              <div>
                <h4>本地 MCP 能力</h4>
                <p>主程序本地暴露给模型的基础能力开关，可按场景逐项控制。</p>
              </div>
            </div>

            <div class="local-mcp-grid">
              <el-form-item label="退出对话" prop="local_mcp.exit_conversation" class="local-mcp-item">
                <div class="switch-field compact">
                  <div>
                    <div class="switch-title">允许模型结束当前会话</div>
                    <div class="field-help">适合需要主动收尾、关闭会话的工具链场景。</div>
                  </div>
                  <el-switch v-model="form.local_mcp.exit_conversation" />
                </div>
              </el-form-item>

              <el-form-item label="清除对话历史" prop="local_mcp.clear_conversation_history" class="local-mcp-item">
                <div class="switch-field compact">
                  <div>
                    <div class="switch-title">允许模型清空当前上下文</div>
                    <div class="field-help">适合切换任务或重置上下文时主动调用。</div>
                  </div>
                  <el-switch v-model="form.local_mcp.clear_conversation_history" />
                </div>
              </el-form-item>

              <el-form-item label="播放音乐" prop="local_mcp.play_music" class="local-mcp-item">
                <div class="switch-field compact">
                  <div>
                    <div class="switch-title">允许模型触发音乐播放</div>
                    <div class="field-help">若不需要音频娱乐能力，可关闭。</div>
                  </div>
                  <el-switch v-model="form.local_mcp.play_music" />
                </div>
              </el-form-item>
            </div>
          </div>

          <div class="server-list">
            <div class="server-list-header">
              <div>
                <h4>服务器列表</h4>
                <p>在列表中管理 MCP 服务器；连续探测失败 {{ MAX_PROBE_FAILURES }} 次将自动禁用该服务。</p>
              </div>
              <el-button type="primary" @click="openCreateServerDialog">
                <el-icon><Plus /></el-icon>
                添加服务器
              </el-button>
            </div>

            <el-table
              :data="form.mcp.global.servers"
              stripe
              class="server-table"
              empty-text="还没有 MCP 服务器，点击右上角添加"
            >
              <el-table-column label="名称" min-width="140" show-overflow-tooltip>
                <template #default="{ row }">
                  <span :class="{ 'text-muted': !row.name }">{{ row.name || '未命名' }}</span>
                </template>
              </el-table-column>

              <el-table-column label="传输" width="140">
                <template #default="{ row }">
                  <el-tag size="small" effect="plain">{{ formatTransport(row.type) }}</el-tag>
                </template>
              </el-table-column>

              <el-table-column label="URL" min-width="280" show-overflow-tooltip>
                <template #default="{ row }">
                  <span :class="{ 'text-muted': !row.url }">{{ row.url || '未填写' }}</span>
                </template>
              </el-table-column>

              <el-table-column label="工具" width="120">
                <template #default="{ row }">
                  <el-tag size="small" :type="row.allowed_tools?.length ? 'warning' : 'info'">
                    {{ row.allowed_tools?.length ? `${row.allowed_tools.length} 个已选` : '全部工具' }}
                  </el-tag>
                </template>
              </el-table-column>

              <el-table-column label="探测状态" width="150">
                <template #default="{ row }">
                  <el-tooltip
                    v-if="row.probe_last_error"
                    :content="row.probe_last_error"
                    placement="top"
                    :show-after="300"
                  >
                    <el-tag size="small" :type="probeStatusTagType(row)" effect="plain">
                      {{ formatProbeStatus(row) }}
                    </el-tag>
                  </el-tooltip>
                  <el-tag v-else size="small" :type="probeStatusTagType(row)" effect="plain">
                    {{ formatProbeStatus(row) }}
                  </el-tag>
                </template>
              </el-table-column>

              <el-table-column label="启用" width="90" align="center">
                <template #default="{ row }">
                  <el-switch v-model="row.enabled" size="small" @change="(v) => onServerEnabledChange(row, v)" />
                </template>
              </el-table-column>

              <el-table-column label="操作" width="260" fixed="right">
                <template #default="{ row, $index }">
                  <el-button link type="primary" @click="openEditServerDialog(row, $index)">编辑</el-button>
                  <el-button link type="primary" @click="openServerToolsDialog(row)">工具选择</el-button>
                  <el-button
                    link
                    type="primary"
                    :loading="row._tools_loading"
                    @click="discoverGlobalServerTools(row)"
                  >
                    探测
                  </el-button>
                  <el-button link type="danger" @click="confirmRemoveServer($index, row)">删除</el-button>
                </template>
              </el-table-column>
            </el-table>
          </div>
        </el-card>
      </div>

      <div class="footer-bar">
        <p class="footer-note">
          保存后会更新默认 MCP 全局配置；探测状态会一并保存。连续失败 {{ MAX_PROBE_FAILURES }} 次的服务将自动禁用。
        </p>
        <div class="footer-actions">
          <el-button plain :loading="loading" @click="loadConfig">重置为当前配置</el-button>
          <el-button type="warning" plain :loading="testing" @click="handleTestConnectivity">
            测试连通性
          </el-button>
          <el-button type="primary" :loading="saving" @click="handleSave">保存配置</el-button>
        </div>
      </div>
    </el-form>

    <el-dialog
      v-model="serverDialogVisible"
      :title="editingServerIndex === null ? '添加服务器' : '编辑服务器'"
      width="640px"
      destroy-on-close
    >
      <el-form ref="serverFormRef" :model="serverForm" :rules="serverRules" label-width="100px">
        <el-form-item label="名称" prop="name">
          <el-input v-model="serverForm.name" placeholder="例如：Amap MCP" />
        </el-form-item>
        <el-form-item label="传输" prop="type">
          <el-select v-model="serverForm.type" placeholder="选择服务器类型" style="width: 100%">
            <el-option label="SSE" value="sse" />
            <el-option label="StreamableHTTP" value="streamablehttp" />
          </el-select>
        </el-form-item>
        <el-form-item label="URL" prop="url">
          <el-input v-model="serverForm.url" placeholder="例如：https://example.com/mcp" />
        </el-form-item>
        <el-form-item label="启用">
          <el-switch v-model="serverForm.enabled" />
          <span class="inline-help">停用后该服务不会参与全局工具发现与调用</span>
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="serverDialogVisible = false">取消</el-button>
        <el-button type="primary" :loading="serverSaving" @click="saveServerDialog">保存</el-button>
      </template>
    </el-dialog>

    <el-dialog v-model="toolsDialogVisible" title="工具选择" width="720px" destroy-on-close>
      <div v-if="toolsTargetServer" class="tools-dialog-head">
        <div>
          <strong>{{ toolsTargetServer.name || '未命名服务器' }}</strong>
          <p class="field-help">{{ toolsTargetServer.url || '请先填写 URL' }}</p>
        </div>
        <el-button
          size="small"
          :loading="toolsTargetServer._tools_loading"
          @click="discoverGlobalServerTools(toolsTargetServer)"
        >
          重新探测
        </el-button>
      </div>
      <el-radio-group v-model="toolsMode" class="tools-mode" @change="handleToolsModeChange">
        <el-radio value="all">允许全部工具</el-radio>
        <el-radio value="selected">仅允许选定工具</el-radio>
      </el-radio-group>
      <el-select
        v-if="toolsMode === 'selected'"
        v-model="toolsSelected"
        multiple
        filterable
        clearable
        collapse-tags
        collapse-tags-tooltip
        style="width: 100%"
        placeholder="选择要暴露给模型的工具"
        :loading="toolsTargetServer?._tools_loading"
      >
        <el-option
          v-for="tool in toolsTargetServer?._tool_options || []"
          :key="tool.name"
          :label="tool.name"
          :value="tool.name"
        >
          <div class="tool-option-row">
            <span class="tool-option-name">{{ tool.name }}</span>
            <span class="tool-option-desc">{{ tool.description || '无描述' }}</span>
          </div>
        </el-option>
      </el-select>
      <p v-else class="field-help tools-all-hint">不限制工具范围时，该服务器的全部工具均可被模型调用。</p>
      <template #footer>
        <el-button @click="toolsDialogVisible = false">取消</el-button>
        <el-button type="primary" @click="saveServerTools">确定</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup>
import { computed, onMounted, reactive, ref } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Plus } from '@element-plus/icons-vue'
import api from '@/utils/api'
import {
  testWithData,
  formatTestMessage,
  formatTestResultLabel
} from '@/utils/configTest'

const loading = ref(false)
const saving = ref(false)
const testing = ref(false)
const testResult = ref(null)
const configId = ref(null)
const formRef = ref()
const serverFormRef = ref()
const serverDialogVisible = ref(false)
const serverSaving = ref(false)
const editingServerIndex = ref(null)
const toolsDialogVisible = ref(false)
const toolsTargetServer = ref(null)
const toolsMode = ref('all')
const toolsSelected = ref([])

/** 连续探测失败达到此次数后自动禁用 */
const MAX_PROBE_FAILURES = 3

const serverForm = reactive({
  name: '',
  type: 'streamablehttp',
  url: '',
  enabled: true
})

const serverRules = {
  name: [{ required: true, message: '请输入服务器名称', trigger: 'blur' }],
  type: [{ required: true, message: '请选择传输类型', trigger: 'change' }],
  url: [{ required: true, message: '请输入服务器 URL', trigger: 'blur' }]
}

const createDefaultState = () => ({
  mcp: {
    global: {
      enabled: true,
      servers: [],
      reconnect_interval: 300,
      max_reconnect_attempts: 10
    }
  },
  local_mcp: {
    exit_conversation: true,
    clear_conversation_history: true,
    play_music: false
  }
})

const form = reactive(createDefaultState())

const rules = {
  'mcp.global.reconnect_interval': [
    { required: true, message: '请输入重连间隔', trigger: 'blur' },
    { type: 'number', min: 1, max: 3600, message: '重连间隔必须在 1-3600 之间', trigger: 'blur' }
  ],
  'mcp.global.max_reconnect_attempts': [
    { required: true, message: '请输入最大重连次数', trigger: 'blur' },
    { type: 'number', min: 1, max: 100, message: '最大重连次数必须在 1-100 之间', trigger: 'blur' }
  ]
}

const createGlobalServer = () => ({
  name: '',
  type: 'streamablehttp',
  url: '',
  enabled: true,
  allowed_tools: [],
  probe_status: 'unknown',
  probe_fail_count: 0,
  probe_last_error: '',
  probe_last_at: '',
  probe_tool_count: 0,
  _tool_options: [],
  _tools_loading: false
})

const resetProbeMeta = (server) => {
  server.probe_status = 'unknown'
  server.probe_fail_count = 0
  server.probe_last_error = ''
  server.probe_last_at = ''
  server.probe_tool_count = 0
}

const markProbeSuccess = (server, toolCount) => {
  server.probe_status = 'ok'
  server.probe_fail_count = 0
  server.probe_last_error = ''
  server.probe_last_at = new Date().toISOString()
  server.probe_tool_count = toolCount
}

const markProbeFailure = (server, errorMessage) => {
  const failCount = (Number(server.probe_fail_count) || 0) + 1
  server.probe_fail_count = failCount
  server.probe_status = 'failed'
  server.probe_last_error = String(errorMessage || '探测失败').trim()
  server.probe_last_at = new Date().toISOString()
  server.probe_tool_count = 0

  if (failCount >= MAX_PROBE_FAILURES) {
    server.enabled = false
    ElMessage.warning(
      `「${server.name || '未命名'}」已连续失败 ${failCount} 次，已自动禁用`
    )
  }
}

const formatProbeStatus = (row) => {
  if (row._tools_loading) return '探测中…'
  const failCount = Number(row.probe_fail_count) || 0
  if (!row.enabled && failCount >= MAX_PROBE_FAILURES) {
    return `已自动禁用`
  }
  if (row.probe_status === 'ok') {
    const n = Number(row.probe_tool_count) || row._tool_options?.length || 0
    return n > 0 ? `正常 · ${n} 工具` : '正常'
  }
  if (row.probe_status === 'failed') {
    return `失败 ${failCount}/${MAX_PROBE_FAILURES}`
  }
  return '未探测'
}

const probeStatusTagType = (row) => {
  if (row._tools_loading) return 'warning'
  const failCount = Number(row.probe_fail_count) || 0
  if (!row.enabled && failCount >= MAX_PROBE_FAILURES) return 'danger'
  if (row.probe_status === 'ok') return 'success'
  if (row.probe_status === 'failed') return 'danger'
  return 'info'
}

const onServerEnabledChange = (row, enabled) => {
  if (!enabled) return
  const failCount = Number(row.probe_fail_count) || 0
  if (failCount >= MAX_PROBE_FAILURES || row.probe_status === 'failed') {
    resetProbeMeta(row)
  }
}

const syncProbeStatusFromConnectivityTest = (result) => {
  const servers = form.mcp.global.servers.filter((s) => String(s.url || '').trim())
  if (servers.length === 0) return

  if (result.ok) {
    for (const server of servers) {
      markProbeSuccess(server, server.probe_tool_count || server._tool_options?.length || 0)
    }
    return
  }

  const failed = new Map()
  String(result.message || '')
    .split(';')
    .forEach((part) => {
      const trimmed = part.trim()
      const idx = trimmed.indexOf(':')
      if (idx <= 0) return
      const name = trimmed.slice(0, idx).trim()
      const err = trimmed.slice(idx + 1).trim()
      if (name && err) failed.set(name, err)
    })

  if (failed.size === 0) return

  for (const server of servers) {
    const err = failed.get(server.name)
    if (err) {
      markProbeFailure(server, err)
    } else {
      markProbeSuccess(server, server.probe_tool_count || server._tool_options?.length || 0)
    }
  }
}

const mergeServerToolOptions = (server, tools = []) => {
  const merged = new Map()

  ;(tools || []).forEach((tool) => {
    if (!tool?.name) return
    merged.set(tool.name, {
      name: tool.name,
      description: tool.description || ''
    })
  })

  ;(server.allowed_tools || []).forEach((name) => {
    if (!name || merged.has(name)) return
    merged.set(name, {
      name,
      description: '当前已选择'
    })
  })

  server._tool_options = Array.from(merged.values()).sort((a, b) => a.name.localeCompare(b.name))
}

const normalizeGlobalServer = (server = {}) => {
  const failCount = Number(server.probe_fail_count) || 0
  let enabled = server.enabled !== false
  if (failCount >= MAX_PROBE_FAILURES) {
    enabled = false
  }

  const normalized = {
    ...server,
    name: server.name || '',
    type: server.type || 'streamablehttp',
    url: server.url || '',
    enabled,
    allowed_tools: Array.isArray(server.allowed_tools) ? [...server.allowed_tools] : [],
    probe_status: server.probe_status || (failCount > 0 ? 'failed' : 'unknown'),
    probe_fail_count: failCount,
    probe_last_error: server.probe_last_error || '',
    probe_last_at: server.probe_last_at || '',
    probe_tool_count: Number(server.probe_tool_count) || 0,
    _tool_options: [],
    _tools_loading: false
  }
  mergeServerToolOptions(normalized)
  return normalized
}

const enabledServerCount = computed(() => form.mcp.global.servers.filter(server => server.enabled).length)

const formatTransport = (type) => {
  if (type === 'sse') return 'SSE'
  if (type === 'streamablehttp') return 'StreamableHTTP'
  return type || '-'
}

const resetServerForm = () => {
  serverForm.name = ''
  serverForm.type = 'streamablehttp'
  serverForm.url = ''
  serverForm.enabled = true
}

const openCreateServerDialog = () => {
  editingServerIndex.value = null
  resetServerForm()
  serverDialogVisible.value = true
}

const openEditServerDialog = (row, index) => {
  editingServerIndex.value = index
  serverForm.name = row.name || ''
  serverForm.type = row.type || 'streamablehttp'
  serverForm.url = row.url || ''
  serverForm.enabled = row.enabled !== false
  serverDialogVisible.value = true
}

const saveServerDialog = async () => {
  if (!serverFormRef.value) return
  const valid = await serverFormRef.value.validate().catch(() => false)
  if (!valid) return

  serverSaving.value = true
  try {
    if (editingServerIndex.value === null) {
      const server = createGlobalServer()
      server.name = serverForm.name.trim()
      server.type = serverForm.type
      server.url = serverForm.url.trim()
      server.enabled = serverForm.enabled
      form.mcp.global.servers.push(server)
    } else {
      const server = form.mcp.global.servers[editingServerIndex.value]
      if (!server) return
      const urlChanged =
        server.url.trim() !== serverForm.url.trim() || server.type !== serverForm.type
      server.name = serverForm.name.trim()
      server.type = serverForm.type
      server.url = serverForm.url.trim()
      server.enabled = serverForm.enabled
      if (urlChanged) {
        resetProbeMeta(server)
      }
    }
    serverDialogVisible.value = false
    ElMessage.success(editingServerIndex.value === null ? '已添加服务器' : '已更新服务器')
  } finally {
    serverSaving.value = false
  }
}

const openServerToolsDialog = (row) => {
  toolsTargetServer.value = row
  toolsSelected.value = Array.isArray(row.allowed_tools) ? [...row.allowed_tools] : []
  toolsMode.value = toolsSelected.value.length > 0 ? 'selected' : 'all'
  toolsDialogVisible.value = true
  if (!row._tool_options?.length && row.url) {
    discoverGlobalServerTools(row)
  }
}

const handleToolsModeChange = (mode) => {
  if (mode === 'all') {
    toolsSelected.value = []
  }
}

const saveServerTools = () => {
  if (!toolsTargetServer.value) return
  toolsTargetServer.value.allowed_tools =
    toolsMode.value === 'selected' ? [...toolsSelected.value] : []
  toolsDialogVisible.value = false
  ElMessage.success('工具范围已更新，记得保存配置')
}

const confirmRemoveServer = async (index, row) => {
  try {
    await ElMessageBox.confirm(
      `确认删除服务器「${row.name || '未命名'}」？`,
      '删除确认',
      { type: 'warning', confirmButtonText: '删除', cancelButtonText: '取消' }
    )
    form.mcp.global.servers.splice(index, 1)
    ElMessage.success('已删除')
  } catch {
    // cancelled
  }
}

const resetForm = () => {
  const defaults = createDefaultState()
  form.mcp.global.enabled = defaults.mcp.global.enabled
  form.mcp.global.reconnect_interval = defaults.mcp.global.reconnect_interval
  form.mcp.global.max_reconnect_attempts = defaults.mcp.global.max_reconnect_attempts
  form.mcp.global.servers = defaults.mcp.global.servers
  form.local_mcp.exit_conversation = defaults.local_mcp.exit_conversation
  form.local_mcp.clear_conversation_history = defaults.local_mcp.clear_conversation_history
  form.local_mcp.play_music = defaults.local_mcp.play_music
}

const sanitizeGlobalServers = () => {
  return form.mcp.global.servers.map((server) => {
    const sanitized = { ...server }
    delete sanitized._tool_options
    delete sanitized._tools_loading
    return sanitized
  })
}

const generateConfig = () => {
  return JSON.stringify({
    mcp: {
      global: {
        ...form.mcp.global,
        servers: sanitizeGlobalServers()
      }
    },
    local_mcp: { ...form.local_mcp }
  })
}

const discoverGlobalServerTools = async (server, { silent = false } = {}) => {
  if (!server?.url) {
    if (!silent) ElMessage.warning('请先填写服务器 URL')
    return false
  }

  server._tools_loading = true
  try {
    const response = await api.post('/admin/mcp-configs/discover-tools', {
      transport: server.type,
      url: server.url,
      headers: server.headers || null
    })
    mergeServerToolOptions(server, response.data?.data?.tools || [])
    const toolCount = server._tool_options.length
    markProbeSuccess(server, toolCount)
    if (!silent) ElMessage.success(`探测到 ${toolCount} 个工具`)
    return true
  } catch (error) {
    mergeServerToolOptions(server)
    const message = error.response?.data?.error || '探测工具失败'
    markProbeFailure(server, message)
    if (!silent) {
      if ((Number(server.probe_fail_count) || 0) < MAX_PROBE_FAILURES) {
        ElMessage.error(message)
      }
    }
    return false
  } finally {
    server._tools_loading = false
  }
}

const loadConfig = async () => {
  loading.value = true
  try {
    const response = await api.get('/admin/mcp-configs')
    const configs = response.data?.data || []

    resetForm()

    if (configs.length > 0) {
      const config = configs.find(item => item.is_default) || configs[0]
      configId.value = config.id

      try {
        const configData = JSON.parse(config.json_data || '{}')
        if (configData.global && !configData.mcp) {
          form.mcp.global = {
            ...form.mcp.global,
            ...configData.global,
            servers: Array.isArray(configData.global?.servers)
              ? configData.global.servers.map(normalizeGlobalServer)
              : []
          }
        } else if (configData.mcp?.global) {
          form.mcp.global = {
            ...form.mcp.global,
            ...configData.mcp.global,
            servers: Array.isArray(configData.mcp.global?.servers)
              ? configData.mcp.global.servers.map(normalizeGlobalServer)
              : []
          }
        }

        if (configData.local_mcp) {
          Object.assign(form.local_mcp, configData.local_mcp)
        }
      } catch (error) {
        ElMessage.warning('MCP 配置格式异常，已回退到默认值')
      }
    } else {
      configId.value = null
    }
  } catch (error) {
    ElMessage.error('加载 MCP 配置失败')
  } finally {
    loading.value = false
  }
}

const handleTestConnectivity = async () => {
  testing.value = true
  try {
    const result = await testWithData('mcp', {
      mcp_global_config: {
        config: JSON.parse(generateConfig())
      }
    })
    testResult.value = result
    syncProbeStatusFromConnectivityTest(result)

    if (result.ok) {
      ElMessage.success(formatTestMessage(result))
    } else {
      ElMessage.warning(formatTestMessage(result))
    }
  } catch (error) {
    const message = error.response?.data?.error || '测试请求失败'
    testResult.value = { ok: false, message }
    ElMessage.error(message)
  } finally {
    testing.value = false
  }
}

const handleSave = async () => {
  if (!formRef.value) return

  try {
    await formRef.value.validate()
  } catch {
    return
  }

  saving.value = true
  try {
    const payload = {
      name: 'MCP全局配置',
      config_id: 'mcp_global_config',
      is_default: true,
      json_data: generateConfig()
    }

    if (configId.value) {
      await api.put(`/admin/mcp-configs/${configId.value}`, payload)
      ElMessage.success('MCP 配置已更新')
    } else {
      const response = await api.post('/admin/mcp-configs', payload)
      configId.value = response.data?.data?.id || configId.value
      ElMessage.success('MCP 配置已保存')
    }

    await loadConfig()
  } catch (error) {
    ElMessage.error(error.response?.data?.message || '保存 MCP 配置失败')
  } finally {
    saving.value = false
  }
}

onMounted(() => {
  loadConfig()
})
</script>

<style scoped>
.mcp-config {
  padding: 0 24px 32px;
}

.config-form {
  display: grid;
  gap: 24px;
}

.config-layout {
  display: grid;
  gap: 24px;
}

.config-card {
  border: 1px solid rgba(255, 255, 255, 0.88);
  background: rgba(255, 255, 255, 0.88);
  box-shadow: var(--apple-shadow-md);
}

.card-head {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: 16px;
}

.card-head-tags {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  justify-content: flex-end;
}

.card-kicker {
  display: block;
  margin: 0;
  font-size: 12px;
  font-weight: 700;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: var(--apple-text-tertiary);
}

.card-head h3 {
  margin: 8px 0 0;
  font-size: 22px;
  line-height: 1.15;
  letter-spacing: -0.03em;
  color: var(--apple-text);
}

.card-description,
.field-help,
.footer-note,
.server-list-header p {
  margin: 8px 0 0;
  font-size: 13px;
  line-height: 1.7;
  color: var(--apple-text-secondary);
}

.text-muted {
  color: var(--apple-text-tertiary);
}

.inline-help {
  margin-left: 12px;
  font-size: 12px;
  color: var(--apple-text-secondary);
}

.field-grid {
  display: grid;
  gap: 20px 18px;
}

.field-grid-main {
  grid-template-columns: repeat(3, minmax(0, 1fr));
}

.local-mcp-section {
  margin-top: 24px;
  padding-top: 24px;
  border-top: 1px solid rgba(229, 229, 234, 0.72);
}

.section-head h4,
.server-list-header h4 {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
  color: var(--apple-text);
}

.section-head p,
.server-list-header p {
  margin: 6px 0 0;
}

.local-mcp-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 16px 20px;
  margin-top: 16px;
}

.local-mcp-item :deep(.el-form-item__label) {
  display: none;
}

.switch-field.compact {
  padding: 14px 16px;
  border-radius: 12px;
  border: 1px solid rgba(229, 229, 234, 0.88);
  background: rgba(248, 250, 252, 0.82);
}

.field-span-full {
  grid-column: 1 / -1;
}

.switch-field {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: 8px 18px;
  align-items: center;
}

.switch-title {
  font-size: 15px;
  font-weight: 600;
  color: var(--apple-text);
}

.server-list {
  margin-top: 24px;
  padding-top: 24px;
  border-top: 1px solid rgba(229, 229, 234, 0.72);
}

.server-list-header {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: 16px;
  margin-bottom: 18px;
}

.server-table {
  width: 100%;
  border-radius: 12px;
  overflow: hidden;
}

.tools-dialog-head {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: 16px;
  margin-bottom: 16px;
}

.tools-dialog-head strong {
  font-size: 15px;
  color: var(--apple-text);
}

.tools-mode {
  margin-bottom: 16px;
}

.tools-all-hint {
  margin: 0;
  padding: 12px 14px;
  border-radius: 10px;
  background: rgba(248, 250, 252, 0.9);
}

.tool-option-row {
  display: flex;
  flex-direction: column;
  gap: 2px;
  line-height: 1.35;
}

.tool-option-name {
  color: var(--apple-text);
}

.tool-option-desc {
  color: var(--apple-text-secondary);
  font-size: 12px;
}

.footer-bar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 16px;
  padding: 0 4px;
}

.footer-note {
  max-width: 680px;
  margin: 0;
}

.footer-actions {
  display: flex;
  justify-content: flex-end;
  flex-wrap: wrap;
  gap: 12px;
}

:deep(.el-card__header) {
  padding: 24px 24px 0;
  border-bottom: none;
  background: transparent;
}

:deep(.el-card__body) {
  padding: 24px;
}

:deep(.el-form-item) {
  margin-bottom: 0;
}

:deep(.el-form-item__label) {
  font-size: 14px;
  font-weight: 600;
  color: var(--apple-text);
}

@media (max-width: 1180px) {
  .field-grid-main,
  .local-mcp-grid {
    grid-template-columns: 1fr;
  }
}

@media (max-width: 768px) {
  .mcp-config {
    padding: 0 16px 24px;
  }

  :deep(.el-card__body) {
    padding: 20px;
  }

  :deep(.el-card__header) {
    padding: 20px 20px 0;
  }

  .server-list-header,
  .footer-bar {
    flex-direction: column;
    align-items: stretch;
  }

  .tools-dialog-head {
    flex-direction: column;
  }

  .footer-actions {
    justify-content: stretch;
  }

  .footer-actions :deep(.el-button) {
    flex: 1;
  }
}
</style>
