<template>
  <div class="mcp-market-page">
    <el-tabs v-model="activeTab" class="market-tabs">
      <el-tab-pane name="discover">
        <template #label>
          <span>市场发现</span>
        </template>

        <el-row :gutter="16">
          <el-col :xs="24" :lg="11">
            <el-card shadow="never" class="panel-card">
              <template #header>
                <div class="panel-header">
                  <span>MCP市场</span>
                  <div>
                    <el-button type="primary" size="small" @click="openCreateDialog">新增连接</el-button>
                    <el-button size="small" @click="loadMarkets">
                      <el-icon><Refresh /></el-icon>
                    </el-button>
                  </div>
                </div>
              </template>

              <el-table :data="markets" stripe v-loading="marketsLoading" height="560">
                <el-table-column prop="name" label="名称" min-width="140" />
                <el-table-column prop="provider_id" label="提供商" width="130">
                  <template #default="{ row }">
                    <el-tag size="small">{{ row.provider_id || 'generic' }}</el-tag>
                  </template>
                </el-table-column>
                <el-table-column prop="catalog_url" label="目录URL" min-width="220" show-overflow-tooltip />
                <el-table-column label="鉴权" width="120">
                  <template #default="{ row }">
                    <el-tag size="small" :type="row.has_token ? 'success' : 'info'">
                      {{ row.auth_type || 'none' }}
                    </el-tag>
                  </template>
                </el-table-column>
                <el-table-column label="状态" width="90">
                  <template #default="{ row }">
                    <el-tag size="small" :type="row.enabled ? 'success' : 'info'">
                      {{ row.enabled ? '启用' : '禁用' }}
                    </el-tag>
                  </template>
                </el-table-column>
                <el-table-column label="操作" width="96" fixed="right">
                  <template #default="{ row }">
                    <el-dropdown trigger="click" @command="(cmd) => handleMarketAction(cmd, row)">
                      <el-button link type="primary" class="market-action-btn">
                        <el-icon><MoreFilled /></el-icon>
                      </el-button>
                      <template #dropdown>
                        <el-dropdown-menu>
                          <el-dropdown-item command="edit">编辑</el-dropdown-item>
                          <el-dropdown-item command="test">测试</el-dropdown-item>
                          <el-dropdown-item command="delete" divided>删除</el-dropdown-item>
                        </el-dropdown-menu>
                      </template>
                    </el-dropdown>
                  </template>
                </el-table-column>
              </el-table>
            </el-card>
          </el-col>

          <el-col :xs="24" :lg="13">
            <el-card shadow="never" class="panel-card">
              <template #header>
                <div class="panel-header">
                  <span>聚合服务列表</span>
                  <div class="search-actions">
                    <el-input
                      v-model="serviceQuery"
                      placeholder="搜索服务名/描述/ID"
                      clearable
                      size="small"
                      style="width: 240px"
                      @keyup.enter="loadServices(1)"
                    >
                      <template #append>
                        <el-button @click="loadServices(1)">
                          <el-icon><Search /></el-icon>
                        </el-button>
                      </template>
                    </el-input>
                    <el-button size="small" @click="loadServices(servicePage)">
                      <el-icon><Refresh /></el-icon>
                    </el-button>
                  </div>
                </div>
              </template>

              <el-table :data="services" stripe v-loading="servicesLoading" height="500">
                <template #empty>
                  <el-empty :description="serviceEmptyHint" />
                </template>
                <el-table-column prop="name" label="服务" min-width="180" show-overflow-tooltip />
                <el-table-column prop="market_name" label="来源市场" min-width="120" show-overflow-tooltip />
                <el-table-column prop="service_id" label="Service ID" min-width="180" show-overflow-tooltip />
                <el-table-column label="操作" width="90" fixed="right">
                  <template #default="{ row }">
                    <el-button link type="primary" @click.stop="loadServiceDetail(row)">详情</el-button>
                  </template>
                </el-table-column>
              </el-table>

              <div class="pagination-wrap">
                <el-pagination
                  layout="prev, pager, next, total"
                  :current-page="servicePage"
                  :page-size="servicePageSize"
                  :total="serviceTotal"
                  @current-change="loadServices"
                />
              </div>

              <el-alert
                v-if="serviceWarnings.length > 0"
                type="warning"
                :closable="false"
                title="部分市场拉取失败"
                class="warning-alert"
              >
                <template #default>
                  <div v-for="(warn, idx) in serviceWarnings" :key="idx">{{ warn }}</div>
                </template>
              </el-alert>
            </el-card>
          </el-col>
        </el-row>
      </el-tab-pane>

      <el-tab-pane name="imported">
        <template #label>
          <div class="tab-label-with-badge">
            <span>已导入服务</span>
            <el-badge :value="importedTotal" :max="999" class="tab-badge" />
          </div>
        </template>

        <el-card shadow="never" class="panel-card">
          <template #header>
            <div class="panel-header">
              <span>已导入服务</span>
              <div class="search-actions">
                <el-input
                  v-model="importedQuery"
                  placeholder="搜索名称 / service_id / URL"
                  clearable
                  size="small"
                  style="width: 320px"
                  @keyup.enter="loadImportedItems(1)"
                >
                  <template #append>
                    <el-button @click="loadImportedItems(1)">
                      <el-icon><Search /></el-icon>
                    </el-button>
                  </template>
                </el-input>
                <el-button size="small" @click="loadImportedItems(importedPage)">
                  <el-icon><Refresh /></el-icon>
                </el-button>
                <el-button type="primary" size="small" @click="openCreateImportedDialog">新增服务</el-button>
              </div>
            </div>
          </template>

          <el-table :data="importedItems" stripe v-loading="importedLoading" height="560">
            <el-table-column prop="name" label="名称" min-width="160" show-overflow-tooltip />
            <el-table-column prop="transport" label="传输" width="140" />
            <el-table-column prop="url" label="URL" min-width="320" show-overflow-tooltip />
            <el-table-column prop="service_id" label="Service ID" min-width="180" show-overflow-tooltip />
            <el-table-column label="工具" width="120">
              <template #default="{ row }">
                <el-tag size="small" :type="row.allowed_tools?.length ? 'warning' : 'info'">
                  {{ row.allowed_tools?.length ? `${row.allowed_tools.length}个已选` : '全部工具' }}
                </el-tag>
              </template>
            </el-table-column>
            <el-table-column prop="provider_id" label="提供商" width="120">
              <template #default="{ row }">
                <el-tag size="small">{{ row.provider_id || '-' }}</el-tag>
              </template>
            </el-table-column>
            <el-table-column prop="enabled" label="启用" width="90">
              <template #default="{ row }">
                <el-tag size="small" :type="row.enabled ? 'success' : 'info'">
                  {{ row.enabled ? '启用' : '禁用' }}
                </el-tag>
              </template>
            </el-table-column>
            <el-table-column prop="updated_at" label="更新时间" width="180" />
            <el-table-column label="操作" width="280" fixed="right">
              <template #default="{ row }">
                <el-button link type="primary" @click="openEditImportedDialog(row)">编辑</el-button>
                <el-button link type="primary" @click="openImportedToolsDialog(row)">工具选择</el-button>
                <el-button link :type="row.enabled ? 'warning' : 'success'" @click="toggleImportedEnabled(row)">
                  {{ row.enabled ? '禁用' : '启用' }}
                </el-button>
                <el-button link type="danger" @click="deleteImportedItem(row)">删除</el-button>
              </template>
            </el-table-column>
          </el-table>

          <div class="pagination-wrap">
            <el-pagination
              layout="prev, pager, next, total"
              :current-page="importedPage"
              :page-size="importedPageSize"
              :total="importedTotal"
              @current-change="loadImportedItems"
            />
          </div>
        </el-card>
      </el-tab-pane>
    </el-tabs>

    <el-dialog v-model="detailDialogVisible" title="服务详情" width="900px">
      <div v-loading="detailLoading">
        <el-empty v-if="!serviceDetail && !detailLoading" description="暂无服务详情" />
        <template v-else-if="serviceDetail">
          <div class="detail-grid">
            <div><strong>服务：</strong>{{ serviceDetail.name || '-' }}</div>
            <div><strong>来源市场：</strong>{{ serviceDetail.market_name || '-' }}</div>
            <div><strong>Service ID：</strong>{{ serviceDetail.service_id || '-' }}</div>
          </div>
          <div v-if="serviceDetail.description" class="detail-desc">{{ serviceDetail.description }}</div>
          <div v-if="serviceDetail.source_url" class="detail-desc">
            <strong>服务商文档：</strong>
            <el-link :href="serviceDetail.source_url" target="_blank" type="primary">
              {{ serviceDetail.source_url }}
            </el-link>
          </div>
          <el-alert
            v-if="(serviceDetail.endpoints || []).length > 1"
            type="info"
            :closable="false"
            show-icon
            title="该服务有多个接入地址，请选择要导入的一条（已按官方地址优先排序）"
            class="endpoint-pick-alert"
          />
          <el-table
            :data="serviceDetail.endpoints || []"
            size="small"
            stripe
            highlight-current-row
            class="endpoint-pick-table"
            @row-click="selectDetailEndpoint"
          >
            <el-table-column label="选择" width="64" align="center">
              <template #default="{ row }">
                <el-radio v-model="selectedEndpointUrl" :value="row.url" @click.stop />
              </template>
            </el-table-column>
            <el-table-column prop="name" label="资源名" min-width="100" show-overflow-tooltip />
            <el-table-column prop="source" label="来源" width="130" show-overflow-tooltip>
              <template #default="{ row }">
                <el-tag size="small" effect="plain">{{ formatEndpointSource(row.source) }}</el-tag>
              </template>
            </el-table-column>
            <el-table-column prop="transport" label="传输" width="120" />
            <el-table-column label="鉴权" width="100">
              <template #default="{ row }">
                <el-tag size="small" :type="row.auth_required === false ? 'warning' : 'info'" effect="plain">
                  {{ row.auth_required === false ? '服务商' : '市场' }}
                </el-tag>
              </template>
            </el-table-column>
            <el-table-column prop="url" label="URL" min-width="280" show-overflow-tooltip />
          </el-table>
        </template>
      </div>
      <template #footer>
        <el-button @click="detailDialogVisible = false">关闭</el-button>
        <el-button type="primary" :loading="detailImporting" :disabled="!canImportDetail" @click="importFromDetail">
          导入选中接入地址
        </el-button>
      </template>
    </el-dialog>

    <el-dialog v-model="importedDialogVisible" :title="editingImported ? '编辑导入服务' : '新增导入服务'" width="700px">
      <el-form ref="importedFormRef" :model="importedForm" :rules="importedRules" label-width="120px">
        <el-form-item label="名称" prop="name">
          <el-input v-model="importedForm.name" placeholder="服务展示名称" />
        </el-form-item>
        <el-form-item label="启用">
          <el-switch v-model="importedForm.enabled" />
        </el-form-item>
        <el-form-item label="传输" prop="transport">
          <el-select v-model="importedForm.transport" style="width: 100%">
            <el-option label="SSE" value="sse" />
            <el-option label="StreamableHTTP" value="streamablehttp" />
          </el-select>
        </el-form-item>
        <el-form-item label="URL" prop="url">
          <el-input v-model="importedForm.url" placeholder="MCP 接入地址（以服务商文档或市场返回为准）" />
        </el-form-item>
        <el-form-item v-if="importedForm.source_url" label="服务商文档">
          <el-link :href="importedForm.source_url" target="_blank" type="primary">
            {{ importedForm.source_url }}
          </el-link>
        </el-form-item>
        <el-alert
          v-if="importedEndpointHint"
          :title="importedEndpointHint"
          type="warning"
          :closable="false"
          show-icon
          style="margin-bottom: 12px"
        />
        <el-form-item label="来源市场">
          <el-select v-model="importedForm.market_id" clearable filterable style="width: 100%" placeholder="可选">
            <el-option v-for="item in markets" :key="item.id" :label="item.name" :value="item.id" />
          </el-select>
        </el-form-item>
        <el-form-item label="提供商">
          <el-input v-model="importedForm.provider_id" placeholder="例如：modelscope" />
        </el-form-item>
        <el-form-item label="Service ID">
          <el-input v-model="importedForm.service_id" placeholder="上游服务ID（可选）" />
        </el-form-item>
        <el-form-item label="服务名称">
          <el-input v-model="importedForm.service_name" placeholder="上游服务名（可选）" />
        </el-form-item>
        <el-divider content-position="left">鉴权</el-divider>
        <el-form-item label="Token">
          <el-input
            v-model="importedAuthToken"
            show-password
            clearable
            placeholder="服务商 Token 或 Bearer Token（填写后自动写入 Headers）"
            @input="handleImportedTokenInput"
          />
          <div class="field-help">自动同步到 Headers 的 <code>Authorization: Bearer ...</code>，也可在下方 Headers 中手动修改。</div>
        </el-form-item>
        <el-form-item label="Headers(JSON)">
          <el-input
            v-model="importedHeadersText"
            type="textarea"
            :rows="4"
            placeholder='例如：{"Authorization":"Bearer xxx"}，可添加其他自定义 Header'
            @blur="handleImportedHeadersBlur"
          />
        </el-form-item>
      </el-form>

      <template #footer>
        <el-button @click="importedDialogVisible = false">取消</el-button>
        <el-button type="primary" :loading="importedSaving" @click="saveImportedItem">保存</el-button>
      </template>
    </el-dialog>

    <el-dialog v-model="importedToolsDialogVisible" :title="toolDialogTitle" width="760px">
      <el-form label-width="100px" class="tool-auth-form">
        <el-divider content-position="left">鉴权（探测工具时使用）</el-divider>
        <el-form-item label="Token">
          <el-input
            v-model="importedToolsAuthToken"
            show-password
            clearable
            placeholder="填写后自动写入 Headers，探测前会先保存"
            @input="handleImportedToolsTokenInput"
          />
        </el-form-item>
        <el-form-item label="Headers">
          <el-input
            v-model="importedToolsHeadersText"
            type="textarea"
            :rows="3"
            placeholder='{"Authorization":"Bearer xxx"}'
            @blur="handleImportedToolsHeadersBlur"
          />
          <div class="field-help">Token 与 Headers 可任选其一填写；修改 Headers 后失焦会回读 Token。</div>
        </el-form-item>
      </el-form>

      <div class="tool-selector-card">
        <div class="tool-selector-header">
          <div class="tool-selector-meta">
            <span class="tool-selector-title">工具访问策略</span>
            <span class="tool-selector-tip">空列表表示允许该服务全部工具。</span>
          </div>
          <div class="tool-selector-actions">
            <el-tag size="small" :type="importedToolMode === 'all' ? 'info' : 'warning'">
              {{ importedToolMode === 'all' ? '全部工具' : `已选 ${importedSelectedTools.length} 项` }}
            </el-tag>
            <el-button size="small" :loading="importedToolsLoading" @click="refreshImportedTools">
              探测工具
            </el-button>
          </div>
        </div>

        <el-radio-group v-model="importedToolMode" size="small" class="tool-mode-group" @change="handleImportedToolModeChange">
          <el-radio-button label="all">全部工具</el-radio-button>
          <el-radio-button label="selected">指定工具</el-radio-button>
        </el-radio-group>

        <template v-if="importedToolMode === 'selected'">
          <div class="tool-picker-search">
            <el-input v-model="importedToolQuery" clearable placeholder="搜索工具名或描述">
              <template #prefix>
                <el-icon><Search /></el-icon>
              </template>
            </el-input>
          </div>

          <div v-if="filteredImportedToolOptions.length === 0" class="tool-picker-empty">
            {{ importedToolOptions.length === 0 ? '还没有探测到工具，请先点击“探测工具”。' : '未匹配到可选工具。' }}
          </div>
          <el-checkbox-group v-else v-model="importedSelectedTools" class="tool-grid">
            <el-checkbox
              v-for="tool in filteredImportedToolOptions"
              :key="tool.name"
              :label="tool.name"
              border
              class="tool-tile"
            >
              <div class="tool-tile-body">
                <span class="tool-tile-name">{{ tool.name }}</span>
                <span class="tool-tile-desc">{{ tool.description || '无描述' }}</span>
              </div>
            </el-checkbox>
          </el-checkbox-group>
        </template>
      </div>

      <template #footer>
        <el-button @click="importedToolsDialogVisible = false">取消</el-button>
        <el-button type="primary" :loading="importedSaving" @click="saveImportedToolSelection">保存</el-button>
      </template>
    </el-dialog>

    <el-dialog v-model="marketDialogVisible" :title="editingMarket ? '编辑MCP市场' : '新增MCP市场'" width="640px">
      <el-form ref="marketFormRef" :model="marketForm" :rules="marketRules" label-width="130px">
        <el-form-item label="提供商">
          <el-select v-model="marketForm.provider_id" style="width: 100%" @change="handleProviderChange">
            <el-option v-for="provider in selectableProviderOptions" :key="provider.id" :label="provider.name" :value="provider.id" />
          </el-select>
          <div v-if="currentProvider?.description" class="provider-desc">{{ currentProvider.description }}</div>
        </el-form-item>
        <el-form-item label="名称" prop="name">
          <el-input v-model="marketForm.name" placeholder="例如：魔搭MCP市场" />
        </el-form-item>
        <el-form-item label="目录URL" prop="catalog_url">
          <el-input v-model="marketForm.catalog_url" placeholder="https://example.com/api/services" />
        </el-form-item>
        <el-form-item label="详情URL模板" prop="detail_url_template">
          <el-input v-model="marketForm.detail_url_template" placeholder="https://example.com/api/services/{id}（可选）" />
        </el-form-item>
        <el-form-item label="启用">
          <el-switch v-model="marketForm.enabled" />
        </el-form-item>

        <el-divider>鉴权配置</el-divider>
        <el-form-item label="Token">
          <el-input
            v-model="marketForm.auth.token"
            :placeholder="editingMarket ? `留空则保持原值（当前：${editingMarket.token_mask || '未设置'}）` : '请输入魔搭 Token'"
            show-password
            clearable
          />
        </el-form-item>
      </el-form>

      <template #footer>
        <el-button @click="marketDialogVisible = false">取消</el-button>
        <el-button type="primary" :loading="marketSaving" @click="saveMarket">保存</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup>
