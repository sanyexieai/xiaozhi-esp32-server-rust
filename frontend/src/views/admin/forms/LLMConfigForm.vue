<template>
  <el-form ref="formRef" :model="model" :rules="rules" label-width="120px">
    <el-form-item label="提供商" prop="provider" required>
      <el-select v-model="model.provider" placeholder="请选择提供商" style="width: 100%" @change="onProviderChange">
        <el-option label="OpenAI" value="openai" />
        <el-option label="Ollama" value="ollama" />
        <el-option label="Azure OpenAI" value="azure" />
        <el-option label="Anthropic" value="anthropic" />
        <el-option label="智谱AI" value="zhipu" />
        <el-option label="阿里云" value="aliyun" />
        <el-option label="豆包" value="doubao" />
        <el-option label="硅基流动" value="siliconflow" />
        <el-option label="DeepSeek（深度求索）" value="deepseek" />
        <el-option label="Dify" value="dify" />
        <el-option label="Coze" value="coze" />
      </el-select>
    </el-form-item>
    <el-form-item label="配置名称" prop="name">
      <el-input v-model="model.name" placeholder="请输入配置名称" />
    </el-form-item>
    <el-form-item label="配置ID" prop="config_id">
      <el-input v-model="model.config_id" placeholder="请输入唯一的配置ID" />
    </el-form-item>

    <el-form-item v-if="isOpenAIOrOllama" :label="modelFieldLabel" prop="model_name" required>
      <el-select
        v-model="model.model_name"
        filterable
        allow-create
        default-first-option
        clearable
        :placeholder="modelPlaceholder"
        style="width: 100%"
      >
        <el-option
          v-for="option in modelOptions"
          :key="option.value"
          :label="option.label"
          :value="option.value"
        />
      </el-select>
    </el-form-item>

    <el-form-item v-if="isOpenAIOrOllama && modelHint">
      <el-alert
        :title="modelHint"
        type="info"
        :closable="false"
        show-icon
      />
    </el-form-item>

    <el-form-item label="API密钥" prop="api_key" :required="apiKeyRequired">
      <el-input v-model="model.api_key" type="password" placeholder="请输入API密钥" show-password />
    </el-form-item>

    <el-form-item v-if="showBaseURL" label="基础URL" prop="base_url" required>
      <el-input v-model="model.base_url" placeholder="请输入基础URL" style="width: 100%" />
    </el-form-item>

    <el-form-item v-if="isCoze" label="Bot ID" prop="bot_id" required>
      <el-input v-model="model.bot_id" placeholder="请输入 Coze Bot ID" />
    </el-form-item>

    <el-form-item v-if="isDify || isCoze" label="User前缀" prop="user_prefix">
      <el-input v-model="model.user_prefix" placeholder="可选，默认 xiaozhi" />
    </el-form-item>

    <el-form-item v-if="isCoze" label="Connector ID" prop="connector_id">
      <el-input v-model="model.connector_id" placeholder="可选，默认 1024" />
    </el-form-item>

    <el-form-item v-if="isOpenAIOrOllama && requestConfig.allowMaxTokens" label="max_tokens" prop="max_tokens" required>
      <el-input-number v-model="model.max_tokens" :min="1" :max="100000" placeholder="max_tokens" style="width: 100%" />
    </el-form-item>

    <el-form-item v-if="isOpenAIOrOllama && requestConfig.allowTemperature" label="温度" prop="temperature">
      <el-input-number v-model="model.temperature" :min="0" :max="requestConfig.temperatureMax" :step="0.1" placeholder="温度" style="width: 100%" />
    </el-form-item>

    <el-form-item v-if="isOpenAIOrOllama && requestConfig.allowTopP" label="Top P" prop="top_p">
      <el-input-number v-model="model.top_p" :min="0" :max="1" :step="0.1" placeholder="Top P" style="width: 100%" />
    </el-form-item>

    <el-form-item v-if="isOpenAIOrOllama && requestCapabilityHint" label="参数提示">
      <el-alert
        :title="requestCapabilityHint"
        type="info"
        :closable="false"
        show-icon
      />
    </el-form-item>

    <template v-if="thinkingConfig.visible">
      <el-form-item :label="thinkingConfig.label" prop="thinking_mode">
        <el-select v-model="model.thinking_mode" placeholder="请选择深度思考模式" style="width: 100%">
          <el-option
            v-for="option in thinkingConfig.options"
            :key="option.value"
            :label="option.label"
            :value="option.value"
          />
        </el-select>
      </el-form-item>

      <el-form-item v-if="thinkingConfig.showEffort" label="思考力度" prop="thinking_effort">
        <el-select v-model="model.thinking_effort" placeholder="请选择思考力度" style="width: 100%">
          <el-option
            v-for="option in thinkingConfig.effortOptions"
            :key="option.value"
            :label="option.label"
            :value="option.value"
          />
        </el-select>
      </el-form-item>

      <el-form-item v-if="thinkingConfig.showBudget" label="思考预算" prop="thinking_budget_tokens">
        <el-input-number
          v-model="model.thinking_budget_tokens"
          :min="thinkingConfig.budgetMin"
          :max="thinkingConfig.budgetMax"
          :step="thinkingConfig.budgetStep"
          placeholder="预算 token"
          style="width: 100%"
        />
      </el-form-item>

      <el-form-item v-if="thinkingConfig.showClearThinking" label="历史思考链">
        <el-select v-model="model.thinking_clear_thinking" placeholder="请选择历史思考链处理方式" style="width: 100%">
          <el-option
            v-for="option in thinkingConfig.clearThinkingOptions"
            :key="String(option.value)"
            :label="option.label"
            :value="option.value"
          />
        </el-select>
      </el-form-item>

      <el-form-item>
        <el-alert
          :title="thinkingConfig.hint"
          type="warning"
          :closable="false"
          show-icon
        />
      </el-form-item>
    </template>
  </el-form>
