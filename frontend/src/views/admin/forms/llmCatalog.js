const defaultOption = { label: '默认', value: 'default' }
const enableOption = { label: '开启', value: 'enabled' }
const disableOption = { label: '关闭', value: 'disabled' }
const clearHistoryOptions = [
  { label: '默认', value: 'default' },
  { label: '清除', value: true },
  { label: '保留', value: false }
]

function withDefault(options) {
  return [defaultOption, ...options]
}

function createModel(value, thinking, extra = {}) {
  return {
    value,
    label: value,
    thinking,
    ...extra
  }
}

const openAIReasoningStandard = withDefault([
  { label: '极低', value: 'minimal' },
  { label: '低', value: 'low' },
  { label: '中', value: 'medium' },
  { label: '高', value: 'high' }
])

const openAIReasoningCodex = withDefault([
  { label: '关闭', value: 'none' },
  { label: '低', value: 'low' },
  { label: '中', value: 'medium' },
  { label: '高', value: 'high' }
])

const openAIReasoningCodexMax = withDefault([
  { label: '关闭', value: 'none' },
  { label: '低', value: 'low' },
  { label: '中', value: 'medium' },
  { label: '高', value: 'high' },
  { label: '极高', value: 'xhigh' }
])

const openAIReasoningLegacy = withDefault([
  { label: '低', value: 'low' },
  { label: '中', value: 'medium' },
  { label: '高', value: 'high' }
])

const openAIReasoningHighOnly = withDefault([
  { label: '高', value: 'high' }
])

const booleanThinkingOptions = withDefault([
  enableOption,
  disableOption
])

const doubaoReasoningOptions = withDefault([
  { label: '关闭', value: 'minimal' },
  { label: '低', value: 'low' },
  { label: '中', value: 'medium' },
  { label: '高', value: 'high' }
])

const anthropicAdaptiveOptions = [
  { label: '低', value: 'low' },
  { label: '中', value: 'medium' },
  { label: '高', value: 'high' },
  { label: '极高', value: 'max' }
]

const openAIReasoningLatest = withDefault([
  { label: '关闭', value: 'none' },
  { label: '低', value: 'low' },
  { label: '中', value: 'medium' },
  { label: '高', value: 'high' },
  { label: '极高', value: 'xhigh' }
])

const openAIReasoningLatestPro = withDefault([
  { label: '中', value: 'medium' },
  { label: '高', value: 'high' },
  { label: '极高', value: 'xhigh' }
])

const openAIReasoningRequest = {
  allowMaxTokens: false,
  allowTemperature: false,
  allowTopP: false
}

const anthropicManualThinking = {
  label: '深度思考',
  options: withDefault([{ label: '手动思考', value: 'enabled' }]),
  showBudgetFor: ['enabled'],
  budgetMin: 1024,
  budgetRequiredFor: ['enabled']
}

const anthropicAdaptiveThinking = {
  label: '深度思考',
  options: withDefault([
    { label: '手动思考', value: 'enabled' },
    { label: '自适应思考', value: 'adaptive' }
  ]),
  showBudgetFor: ['enabled'],
  budgetMin: 1024,
  budgetRequiredFor: ['enabled'],
  showEffortFor: ['adaptive'],
  effortOptions: anthropicAdaptiveOptions
}

const zhipuThinkingConfig = {
  label: '深度思考',
  options: booleanThinkingOptions,
  showClearThinkingFor: ['enabled'],
  clearThinkingOptions: clearHistoryOptions
}

const aliyunThinkingConfig = {
  label: '深度思考',
  options: booleanThinkingOptions,
  showBudgetFor: ['enabled'],
  budgetMin: 1,
  budgetStep: 256
}

const siliconflowThinkingConfig = {
  label: '深度思考',
  options: booleanThinkingOptions,
  showBudgetFor: ['enabled'],
  budgetMin: 128,
  budgetMax: 32768,
  budgetStep: 128
}

const providerTypeMap = {
  openai: 'openai',
  ollama: 'ollama',
  azure: 'openai',
  anthropic: 'openai',
  zhipu: 'openai',
  aliyun: 'openai',
  doubao: 'openai',
  siliconflow: 'openai',
  deepseek: 'openai',
  dify: 'dify',
  coze: 'coze'
}

const knownProviders = new Set(Object.keys(providerTypeMap))

const editableBaseURLProviders = new Set(['openai', 'ollama', 'azure', 'dify', 'coze'])