import { ref, reactive, computed, onMounted } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Refresh, Search, MoreFilled } from '@element-plus/icons-vue'
import api from '@/utils/api'
import {
  buildHeadersFromAuth,
  extractTokenFromHeaders,
  loadAuthFieldsFromHeaders,
  parseHeadersText,
  syncHeadersTextFromToken
} from '@/utils/mcpAuthHeaders'

const activeTab = ref('discover')

const markets = ref([])
const marketsLoading = ref(false)
const providerOptions = ref([])
const marketDialogVisible = ref(false)
const marketSaving = ref(false)
const editingMarket = ref(null)
const marketFormRef = ref()

const marketForm = reactive({
  name: '',
  provider_id: 'modelscope',
  catalog_url: '',
  detail_url_template: '',
  enabled: true,
  auth: {
    type: 'bearer',
    token: '',
    header_name: 'Authorization'
  }
})

const marketRules = {
  name: [{ required: true, message: '请输入名称', trigger: 'blur' }],
  catalog_url: [{ required: true, message: '请输入目录URL', trigger: 'blur' }]
}

const selectableProviderOptions = computed(() => {
  return providerOptions.value.filter(item => item.id !== 'generic')
})

const currentProvider = computed(() => {
  return selectableProviderOptions.value.find(item => item.id === marketForm.provider_id) || null
})