</template>

<script setup>
import { computed, ref, watch } from 'vue'
import {
  getProviderFixedType,
  getProviderModelFieldLabel,
  getProviderModelHint,
  getProviderModelOptions,
  getProviderModelPlaceholder,
  getProviderQuickUrl,
  getProviderRequestConfig,
  getProviderThinkingConfig,
  isProviderBaseURLEditable,
  resolveLLMProvider,
  resolveProviderBaseUrl
} from './llmCatalog'

const props = defineProps({
  model: { type: Object, required: true },
  rules: { type: Object, default: () => ({}) }
})

const formRef = ref()

const resolvedProvider = computed(() => resolveLLMProvider(props.model?.provider, props.model?.type))
const effectiveType = computed(() => getProviderFixedType(resolvedProvider.value))
const isOpenAIOrOllama = computed(() => effectiveType.value === 'openai' || effectiveType.value === 'ollama')
const isOllama = computed(() => effectiveType.value === 'ollama')
const isDify = computed(() => effectiveType.value === 'dify')
const isCoze = computed(() => effectiveType.value === 'coze')
const showBaseURL = computed(() => isProviderBaseURLEditable(resolvedProvider.value))
const apiKeyRequired = computed(() => !isOllama.value)

const defaultThinkingMode = 'default'

const provider = computed(() => resolvedProvider.value || '')
const modelFieldLabel = computed(() => getProviderModelFieldLabel(provider.value))
const modelPlaceholder = computed(() => getProviderModelPlaceholder(provider.value))
const modelOptions = computed(() => getProviderModelOptions(provider.value))
const thinkingDefinition = computed(() => getProviderThinkingConfig(provider.value, props.model?.model_name))
const modelHint = computed(() => {
  if (!thinkingDefinition.value?.visible && thinkingDefinition.value?.hint) {
    return thinkingDefinition.value.hint
  }
  return getProviderModelHint(provider.value)
})
const requestConfig = computed(() => getProviderRequestConfig(provider.value, props.model?.model_name))

const requestCapabilityHint = computed(() => {
  const blockedFields = []
  if (!requestConfig.value.allowMaxTokens) {
    blockedFields.push('max_tokens')
  }
  if (!requestConfig.value.allowTemperature) {
    blockedFields.push('temperature')
  }
  if (!requestConfig.value.allowTopP) {
    blockedFields.push('top_p')
  }
  if (!blockedFields.length) {
    return ''
  }
  return `当前模型按文档不建议单独设置 ${blockedFields.join('、')}，保存时不会透传这些字段。`
})