const catalog = {
  openai: {
    quickUrl: 'https://api.openai.com/v1',
    modelPlaceholder: '请选择或输入模型名称',
    modelHint: '默认优先使用官方稳定别名；如需锁定行为，可手动输入精确快照模型 ID。',
    models: [
      createModel('gpt-5.4', { label: '思考强度', options: openAIReasoningLatest }, { request: openAIReasoningRequest }),
      createModel('gpt-5.4-pro', { label: '思考强度', options: openAIReasoningLatestPro }, { request: openAIReasoningRequest }),
      createModel('gpt-5.4-mini', { label: '思考强度', options: openAIReasoningLatest }, { request: openAIReasoningRequest }),
      createModel('gpt-5.4-nano', { label: '思考强度', options: openAIReasoningLatest }, { request: openAIReasoningRequest }),
      createModel('gpt-5.2', { label: '思考强度', options: openAIReasoningLatest }, { request: openAIReasoningRequest }),
      createModel('gpt-5.2-pro', { label: '思考强度', options: openAIReasoningLatestPro }, { request: openAIReasoningRequest }),
      createModel('gpt-5-chat-latest', false, { hint: 'ChatGPT 专用别名，适合兼容旧工作流；新接入优先选择主线 GPT-5.* 模型。' }),
      createModel('gpt-5-pro', { label: '思考强度', options: openAIReasoningHighOnly }, { request: openAIReasoningRequest }),
      createModel('gpt-5', { label: '思考强度', options: openAIReasoningStandard }, { request: openAIReasoningRequest }),
      createModel('gpt-5-mini', { label: '思考强度', options: openAIReasoningStandard }, { request: openAIReasoningRequest }),
      createModel('gpt-5-nano', { label: '思考强度', options: openAIReasoningStandard }, { request: openAIReasoningRequest }),
      createModel('gpt-5.3-codex', { label: '思考强度', options: openAIReasoningCodexMax }, { request: openAIReasoningRequest }),
      createModel('gpt-5.2-codex', { label: '思考强度', options: openAIReasoningCodexMax }, { request: openAIReasoningRequest }),
      createModel('gpt-5-codex', { label: '思考强度', options: openAIReasoningLegacy }, { request: openAIReasoningRequest }),
      createModel('gpt-5.1', { label: '思考强度', options: openAIReasoningCodex }, { request: openAIReasoningRequest }),
      createModel('gpt-5.1-codex', { label: '思考强度', options: openAIReasoningCodex }, { request: openAIReasoningRequest }),
      createModel('gpt-5.1-codex-mini', { label: '思考强度', options: openAIReasoningCodex }, { request: openAIReasoningRequest }),
      createModel('gpt-5.1-codex-max', { label: '思考强度', options: openAIReasoningCodexMax }, { request: openAIReasoningRequest }),
      createModel('o3', { label: '思考强度', options: openAIReasoningLegacy }, { request: openAIReasoningRequest }),
      createModel('o4-mini', { label: '思考强度', options: openAIReasoningLegacy }, { request: openAIReasoningRequest }),
      createModel('o3-mini', { label: '思考强度', options: openAIReasoningLegacy }, { request: openAIReasoningRequest }),
      createModel('o1', { label: '思考强度', options: openAIReasoningLegacy }, { request: openAIReasoningRequest })
    ],
    fallbackThinking: {
      label: '思考强度',
      options: openAIReasoningCodex,
      hint: '自定义模型未命中文档内列表，已回退到通用 reasoning_effort 配置；是否生效取决于实际模型。'
    }
  },
  ollama: {
    quickUrl: 'http://127.0.0.1:11434/v1',
    modelPlaceholder: '请选择或输入模型名称',
    modelHint: 'Ollama 使用本地或私有模型服务，模型列表和地址都允许自定义。',
    models: [],
    fallbackThinking: null
  },
  azure: {
    quickUrl: 'https://your-resource-name.openai.azure.com/openai/v1/',
    modelPlaceholder: '请选择官方模型名或输入自定义部署名',
    modelHint: 'Azure 这里填写的是 deployment name；列表中的名称主要用于参考其底层模型能力。',
    models: [
      createModel('gpt-5.4', { label: '思考强度', options: openAIReasoningLatest }, { request: openAIReasoningRequest }),
      createModel('gpt-5.4-pro', { label: '思考强度', options: openAIReasoningLatestPro }, { request: openAIReasoningRequest }),
      createModel('gpt-5.2', { label: '思考强度', options: openAIReasoningLatest }, { request: openAIReasoningRequest }),
      createModel('gpt-5.2-chat', false, { hint: 'Azure 文档中的 Chat 型号通常通过 deployment 名称接入；是否开放取决于区域和配额。' }),
      createModel('gpt-5.3-codex', { label: '思考强度', options: openAIReasoningCodexMax }, { request: openAIReasoningRequest }),
      createModel('gpt-5.2-codex', { label: '思考强度', options: openAIReasoningCodexMax }, { request: openAIReasoningRequest }),
      createModel('gpt-5-mini', { label: '思考强度', options: openAIReasoningStandard }, { request: openAIReasoningRequest }),
      createModel('gpt-5-nano', { label: '思考强度', options: openAIReasoningStandard }, { request: openAIReasoningRequest }),
      createModel('gpt-5-chat', { label: '思考强度', options: openAIReasoningStandard }, { request: openAIReasoningRequest }),
      createModel('gpt-5-pro', { label: '思考强度', options: openAIReasoningHighOnly }, { request: openAIReasoningRequest }),
      createModel('o4-mini', { label: '思考强度', options: openAIReasoningLegacy }, { request: openAIReasoningRequest }),
      createModel('o3', { label: '思考强度', options: openAIReasoningLegacy }, { request: openAIReasoningRequest }),
      createModel('o3-mini', { label: '思考强度', options: openAIReasoningLegacy }, { request: openAIReasoningRequest }),
      createModel('o1', { label: '思考强度', options: openAIReasoningLegacy }, { request: openAIReasoningRequest })
    ],
    fallbackThinking: {
      label: '思考强度',
      options: openAIReasoningCodex,
      hint: 'Azure 自定义 deployment 未命中文档模型时，会回退到通用 reasoning_effort 配置；具体兼容性以部署模型为准。'
    }
  },
  anthropic: {
    quickUrl: 'https://api.anthropic.com/v1/',
    modelPlaceholder: '请选择或输入模型名称',
    modelHint: '默认优先使用官方稳定别名；若需要固定版本或回归测试，可改填带日期的精确模型 ID。',
    models: [
      createModel('claude-opus-4-6', anthropicAdaptiveThinking),
      createModel('claude-sonnet-4-6', anthropicAdaptiveThinking),
      createModel('claude-haiku-4-5', anthropicManualThinking),
      createModel('claude-3-7-sonnet', anthropicManualThinking),
      createModel('claude-sonnet-4', anthropicManualThinking),
      createModel('claude-opus-4', anthropicManualThinking),
      createModel('claude-opus-4-1', anthropicManualThinking)
    ],
    fallbackThinking: {
      ...anthropicAdaptiveThinking,
      hint: '自定义模型未命中文档内列表。若使用手动思考，需要显式填写 budget_tokens；Adaptive 请仅在文档确认支持的模型上使用。'
    }
  },
  zhipu: {
    quickUrl: 'https://open.bigmodel.cn/api/paas/v4',
    modelPlaceholder: '请选择或输入模型名称',
    modelHint: '智谱文档支持通过 thinking.type 和 clear_thinking 控制思考模式。',
    models: [
      createModel('glm-5', zhipuThinkingConfig),
      createModel('glm-4.7', zhipuThinkingConfig),
      createModel('glm-4.7-flashx', zhipuThinkingConfig),
      createModel('glm-4.7-flash', zhipuThinkingConfig),
      createModel('glm-4.6', zhipuThinkingConfig),
      createModel('glm-4.6v', zhipuThinkingConfig),
      createModel('glm-4.5', zhipuThinkingConfig),
      createModel('glm-4.5-air', zhipuThinkingConfig),
      createModel('glm-4.5-airx', zhipuThinkingConfig),
      createModel('glm-4.5v', zhipuThinkingConfig)
    ],
    fallbackThinking: {
      ...zhipuThinkingConfig,
      hint: '自定义模型未命中文档内列表，已回退到通用 thinking.type / clear_thinking 配置。'
    }
  },
  aliyun: {
    quickUrl: 'https://dashscope.aliyuncs.com/compatible-mode/v1',
    modelPlaceholder: '请选择或输入模型名称',
    modelHint: '默认优先使用官方稳定别名；如果你要锁定具体版本，再手动填写带日期或小版本后缀的模型 ID。',
    models: [
      createModel('qwen-plus-latest', aliyunThinkingConfig),
      createModel('qwen-turbo-latest', aliyunThinkingConfig),
      createModel('qwen3-max', aliyunThinkingConfig),
      createModel('qwen3-235b-a22b', aliyunThinkingConfig),
      createModel('qwen3-30b-a3b', aliyunThinkingConfig),
      createModel('qwen3-next-80b-a3b-thinking', aliyunThinkingConfig),
      createModel('glm-4.7', aliyunThinkingConfig),
      createModel('glm-4.6', aliyunThinkingConfig),
      createModel('glm-4.5', aliyunThinkingConfig),
      createModel('glm-4.5-air', aliyunThinkingConfig),
      createModel('kimi-k2-thinking', aliyunThinkingConfig),
      createModel('qwen3-235b-a22b-thinking-2507', aliyunThinkingConfig, { label: 'qwen3-235b-a22b-thinking-2507（版本化）' }),
      createModel('qwen3-30b-a3b-thinking-2507', aliyunThinkingConfig, { label: 'qwen3-30b-a3b-thinking-2507（版本化）' }),
      createModel('kimi/kimi-k2.5', aliyunThinkingConfig, { label: 'kimi/kimi-k2.5（版本化）' })
    ],
    fallbackThinking: {
      ...aliyunThinkingConfig,
      hint: '自定义模型未命中文档内列表。若模型支持 thinking_budget，可按文档填写；留空时不会传该字段。'
    }
  },
  doubao: {
    quickUrl: 'https://ark.cn-beijing.volces.com/api/v3',
    modelPlaceholder: '请选择或输入模型 ID（通常带版本后缀）',
    modelHint: '豆包优先填写官方真实 Model ID。当前未确认有稳定别名可通用替代，建议以控制台或模型列表中的 Model ID 为准。',
    models: [
      createModel('doubao-seed-2-0-pro-260215', { label: '思考强度', options: doubaoReasoningOptions }, { label: 'Doubao Seed 2.0 Pro (doubao-seed-2-0-pro-260215)' }),
      createModel('doubao-seed-2-0-lite-260215', { label: '思考强度', options: doubaoReasoningOptions }, { label: 'Doubao Seed 2.0 Lite (doubao-seed-2-0-lite-260215)' }),
      createModel('doubao-seed-2-0-mini-260215', { label: '思考强度', options: doubaoReasoningOptions }, { label: 'Doubao Seed 2.0 Mini (doubao-seed-2-0-mini-260215)' }),
      createModel('doubao-seed-1-6-251015', { label: '思考强度', options: doubaoReasoningOptions }, { label: 'Doubao Seed 1.6 (doubao-seed-1-6-251015)' })
    ],
    fallbackThinking: {
      label: '思考强度',
      options: doubaoReasoningOptions,
      hint: '自定义模型未命中文档内列表，已回退到通用 reasoning_effort 配置；是否生效取决于实际模型。'
    }
  },
  siliconflow: {
    quickUrl: 'https://api.siliconflow.cn/v1',
    modelPlaceholder: '请选择或输入模型名称',
    modelHint: 'SiliconFlow 文档直接列出了 enable_thinking 支持模型；仅对文档列出的模型展示预算配置。',
    models: [
      createModel('Pro/zai-org/GLM-5', siliconflowThinkingConfig),
      createModel('Pro/zai-org/GLM-4.7', siliconflowThinkingConfig),
      createModel('deepseek-ai/DeepSeek-V3.2', siliconflowThinkingConfig),
      createModel('Pro/deepseek-ai/DeepSeek-V3.2', siliconflowThinkingConfig),
      createModel('zai-org/GLM-4.6', siliconflowThinkingConfig),
      createModel('Qwen/Qwen3-8B', siliconflowThinkingConfig),
      createModel('Qwen/Qwen3-14B', siliconflowThinkingConfig),
      createModel('Qwen/Qwen3-32B', siliconflowThinkingConfig),
      createModel('Qwen/Qwen3-30B-A3B', siliconflowThinkingConfig),
      createModel('tencent/Hunyuan-A13B-Instruct', siliconflowThinkingConfig),
      createModel('zai-org/GLM-4.5V', siliconflowThinkingConfig),
      createModel('deepseek-ai/DeepSeek-V3.1-Terminus', siliconflowThinkingConfig),
      createModel('Pro/deepseek-ai/DeepSeek-V3.1-Terminus', siliconflowThinkingConfig),
      createModel('Qwen/Qwen3.5-397B-A17B', siliconflowThinkingConfig),
      createModel('Qwen/Qwen3.5-122B-A10B', siliconflowThinkingConfig),
      createModel('Qwen/Qwen3.5-35B-A3B', siliconflowThinkingConfig),
      createModel('Qwen/Qwen3.5-27B', siliconflowThinkingConfig),
      createModel('Qwen/Qwen3.5-9B', siliconflowThinkingConfig),
      createModel('Qwen/Qwen3.5-4B', siliconflowThinkingConfig)
    ],
    fallbackThinking: {
      ...siliconflowThinkingConfig,
      hint: '自定义模型未命中文档内列表。若模型支持 enable_thinking / thinking_budget，可按文档填写；留空时不会传 thinking_budget。'
    }
  },
  deepseek: {
    quickUrl: 'https://api.deepseek.com/v1',
    modelPlaceholder: '请选择或输入模型名称',
    modelHint: '官方 DeepSeek 通过选择不同模型切换思考模式：deepseek-chat 为非思考，deepseek-reasoner 为思考。',
    models: [
      createModel('deepseek-chat', false, {
        hint: 'deepseek-chat 是非思考模型，不需要额外 thinking 参数。'
      }),
      createModel('deepseek-reasoner', false, {
        hint: 'deepseek-reasoner 已内置思考模式，不需要额外 thinking 参数。'
      })
    ],
    fallbackThinking: {
      label: '深度思考',
      options: booleanThinkingOptions,
      hint: '官方 DeepSeek 推荐通过模型名切换思考模式。自定义代理若额外支持 thinking.type，可在这里启用兼容开关。'
    }
  }
}