const services = ref([])
const servicesLoading = ref(false)
const serviceWarnings = ref([])
const servicePage = ref(1)
const servicePageSize = ref(20)
const serviceTotal = ref(0)
const serviceQuery = ref('')
const detailDialogVisible = ref(false)
const detailLoading = ref(false)
const detailImporting = ref(false)
const serviceDetail = ref(null)
const selectedEndpointUrl = ref('')

const canImportDetail = computed(() => {
  if (!serviceDetail.value) return false
  const endpoints = serviceDetail.value.endpoints || []
  if (endpoints.length === 0) return false
  if (endpoints.length === 1) return true
  return !!selectedEndpointUrl.value
})

const importedLoading = ref(false)
const importedSaving = ref(false)
const importedDialogVisible = ref(false)
const importedToolsDialogVisible = ref(false)
const importedToolsLoading = ref(false)
const editingImported = ref(null)
const importedToolTarget = ref(null)
const importedFormRef = ref()
const importedItems = ref([])
const importedToolOptions = ref([])
const importedPage = ref(1)
const importedPageSize = ref(20)
const importedTotal = ref(0)
const importedQuery = ref('')
const importedHeadersText = ref('')
const importedAuthToken = ref('')
const importedToolsAuthToken = ref('')
const importedToolsHeadersText = ref('')
const importedToolMode = ref('all')
const importedToolQuery = ref('')
const importedSelectedTools = ref([])