const thinkingConfig = computed(() => {
  const config = thinkingDefinition.value
  if (!config?.visible) {
    return {
      visible: false,
      label: '深度思考',
      options: [],
      showBudget: false,
      budgetMin: 1,
      budgetMax: 100000,
      budgetStep: 1,
      budgetRequired: false,
      showEffort: false,
      effortOptions: [],
      showClearThinking: false,
      clearThinkingOptions: [],
      hint: config?.hint || ''
    }
  }

  const showBudget = config.showBudgetFor.includes(props.model?.thinking_mode)
  const showEffort = config.showEffortFor.includes(props.model?.thinking_mode)
  const showClearThinking = config.showClearThinkingFor.includes(props.model?.thinking_mode)
  const budgetRequired = config.budgetRequiredFor.includes(props.model?.thinking_mode)

  return {
    visible: true,
    label: config.label,
    options: config.options,
    showBudget,
    budgetMin: config.budgetMin,
    budgetMax: config.budgetMax,
    budgetStep: config.budgetStep,
    budgetRequired,
    showEffort,
    effortOptions: config.effortOptions,
    showClearThinking,
    clearThinkingOptions: config.clearThinkingOptions,
    hint: config.hint || '开启更强的深度思考后，模型通常会花更多时间推理，首包和整体响应都会明显变慢。'
  }
})

function normalizeThinkingState(provider) {
  if (!props.model) {
    return
  }

  const config = getProviderThinkingConfig(provider, props.model?.model_name)
  if (!config?.visible) {
    props.model.thinking_mode = defaultThinkingMode
    props.model.thinking_budget_tokens = null
    props.model.thinking_clear_thinking = 'default'
    return
  }

  const options = config.options.map(option => option.value)
  if (!options.includes(props.model.thinking_mode)) {
    props.model.thinking_mode = defaultThinkingMode
  }

  if (props.model.thinking_budget_tokens !== null && props.model.thinking_budget_tokens !== undefined && props.model.thinking_budget_tokens !== '') {
    const budgetValue = Number(props.model.thinking_budget_tokens)
    if (Number.isNaN(budgetValue) || budgetValue < (config.budgetMin || 1)) {
      props.model.thinking_budget_tokens = null
    }
  }

  if (!props.model.thinking_effort) {
    props.model.thinking_effort = 'medium'
  }

  if (props.model.thinking_clear_thinking === undefined || props.model.thinking_clear_thinking === null || props.model.thinking_clear_thinking === '') {
    props.model.thinking_clear_thinking = 'default'
  }
}

function applyProviderDefaults(value, forceEditableURL = false, resetModel = false) {
  if (!value || !props.model) {
    return
  }

  props.model.type = getProviderFixedType(value)
  if (isProviderBaseURLEditable(value)) {
    const quickUrl = getProviderQuickUrl(value)
    if (forceEditableURL || !props.model.base_url) {
      props.model.base_url = quickUrl
    }
  } else {
    // DeepSeek 等提供商 URL 固定，不在表单展示，但仍需写入配置
    props.model.base_url = getProviderQuickUrl(value) || ''
  }

  if (resetModel) {
    props.model.model_name = ''
  }
  if (value === 'dify' && (!props.model.model_name || resetModel)) {
    props.model.model_name = 'dify'
  }
  if (value === 'coze') {
    if (!props.model.model_name || resetModel) {
      props.model.model_name = 'coze'
    }
    if (!props.model.connector_id) {
      props.model.connector_id = '1024'
    }
  }
}

watch(() => props.model?.provider, (value) => {
  if (!value || !props.model) {
    return
  }
  applyProviderDefaults(value, false)
  normalizeThinkingState(value)
}, { immediate: true })

watch(() => props.model?.model_name, () => {
  if (!provider.value) {
    return
  }
  normalizeThinkingState(provider.value)
}, { immediate: true })

function onProviderChange(value) {
  if (!value || !props.model) {
    return
  }
  applyProviderDefaults(value, true, true)
}