function cloneOptions(options = []) {
  return options.map(option => ({ ...option }))
}

function normalizeModelName(modelName) {
  return String(modelName || '').trim().toLowerCase()
}

export function resolveLLMProvider(provider, type) {
  const normalizedProvider = String(provider || '').trim().toLowerCase()
  const normalizedType = String(type || '').trim().toLowerCase()

  if (normalizedProvider === 'openai' && ['ollama', 'dify', 'coze'].includes(normalizedType)) {
    return normalizedType
  }
  if (knownProviders.has(normalizedProvider)) {
    return normalizedProvider
  }
  if (['ollama', 'dify', 'coze'].includes(normalizedType)) {
    return normalizedType
  }
  return 'openai'
}

export function getProviderFixedType(provider) {
  return providerTypeMap[provider] || 'openai'
}

export function isProviderBaseURLEditable(provider) {
  return editableBaseURLProviders.has(provider)
}

export function getProviderQuickUrl(provider) {
  return catalog[provider]?.quickUrl || ''
}

/** 解析应写入 json_data 的 base_url（含 DeepSeek 等固定地址提供商） */
export function resolveProviderBaseUrl(provider, baseUrl = '') {
  const trimmed = String(baseUrl || '').trim()
  if (trimmed) {
    return trimmed
  }
  return getProviderQuickUrl(provider) || ''
}