const importedForm = reactive({
  name: '',
  enabled: true,
  transport: 'streamablehttp',
  url: '',
  allowed_tools: [],
  market_id: null,
  provider_id: '',
  service_id: '',
  service_name: '',
  source_url: '',
  auth_required: true
})

const isModelscopeInferenceUrl = (url) =>
  String(url || '').toLowerCase().includes('mcp.api-inference.modelscope.net')

const importedEndpointHint = computed(() => {
  if (importedForm.auth_required === false && isModelscopeInferenceUrl(importedForm.url)) {
    if (importedForm.source_url) {
      return `auth_required=false：魔搭托管地址可能不接受服务商 Token，请查阅服务商文档确认官方 MCP 接入地址`
    }
    return 'auth_required=false：当前为魔搭托管地址，若探测失败请查阅服务商文档，改用官方 MCP 地址并填写服务商 Token'
  }
  if (importedForm.auth_required === false) {
    return '该服务需使用服务商 Token（Headers 中的 Authorization），不是魔搭 ms- Token'
  }
  return ''
})

const importedRules = {
  name: [{ required: true, message: '请输入名称', trigger: 'blur' }],
  transport: [{ required: true, message: '请选择传输类型', trigger: 'change' }],
  url: [{ required: true, message: '请输入URL', trigger: 'blur' }]
}

const toolDialogTitle = computed(() => {
  return importedToolTarget.value ? `工具选择 · ${importedToolTarget.value.name}` : '工具选择'
})

const filteredImportedToolOptions = computed(() => {
  const query = importedToolQuery.value.trim().toLowerCase()
  if (!query) return importedToolOptions.value
  return importedToolOptions.value.filter((tool) => {
    return tool.name.toLowerCase().includes(query) || (tool.description || '').toLowerCase().includes(query)
  })
})

const serviceEmptyHint = computed(() => {
  if (serviceWarnings.value.length > 0) {
    return serviceWarnings.value.join('；')
  }
  if (markets.value.length === 0) {
    return '请先在左侧新增并启用 MCP 市场连接'
  }
  return '暂无服务：魔搭需填写 Token，并在 modelscope.cn 激活 MCP 服务后刷新'
})

const getDefaultProviderId = () => {
  if (selectableProviderOptions.value.length === 0) return 'modelscope'
  if (selectableProviderOptions.value.some(item => item.id === 'modelscope')) return 'modelscope'
  return selectableProviderOptions.value[0].id
}

