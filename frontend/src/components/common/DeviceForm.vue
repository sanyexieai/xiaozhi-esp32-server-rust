<template>
  <el-form
    ref="formRef"
    :model="form"
    :rules="rules"
    :label-position="labelPosition"
    :label-width="labelWidth"
    class="shared-device-form"
  >
    <el-form-item v-if="isAdmin" label="所属用户" prop="user_id">
      <el-select
        v-model="form.user_id"
        placeholder="请选择所属用户"
        filterable
        style="width: 100%"
        :loading="loading.users"
      >
        <el-option
          v-for="user in users"
          :key="user.id"
          :label="userLabel(user)"
          :value="user.id"
        />
      </el-select>
    </el-form-item>

    <el-form-item v-if="isBindMode && !hasFixedAgent" label="目标智能体" prop="agent_id">
      <el-select v-model="form.agent_id" placeholder="请选择要绑定的智能体" filterable style="width: 100%">
        <el-option
          v-for="agent in displayAgents"
          :key="agent.id"
          :label="agent.name || `智能体 #${agent.id}`"
          :value="agent.id"
        />
      </el-select>
    </el-form-item>

    <el-form-item v-if="isBindMode" label="设备验证码或 MAC" prop="identifier">
      <el-input
        v-model="form.identifier"
        placeholder="请输入 6 位验证码或设备 MAC"
        clearable
        autocomplete="off"
      />
      <div class="form-hint">
        <span>示例：</span>
        <code>123456</code>
        <code>28:0A:C6:1D:3B:E8</code>
      </div>
    </el-form-item>

    <el-form-item label="设备昵称" prop="nick_name">
      <el-input
        v-model="form.nick_name"
        placeholder="例如：客厅音箱、办公室小智"
        maxlength="50"
        show-word-limit
        clearable
      />
    </el-form-item>

    <template v-if="!isBindMode">
      <div class="device-form-grid">
        <el-form-item label="设备标识" prop="device_name">
          <el-input
            v-model="form.device_name"
            placeholder="设备端上报的 Device-ID / MAC"
            clearable
          />
        </el-form-item>
        <el-form-item label="激活码" prop="device_code">
          <el-input v-model="form.device_code" placeholder="设备激活码" clearable />
        </el-form-item>
      </div>

      <div class="device-form-grid">
        <el-form-item v-if="isAdmin" label="激活状态" prop="activated">
          <el-switch v-model="form.activated" />
        </el-form-item>
        <el-form-item label="关联智能体" prop="agent_id">
          <el-select
            v-model="form.agent_id"
            placeholder="请选择智能体"
            style="width: 100%"
            clearable
            filterable
            :disabled="isAdmin && !form.user_id"
            :loading="loading.agents"
          >
            <el-option label="不关联智能体" :value="0" />
            <el-option
              v-for="agent in displayAgents"
              :key="agent.id"
              :label="agentLabel(agent)"
              :value="agent.id"
            />
          </el-select>
        </el-form-item>
      </div>
    </template>
  </el-form>
</template>

<script setup>
import { computed, onMounted, ref, watch } from 'vue'
import {
  buildDevicePayload,
  useAgentFormOptions
} from '../../composables/useAgentFormOptions'

const props = defineProps({
  modelValue: {
    type: Object,
    required: true
  },
  isAdmin: {
    type: Boolean,
    default: false
  },
  mode: {
    type: String,
    default: 'create'
  },
  fixedAgentId: {
    type: [Number, String, null],
    default: null
  },
  agents: {
    type: Array,
    default: () => []
  },
  labelPosition: {
    type: String,
    default: 'top'
  },
  labelWidth: {
    type: String,
    default: '110px'
  }
})

const emit = defineEmits(['update:modelValue'])

const form = computed({
  get: () => props.modelValue,
  set: (value) => emit('update:modelValue', value)
})

const formRef = ref(null)
const targetUserId = computed(() => props.isAdmin ? Number(form.value.user_id || 0) : 0)
const isBindMode = computed(() => props.mode === 'bind')
const hasFixedAgent = computed(() => props.fixedAgentId !== null && props.fixedAgentId !== undefined && props.fixedAgentId !== '')