export function getProviderModelOptions(provider) {
  return (catalog[provider]?.models || []).map(model => ({
    label: model.label,
    value: model.value
  }))
}

export function getProviderModelHint(provider) {
  return catalog[provider]?.modelHint || ''
}

export function getProviderModelFieldLabel(provider) {
  if (provider === 'azure') {
    return '部署名称'
  }
  if (provider === 'doubao') {
    return '模型 ID'
  }
  return '模型名称'
}

export function getProviderModelPlaceholder(provider) {
  return catalog[provider]?.modelPlaceholder || '请选择或输入模型名称'
}

export function resolveProviderModel(provider, modelName) {
  const normalized = normalizeModelName(modelName)
  if (!normalized) {
    return null
  }

  const models = catalog[provider]?.models || []
  return models.find(model => normalizeModelName(model.value) === normalized) || null
}

export function getProviderRequestConfig(provider, modelName) {
  const model = resolveProviderModel(provider, modelName)
  return {
    allowMaxTokens: true,
    allowTemperature: true,
    allowTopP: true,
    temperatureMax: 2,
    ...(model?.request || {})
  }
}

export function getProviderThinkingConfig(provider, modelName) {
  const model = resolveProviderModel(provider, modelName)
  if (model?.thinking === false) {
    return {
      visible: false,
      hint: model.hint || ''
    }
  }

  const source = model?.thinking || catalog[provider]?.fallbackThinking
  if (!source) {
    return {
      visible: false,
      hint: model?.hint || ''
    }
  }

  return {
    visible: true,
    label: source.label || '深度思考',
    options: cloneOptions(source.options),
    showBudgetFor: [...(source.showBudgetFor || [])],
    budgetMin: source.budgetMin || 1,
    budgetMax: source.budgetMax || 100000,
    budgetStep: source.budgetStep || 1,
    budgetRequiredFor: [...(source.budgetRequiredFor || [])],
    showEffortFor: [...(source.showEffortFor || [])],
    effortOptions: cloneOptions(source.effortOptions || []),
    showClearThinkingFor: [...(source.showClearThinkingFor || [])],
    clearThinkingOptions: cloneOptions(source.clearThinkingOptions || clearHistoryOptions),
    hint: model?.hint || source.hint || ''
  }
}