const loadProviders = async () => {
  try {
    const resp = await api.get('/admin/mcp-market/providers')
    providerOptions.value = resp.data.data || []
    if (!marketForm.provider_id) {
      marketForm.provider_id = getDefaultProviderId()
    }
    if (!selectableProviderOptions.value.some(item => item.id === marketForm.provider_id)) {
      marketForm.provider_id = getDefaultProviderId()
    }
  } catch (error) {
    providerOptions.value = [{ id: 'modelscope', name: '魔搭 ModelScope' }]
    marketForm.provider_id = marketForm.provider_id || 'modelscope'
    ElMessage.error(error.response?.data?.error || '加载提供商失败')
  }
}

const applyProviderPreset = (providerId, force = false) => {
  const provider = selectableProviderOptions.value.find(item => item.id === providerId)
  if (!provider) return

  if (force || !marketForm.catalog_url) {
    marketForm.catalog_url = provider.catalog_url || ''
  }
  if (force || !marketForm.detail_url_template) {
    marketForm.detail_url_template = provider.detail_url_template || ''
  }
  if (force || !marketForm.auth.type) {
    marketForm.auth.type = 'bearer'
  }
  marketForm.auth.header_name = 'Authorization'

  if (force) {
    marketForm.auth.token = ''
  }

  if (!editingMarket.value && (force || !marketForm.name) && provider.id === 'modelscope') {
    marketForm.name = '魔搭MCP市场'
  }
}

const handleProviderChange = (providerId) => {
  applyProviderPreset(providerId, true)
}

const handleMarketAction = async (command, row) => {
  if (command === 'edit') {
    openEditDialog(row)
    return
  }
  if (command === 'test') {
    await testMarket(row)
    return
  }
  if (command === 'delete') {
    await deleteMarket(row)
  }
}

const loadMarkets = async () => {
  marketsLoading.value = true
  try {
    const resp = await api.get('/admin/mcp-markets')
    markets.value = resp.data.data || []
  } catch (error) {
    ElMessage.error(error.response?.data?.error || '加载MCP市场失败')
  } finally {
    marketsLoading.value = false
  }
}

const resetMarketForm = () => {
  marketForm.name = ''
  marketForm.provider_id = getDefaultProviderId()
  marketForm.catalog_url = ''
  marketForm.detail_url_template = ''
  marketForm.enabled = true
  marketForm.auth.type = 'bearer'
  marketForm.auth.token = ''
  marketForm.auth.header_name = 'Authorization'
}

const openCreateDialog = () => {
  editingMarket.value = null
  resetMarketForm()
  applyProviderPreset(marketForm.provider_id, true)
  marketDialogVisible.value = true
}

const openEditDialog = (row) => {
  editingMarket.value = row
  marketForm.name = row.name
  const rowProviderId = row.provider_id || getDefaultProviderId()
  marketForm.provider_id = selectableProviderOptions.value.some(item => item.id === rowProviderId)
    ? rowProviderId
    : getDefaultProviderId()
  marketForm.catalog_url = row.catalog_url
  marketForm.detail_url_template = row.detail_url_template || ''
  marketForm.enabled = !!row.enabled
  marketForm.auth.type = 'bearer'
  marketForm.auth.header_name = 'Authorization'
  marketForm.auth.token = ''
  marketDialogVisible.value = true
}

const saveMarket = async () => {
  if (!marketFormRef.value) return
  const valid = await marketFormRef.value.validate().catch(() => false)
  if (!valid) return

  const payload = {
    name: marketForm.name,
    provider_id: marketForm.provider_id,
    catalog_url: marketForm.catalog_url,
    detail_url_template: marketForm.detail_url_template,
    enabled: marketForm.enabled,
    auth: {
      type: 'bearer',
      token: marketForm.auth.token,
      header_name: 'Authorization'
    }
  }

  marketSaving.value = true
  try {
    if (editingMarket.value) {
      await api.put(`/admin/mcp-markets/${editingMarket.value.id}`, payload)
      ElMessage.success('更新成功')
    } else {
      await api.post('/admin/mcp-markets', payload)
      ElMessage.success('创建成功')
    }
    marketDialogVisible.value = false
    await loadMarkets()
    await loadServices(1)
  } catch (error) {
    const msg = error.response?.data?.error
      || error.response?.data?.message
      || (error.message === 'Network Error' ? '无法连接管理后台，请确认 xiaozhi-manager 已启动' : '')
      || '保存失败'
    ElMessage.error(msg)
  } finally {
    marketSaving.value = false
  }
}

const deleteMarket = async (row) => {
  try {
    await ElMessageBox.confirm(`确认删除MCP市场「${row.name}」？`, '提示', {
      type: 'warning',
      confirmButtonText: '删除',
      cancelButtonText: '取消'
    })
    await api.delete(`/admin/mcp-markets/${row.id}`)
    ElMessage.success('删除成功')
    await loadMarkets()
    await loadServices(1)
  } catch (error) {
    if (error !== 'cancel') {
      ElMessage.error(error.response?.data?.error || '删除失败')
    }
  }
}

const testMarket = async (row) => {
  try {
    const resp = await api.post(`/admin/mcp-markets/${row.id}/test`)
    const count = resp.data?.data?.service_count ?? 0
    ElMessage.success(`连接成功，可发现 ${count} 个服务`)
  } catch (error) {
    ElMessage.error(error.response?.data?.error || '连接测试失败')
  }
}

const loadServices = async (page = 1) => {
  servicePage.value = page
  servicesLoading.value = true
  try {
    const resp = await api.get('/admin/mcp-market/services', {
      params: {
        q: serviceQuery.value,
        page: servicePage.value,
        page_size: servicePageSize.value
      }
    })
    const data = resp.data.data || {}
    services.value = data.items || []
    serviceTotal.value = data.total || 0
    serviceWarnings.value = data.warnings || []
  } catch (error) {
    ElMessage.error(error.response?.data?.error || '加载聚合服务失败')
  } finally {
    servicesLoading.value = false
  }
}

const encodeMarketServicePath = (serviceId) =>
  String(serviceId || '')
    .split('/')
    .map((part) => encodeURIComponent(part))
    .join('/')