const {
  users,
  agents,
  loading,
  loadUsers,
  loadAgents
} = useAgentFormOptions({
  isAdmin: computed(() => props.isAdmin),
  targetUserId
})

const displayAgents = computed(() => {
  const source = props.agents.length ? props.agents : agents.value
  if (props.isAdmin && targetUserId.value) {
    return source.filter((agent) => Number(agent.user_id) === targetUserId.value)
  }
  return source
})

const agentLabel = (agent) => {
  if (!props.isAdmin) return agent.name || `智能体 #${agent.id}`
  const username = agent.username ? ` · ${agent.username}` : ''
  return `${agent.name || `智能体 #${agent.id}`} (用户${agent.user_id}${username})`
}

const userLabel = (user) => {
  const name = user?.username || user?.name || `用户 #${user?.id}`
  return `${name} (ID: ${user?.id})`
}

const validateIdentifier = (_, value, callback) => {
  if (!isBindMode.value) {
    callback()
    return
  }
  if (!String(value || '').trim()) {
    callback(new Error('请输入设备验证码或设备 MAC'))
    return
  }
  callback()
}

const validateDeviceIdentity = (_, value, callback) => {
  if (isBindMode.value) {
    callback()
    return
  }
  const deviceName = String(form.value.device_name || '').trim()
  const deviceCode = String(form.value.device_code || '').trim()
  if (props.isAdmin) {
    if (!deviceName && !deviceCode) {
      callback(new Error('设备标识和激活码至少填写一个'))
      return
    }
  } else if (!deviceName) {
    callback(new Error('请输入设备标识'))
    return
  }
  callback()
}

const rules = computed(() => ({
  user_id: props.isAdmin ? [{ required: true, message: '请选择所属用户', trigger: 'change' }] : [],
  agent_id: isBindMode.value && !hasFixedAgent.value
    ? [{ required: true, message: '请选择目标智能体', trigger: 'change' }]
    : [],
  identifier: [{ validator: validateIdentifier, trigger: 'blur' }],
  nick_name: [{ max: 50, message: '设备昵称最多 50 个字符', trigger: 'blur' }],
  device_name: [{ validator: validateDeviceIdentity, trigger: 'blur' }],
  device_code: [{ validator: validateDeviceIdentity, trigger: 'blur' }]
}))

const reloadOptions = async () => {
  await Promise.all([
    props.isAdmin ? loadUsers().catch(() => []) : Promise.resolve([]),
    loadAgents().catch(() => [])
  ])
}

watch(
  () => form.value.user_id,
  async (next, prev) => {
    if (!props.isAdmin || next === prev) return
    form.value.agent_id = 0
    await loadAgents().catch(() => [])
  }
)

watch(
  () => props.fixedAgentId,
  (value) => {
    if (hasFixedAgent.value) {
      form.value.agent_id = Number(value)
    }
  },
  { immediate: true }
)

onMounted(() => {
  reloadOptions()
})

const validate = () => formRef.value?.validate?.()
const resetFields = () => formRef.value?.resetFields?.()
const clearValidate = () => formRef.value?.clearValidate?.()
const buildPayload = () => buildDevicePayload(form.value, { isAdmin: props.isAdmin, mode: props.mode })

defineExpose({
  validate,
  resetFields,
  clearValidate,
  reloadOptions,
  buildPayload
})
</script>

<style scoped>
.shared-device-form {
  display: grid;
  gap: 2px;
}

.device-form-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 14px;
}

.form-hint {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  margin-top: 8px;
  color: #909399;
  font-size: 12px;
}

.form-hint code {
  padding: 4px 8px;
  border-radius: 8px;
  background: #f5f7fa;
  color: #606266;
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, 'Liberation Mono', 'Courier New', monospace;
}

@media (max-width: 760px) {
  .device-form-grid {
    grid-template-columns: 1fr;
  }
}
</style>