function getJsonData() {
  const m = props.model
  const providerName = resolveLLMProvider(m?.provider, m?.type)
  const providerType = getProviderFixedType(providerName)
  const thinking = buildThinkingPayload(m)
  if (providerType === 'dify') {
    const config = {
      api_key: m.api_key,
      user_prefix: m.user_prefix
    }
    if (m.base_url) config.base_url = m.base_url
    return JSON.stringify(config, null, 2)
  }
  if (providerType === 'coze') {
    const config = {
      api_key: m.api_key,
      bot_id: m.bot_id,
      user_prefix: m.user_prefix,
      connector_id: m.connector_id
    }
    if (m.base_url) config.base_url = m.base_url
    return JSON.stringify(config, null, 2)
  }

  const config = {
    model_name: m.model_name,
    api_key: m.api_key
  }
  const baseUrl = resolveProviderBaseUrl(providerName, m.base_url)
  if (baseUrl) {
    config.base_url = baseUrl
  }
  if (requestConfig.value.allowMaxTokens && m.max_tokens !== undefined && m.max_tokens !== null && m.max_tokens !== '') {
    config.max_tokens = m.max_tokens
  }
  if (requestConfig.value.allowTemperature && m.temperature !== undefined && m.temperature !== null) config.temperature = m.temperature
  if (requestConfig.value.allowTopP && m.top_p !== undefined && m.top_p !== null) config.top_p = m.top_p
  if (thinking) config.thinking = thinking
  return JSON.stringify(config, null, 2)
}

function buildThinkingPayload(model) {
  const providerName = resolveLLMProvider(model?.provider, model?.type)
  const config = getProviderThinkingConfig(providerName, model?.model_name)
  if (!config?.visible) {
    return undefined
  }

  const mode = model?.thinking_mode || defaultThinkingMode
  if (mode === defaultThinkingMode) {
    return undefined
  }

  const payload = { mode }
  if ((config.showBudgetFor || []).includes(mode) && model?.thinking_budget_tokens !== null && model?.thinking_budget_tokens !== undefined && model?.thinking_budget_tokens !== '') {
    payload.budget_tokens = Number(model.thinking_budget_tokens)
  }
  if ((config.showEffortFor || []).includes(mode) && model?.thinking_effort) {
    payload.effort = model.thinking_effort
  }
  if ((config.showClearThinkingFor || []).includes(mode) && typeof model?.thinking_clear_thinking === 'boolean') {
    payload.clear_thinking = model.thinking_clear_thinking
  }
  return payload
}

function validate(callback) {
  if (callback) {
    return formRef.value?.validate((valid) => {
      let finalValid = valid
      if (finalValid && isCoze.value && !props.model?.bot_id) {
        finalValid = false
      }
      if (finalValid && thinkingConfig.value.showBudget && thinkingConfig.value.budgetRequired && (props.model?.thinking_budget_tokens === null || props.model?.thinking_budget_tokens === undefined || props.model?.thinking_budget_tokens === '')) {
        finalValid = false
      }
      if (finalValid && thinkingConfig.value.showBudget && props.model?.thinking_budget_tokens !== null && props.model?.thinking_budget_tokens !== undefined && props.model?.thinking_budget_tokens !== '' && Number(props.model?.thinking_budget_tokens) < thinkingConfig.value.budgetMin) {
        finalValid = false
      }
      callback(finalValid)
      return finalValid
    })
  }

  return formRef.value?.validate().then(() => {
    if (isCoze.value && !props.model?.bot_id) {
      return Promise.reject(new Error('请输入Coze Bot ID'))
    }
    if (thinkingConfig.value.showBudget && thinkingConfig.value.budgetRequired && (props.model?.thinking_budget_tokens === null || props.model?.thinking_budget_tokens === undefined || props.model?.thinking_budget_tokens === '')) {
      return Promise.reject(new Error('请填写思考预算'))
    }
    if (thinkingConfig.value.showBudget && props.model?.thinking_budget_tokens !== null && props.model?.thinking_budget_tokens !== undefined && props.model?.thinking_budget_tokens !== '' && Number(props.model?.thinking_budget_tokens) < thinkingConfig.value.budgetMin) {
      return Promise.reject(new Error(`思考预算不能小于 ${thinkingConfig.value.budgetMin}`))
    }
    return true
  })
}

function resetFields() {
  formRef.value?.resetFields()
}

defineExpose({ validate, getJsonData, resetFields })
</script>