const formatEndpointSource = (source) => {
  const map = {
    mcpServers: '官方 mcpServers',
    operational_urls: '魔搭托管',
    operational_url: '运营地址',
    mcp_servers: 'mcp_servers',
    servers: 'servers',
    endpoints: 'endpoints',
    other: '其他'
  }
  return map[source] || source || '-'
}

const syncSelectedEndpoint = (detail) => {
  const endpoints = detail?.endpoints || []
  if (endpoints.length === 0) {
    selectedEndpointUrl.value = ''
    return
  }
  const current = selectedEndpointUrl.value
  if (current && endpoints.some((ep) => ep.url === current)) {
    return
  }
  selectedEndpointUrl.value = endpoints[0]?.url || ''
}

const selectDetailEndpoint = (row) => {
  if (row?.url) {
    selectedEndpointUrl.value = row.url
  }
}

const loadServiceDetail = async (row) => {
  detailDialogVisible.value = true
  detailLoading.value = true
  serviceDetail.value = null
  selectedEndpointUrl.value = ''
  try {
    const resp = await api.get(
      `/admin/mcp-market/services/${row.market_id}/${encodeMarketServicePath(row.service_id)}`
    )
    serviceDetail.value = resp.data?.data || null
    syncSelectedEndpoint(serviceDetail.value)
  } catch (error) {
    ElMessage.error(error.response?.data?.error || '加载服务详情失败')
  } finally {
    detailLoading.value = false
  }
}

const importFromDetail = async () => {
  const row = serviceDetail.value
  if (!row?.market_id || !row?.service_id) {
    ElMessage.error('服务标识缺失，无法导入')
    return
  }

  const endpoints = row.endpoints || []
  const endpointUrl =
    endpoints.length > 1 ? selectedEndpointUrl.value : endpoints[0]?.url || selectedEndpointUrl.value
  if (!endpointUrl) {
    ElMessage.warning('请先选择要导入的接入地址')
    return
  }

  detailImporting.value = true
  try {
    const payload = {
      market_id: row.market_id,
      service_id: row.service_id,
      name_override: '',
      endpoint_url: endpointUrl
    }
    const resp = await api.post('/admin/mcp-market/import', payload)
    const result = resp.data.data || {}
    ElMessage.success(`导入成功：${result.imported_count || 0} 个服务已应用`)
    if (Array.isArray(result.warnings) && result.warnings.length > 0) {
      ElMessage.warning(result.warnings.join('；'))
    }
    await loadServices(servicePage.value)
    await loadImportedItems(1)
    detailDialogVisible.value = false
    activeTab.value = 'imported'
  } catch (error) {
    ElMessage.error(error.response?.data?.error || '导入失败')
  } finally {
    detailImporting.value = false
  }
}

const loadImportedItems = async (page = 1) => {
  importedPage.value = page
  importedLoading.value = true
  try {
    const resp = await api.get('/admin/mcp-market/imported-services', {
      params: {
        q: importedQuery.value,
        page: importedPage.value,
        page_size: importedPageSize.value
      }
    })
    const data = resp.data.data || {}
    importedItems.value = data.items || []
    importedTotal.value = data.total || 0
  } catch (error) {
    ElMessage.error(error.response?.data?.error || '加载导入服务失败')
  } finally {
    importedLoading.value = false
  }
}

const parseImportedHeaders = () => {
  try {
    return buildHeadersFromAuth(importedAuthToken.value, importedHeadersText.value)
  } catch {
    throw new Error('Headers 不是合法 JSON 对象')
  }
}

const handleImportedTokenInput = () => {
  importedHeadersText.value = syncHeadersTextFromToken(importedAuthToken.value, importedHeadersText.value)
}

const handleImportedHeadersBlur = () => {
  try {
    const parsed = parseHeadersText(importedHeadersText.value)
    importedAuthToken.value = extractTokenFromHeaders(parsed)
  } catch {
    // 用户正在编辑 JSON，忽略
  }
}

const handleImportedToolsTokenInput = () => {
  importedToolsHeadersText.value = syncHeadersTextFromToken(
    importedToolsAuthToken.value,
    importedToolsHeadersText.value
  )
}

const handleImportedToolsHeadersBlur = () => {
  try {
    const parsed = parseHeadersText(importedToolsHeadersText.value)
    importedToolsAuthToken.value = extractTokenFromHeaders(parsed)
  } catch {
    // ignore
  }
}

const buildImportedToolsHeaders = () => {
  try {
    return buildHeadersFromAuth(importedToolsAuthToken.value, importedToolsHeadersText.value)
  } catch {
    throw new Error('Headers 不是合法 JSON 对象')
  }
}

const persistImportedToolsHeaders = async () => {
  if (!importedToolTarget.value) return null
  const headers = buildImportedToolsHeaders()
  const row = importedToolTarget.value
  const payload = {
    name: row.name,
    enabled: row.enabled,
    transport: row.transport,
    url: row.url,
    headers,
    allowed_tools: row.allowed_tools || [],
    market_id: row.market_id || null,
    provider_id: row.provider_id || '',
    service_id: row.service_id || '',
    service_name: row.service_name || '',
    source_url: row.source_url || '',
    auth_required: row.auth_required !== false
  }
  await api.put(`/admin/mcp-market/imported-services/${row.id}`, payload)
  importedToolTarget.value = { ...row, headers }
  return headers
}

const resetImportedForm = () => {
  importedForm.name = ''
  importedForm.enabled = true
  importedForm.transport = 'streamablehttp'
  importedForm.url = ''
  importedForm.market_id = null
  importedForm.provider_id = ''
  importedForm.service_id = ''
  importedForm.service_name = ''
  importedForm.source_url = ''
  importedForm.auth_required = true
  importedHeadersText.value = ''
  importedAuthToken.value = ''
}

const mergeImportedToolOptions = (tools = [], selected = []) => {
  const merged = new Map()

  ;(tools || []).forEach((tool) => {
    if (!tool?.name) return
    merged.set(tool.name, {
      name: tool.name,
      description: tool.description || ''
    })
  })

  ;(selected || []).forEach((name) => {
    if (!name || merged.has(name)) return
    merged.set(name, {
      name,
      description: '当前配置已选择'
    })
  })

  importedToolOptions.value = Array.from(merged.values()).sort((a, b) => a.name.localeCompare(b.name))
}

const loadImportedToolOptions = async (serviceId, showToast = false) => {
  if (!serviceId) {
    mergeImportedToolOptions([], importedSelectedTools.value)
    return
  }

  importedToolsLoading.value = true
  try {
    const resp = await api.get(`/admin/mcp-market/imported-services/${serviceId}/tools`)
    const data = resp.data?.data || {}
    const tools = data.tools || []
    mergeImportedToolOptions(tools, importedSelectedTools.value)
    if (showToast) {
      if (tools.length > 0) {
        ElMessage.success(`探测到 ${tools.length} 个工具`)
        if (importedToolMode.value === 'all') {
          importedToolMode.value = 'selected'
        }
      } else {
        ElMessage.warning('未探测到可用工具，请确认 URL、传输方式与鉴权 Headers 是否正确')
      }
    }
  } catch (error) {
    mergeImportedToolOptions([], importedSelectedTools.value)
    ElMessage.error(error.response?.data?.error || '探测工具失败')
  } finally {
    importedToolsLoading.value = false
  }
}

const openCreateImportedDialog = () => {
  editingImported.value = null
  resetImportedForm()
  importedDialogVisible.value = true
}

const openEditImportedDialog = (row) => {
  editingImported.value = row
  importedForm.name = row.name || ''
  importedForm.enabled = !!row.enabled
  importedForm.transport = row.transport || 'streamablehttp'
  importedForm.url = row.url || ''
  importedForm.market_id = row.market_id || null
  importedForm.provider_id = row.provider_id || ''
  importedForm.service_id = row.service_id || ''
  importedForm.service_name = row.service_name || ''
  importedForm.source_url = row.source_url || ''
  importedForm.auth_required = row.auth_required !== false
  const authFields = loadAuthFieldsFromHeaders(row.headers)
  importedAuthToken.value = authFields.token
  importedHeadersText.value = authFields.headersText
  importedDialogVisible.value = true
}

const syncImportedToolMode = (selected = importedSelectedTools.value) => {
  importedToolMode.value = selected.length > 0 ? 'selected' : 'all'
}

const handleImportedToolModeChange = (mode) => {
  importedToolQuery.value = ''
  if (mode === 'all') {
    importedSelectedTools.value = []
  }
}

const openImportedToolsDialog = async (row) => {
  importedToolTarget.value = row
  importedSelectedTools.value = Array.isArray(row.allowed_tools) ? [...row.allowed_tools] : []
  importedToolQuery.value = ''
  const authFields = loadAuthFieldsFromHeaders(row.headers)
  importedToolsAuthToken.value = authFields.token
  importedToolsHeadersText.value = authFields.headersText
  syncImportedToolMode(importedSelectedTools.value)
  mergeImportedToolOptions([], importedSelectedTools.value)
  importedToolsDialogVisible.value = true
  await loadImportedToolOptions(row.id)
}

const refreshImportedTools = async () => {
  if (!importedToolTarget.value?.id) {
    ElMessage.warning('请先选择一个已导入服务')
    return
  }
  try {
    await persistImportedToolsHeaders()
  } catch (error) {
    ElMessage.error(error.message || '鉴权配置无效')
    return
  }
  await loadImportedToolOptions(importedToolTarget.value.id, true)
}

const saveImportedItem = async () => {
  if (!importedFormRef.value) return
  const valid = await importedFormRef.value.validate().catch(() => false)
  if (!valid) return

  let headers = null
  try {
    headers = parseImportedHeaders()
  } catch (error) {
    ElMessage.error(error.message)
    return
  }

  const payload = {
    name: importedForm.name,
    enabled: importedForm.enabled,
    transport: importedForm.transport,
    url: importedForm.url,
    headers,
    allowed_tools: editingImported.value?.allowed_tools || [],
    market_id: importedForm.market_id || null,
    provider_id: importedForm.provider_id,
    service_id: importedForm.service_id,
    service_name: importedForm.service_name,
    source_url: importedForm.source_url,
    auth_required: importedForm.auth_required
  }

  importedSaving.value = true
  try {
    if (editingImported.value) {
      await api.put(`/admin/mcp-market/imported-services/${editingImported.value.id}`, payload)
      ElMessage.success('更新成功')
    } else {
      await api.post('/admin/mcp-market/imported-services', payload)
      ElMessage.success('创建成功')
    }
    importedDialogVisible.value = false
    await loadImportedItems(importedPage.value)
  } catch (error) {
    ElMessage.error(error.response?.data?.error || '保存失败')
  } finally {
    importedSaving.value = false
  }
}

const saveImportedToolSelection = async () => {
  if (!importedToolTarget.value) return
  if (importedToolMode.value === 'selected' && importedSelectedTools.value.length === 0) {
    ElMessage.warning('请至少选择一个工具，或切换为“全部工具”')
    return
  }

  const row = importedToolTarget.value
  let headers = null
  try {
    headers = buildImportedToolsHeaders()
  } catch (error) {
    ElMessage.error(error.message)
    return
  }

  const payload = {
    name: row.name,
    enabled: row.enabled,
    transport: row.transport,
    url: row.url,
    headers,
    allowed_tools: importedToolMode.value === 'all' ? [] : importedSelectedTools.value,
    market_id: row.market_id || null,
    provider_id: row.provider_id || '',
    service_id: row.service_id || '',
    service_name: row.service_name || ''
  }

  importedSaving.value = true
  try {
    await api.put(`/admin/mcp-market/imported-services/${row.id}`, payload)
    ElMessage.success('工具策略已更新')
    importedToolsDialogVisible.value = false
    importedToolTarget.value = null
    importedToolQuery.value = ''
    await loadImportedItems(importedPage.value)
  } catch (error) {
    ElMessage.error(error.response?.data?.error || '保存失败')
  } finally {
    importedSaving.value = false
  }
}

const toggleImportedEnabled = async (row) => {
  const payload = {
    name: row.name,
    enabled: !row.enabled,
    transport: row.transport,
    url: row.url,
    headers: row.headers || null,
    allowed_tools: row.allowed_tools || [],
    market_id: row.market_id || null,
    provider_id: row.provider_id || '',
    service_id: row.service_id || '',
    service_name: row.service_name || ''
  }
  try {
    await api.put(`/admin/mcp-market/imported-services/${row.id}`, payload)
    ElMessage.success(row.enabled ? '已禁用' : '已启用')
    await loadImportedItems(importedPage.value)
  } catch (error) {
    ElMessage.error(error.response?.data?.error || '更新状态失败')
  }
}

const deleteImportedItem = async (row) => {
  try {
    await ElMessageBox.confirm(`确认删除导入服务「${row.name}」？`, '提示', {
      type: 'warning',
      confirmButtonText: '删除',
      cancelButtonText: '取消'
    })
    await api.delete(`/admin/mcp-market/imported-services/${row.id}`)
    ElMessage.success('删除成功')
    await loadImportedItems(importedPage.value)
  } catch (error) {
    if (error !== 'cancel') {
      ElMessage.error(error.response?.data?.error || '删除失败')
    }
  }
}

onMounted(async () => {
  await loadProviders()
  await loadMarkets()
  await loadServices(1)
  await loadImportedItems(1)
})
</script>

<style scoped>
.mcp-market-page {
  padding: 20px;
}

.market-tabs {
  --el-tabs-header-height: 44px;
}

.tab-label-with-badge {
  display: inline-flex;
  align-items: center;
  gap: 8px;
}

.tab-badge {
  line-height: 1;
}

.market-action-btn {
  padding: 0;
  min-width: 22px;
}

.panel-card {
  margin-bottom: 16px;
}

.panel-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
}

.search-actions {
  display: flex;
  gap: 8px;
  align-items: center;
}

.pagination-wrap {
  margin-top: 10px;
  display: flex;
  justify-content: flex-end;
}

.warning-alert {
  margin-top: 12px;
}

.detail-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(180px, 1fr));
  gap: 8px 12px;
  margin-bottom: 10px;
}

.detail-desc {
  margin: 8px 0 12px;
  color: var(--el-text-color-regular);
  line-height: 1.5;
}

.endpoint-pick-alert {
  margin: 12px 0;
}

.endpoint-pick-table {
  margin-top: 8px;
}

.provider-desc {
  margin-top: 6px;
  line-height: 1.5;
  color: #6b7280;
  font-size: 12px;
}

.field-help {
  margin-top: 6px;
  font-size: 12px;
  line-height: 1.5;
  color: #6b7280;
}

.field-help code {
  font-size: 11px;
}

.tool-auth-form {
  margin-bottom: 12px;
}

.tool-selector-card {
  border: 1px solid #e5edf5;
  border-radius: 16px;
  padding: 16px;
  background: linear-gradient(180deg, #fbfdff 0%, #f4f8fc 100%);
}

.tool-selector-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
}

.tool-selector-meta {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.tool-selector-title {
  color: #172033;
  font-size: 15px;
  font-weight: 600;
}

.tool-selector-tip {
  color: #667085;
  font-size: 12px;
  line-height: 1.5;
}

.tool-selector-actions {
  display: flex;
  align-items: center;
  gap: 10px;
}

.tool-mode-group {
  margin-top: 14px;
}

.tool-picker-search {
  margin-top: 14px;
  max-width: 320px;
}

.tool-picker-empty {
  margin-top: 14px;
  border: 1px dashed #d6deeb;
  border-radius: 12px;
  padding: 18px 16px;
  color: #667085;
  font-size: 13px;
  text-align: center;
  background: rgba(255, 255, 255, 0.72);
}

.tool-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px;
  margin-top: 14px;
}

.tool-tile {
  margin-right: 0;
  min-width: 0;
}

.tool-tile-body {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
  overflow: hidden;
}

.tool-tile-name {
  display: -webkit-box;
  color: #182230;
  font-size: 13px;
  font-weight: 600;
  line-height: 1.35;
  overflow: hidden;
  text-overflow: ellipsis;
  word-break: break-word;
  overflow-wrap: anywhere;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
}

.tool-tile-desc {
  display: -webkit-box;
  color: #667085;
  font-size: 12px;
  line-height: 1.45;
  overflow: hidden;
  text-overflow: ellipsis;
  word-break: break-word;
  overflow-wrap: anywhere;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
}

:deep(.tool-grid .el-checkbox.is-bordered) {
  width: 100%;
  height: auto;
  margin-right: 0;
  padding: 12px 14px;
  border-radius: 14px;
  align-items: flex-start;
  background: rgba(255, 255, 255, 0.9);
  border-color: #d7e1ec;
}

:deep(.tool-grid .el-checkbox__label) {
  padding-left: 10px;
  width: 100%;
  min-width: 0;
  overflow: hidden;
}

:deep(.tool-grid .el-checkbox.is-bordered.is-checked) {
  background: #eff7ff;
  border-color: #7ab8ff;
  box-shadow: 0 0 0 1px rgba(64, 158, 255, 0.08);
}

@media (max-width: 992px) {
  .detail-grid {
    grid-template-columns: 1fr;
  }

  .search-actions {
    flex-wrap: wrap;
  }

  .panel-header {
    flex-wrap: wrap;
  }

  .tool-selector-header {
    flex-direction: column;
  }

  .tool-selector-actions {
    width: 100%;
    justify-content: space-between;
  }

  .tool-grid {
    grid-template-columns: 1fr;
  }

  .tool-picker-search {
    max-width: none;
  }
}
</style>
