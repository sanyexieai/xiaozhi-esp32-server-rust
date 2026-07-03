<template>
  <el-dialog
    v-model="visible"
    title="语音推送"
    width="620px"
    class="inject-message-dialog"
    :close-on-click-modal="false"
    @closed="resetForm"
  >
    <el-form
      ref="formRef"
      :model="form"
      :rules="rules"
      label-position="top"
    >
      <el-form-item label="选择设备" prop="device_id">
        <el-select
          v-model="form.device_id"
          placeholder="请选择要推送语音的设备"
          style="width: 100%"
          filterable
          :disabled="deviceSelectDisabled"
          popper-class="inject-device-select-popper"
        >
          <el-option
            v-for="device in devices"
            :key="device.id || device.device_code"
            :label="getDeviceOptionLabel(device)"
            :value="device.device_name || ''"
          >
            <div class="device-option">
              <div class="device-option-header">
                <span class="device-name">{{ getDeviceNickName(device) }}</span>
                <el-tag :type="deviceOnlineTagType(device, resolveLive(device))" size="small">
                  {{ deviceOnlineLabel(device, resolveLive(device)) }}
                </el-tag>
                <DeviceEndpointBadges
                  :live="resolveLive(device)"
                  :show-offline="false"
                  class="inject-device-endpoints"
                />
              </div>
              <div class="device-code">设备ID: {{ getDeviceIdText(device) }}</div>
              <div v-if="device.device_code" class="device-code">激活码: {{ device.device_code }}</div>
              <div class="device-agent">智能体: {{ device.agent_name || '未绑定' }}</div>
            </div>
          </el-option>
        </el-select>
      </el-form-item>

      <el-form-item label="推送内容" prop="message">
        <el-input
          v-model="form.message"
          type="textarea"
          :rows="4"
          placeholder="请输入要推送播报的内容"
          maxlength="500"
          show-word-limit
        />
      </el-form-item>

      <el-form-item label="音频路由" prop="target">
        <el-select v-model="form.target" style="width: 100%">
          <el-option label="优先硬件（默认）" value="hardware_first" />
          <el-option label="仅硬件播报" value="hardware" />
          <el-option label="仅 Web 播报" value="web" />
          <el-option label="全部端点" value="all" />
        </el-select>
      </el-form-item>

      <el-form-item label="直接播报" prop="skip_llm">
        <div class="switch-field">
          <div class="switch-copy">
            <div class="switch-title">{{ directPlayback ? '开启' : '关闭' }}</div>
            <div class="switch-desc">
              {{ directPlayback ? '消息将直接转语音播报，不经过 LLM 推理。' : '消息将先经过 LLM 处理，再进行播报。' }}
            </div>
          </div>
          <el-switch
            v-model="directPlayback"
            inline-prompt
            active-text="开启"
            inactive-text="关闭"
          />
        </div>
      </el-form-item>

      <el-form-item label="是否转空闲" prop="auto_listen">
        <div class="switch-field">
          <div class="switch-copy">
            <div class="switch-title">{{ returnToIdleAfterPlayback ? '开启' : '关闭' }}</div>
            <div class="switch-desc">
              {{ returnToIdleAfterPlayback ? '播报完成后回到空闲，适合广播通知和单向播报。' : '播报完成后继续监听，可直接进入下一轮对话。' }}
            </div>
          </div>
          <el-switch
            v-model="returnToIdleAfterPlayback"
            inline-prompt
            active-text="开启"
            inactive-text="关闭"
          />
        </div>
      </el-form-item>
    </el-form>

    <template #footer>
      <div class="dialog-footer">
        <el-button @click="handleClose">取消</el-button>
        <el-button type="primary" :loading="submitting" @click="handleSubmit">
          {{ submitting ? '推送中...' : '语音推送' }}
        </el-button>
      </div>
    </template>
  </el-dialog>
</template>

<script setup>
import { computed, reactive, ref, watch } from 'vue'
import { ElMessage } from 'element-plus'
import api from '../../utils/api'
import { deviceOnlineLabel, deviceOnlineTagType } from '../../utils/deviceStatus'
import DeviceEndpointBadges from '../device/DeviceEndpointBadges.vue'

const props = defineProps({
  modelValue: {
    type: Boolean,
    default: false
  },
  devices: {
    type: Array,
    default: () => []
  },
  defaultDeviceId: {
    type: String,
    default: ''
  },
  lockDevice: {
    type: Boolean,
    default: false
  },
  getLive: {
    type: Function,
    default: null
  }
})

const emit = defineEmits(['update:modelValue', 'success'])

const formRef = ref()
const submitting = ref(false)
const visible = computed({
  get: () => props.modelValue,
  set: (value) => emit('update:modelValue', value)
})

const directPlayback = computed({
  get: () => form.skip_llm,
  set: (value) => {
    form.skip_llm = value
  }
})

const returnToIdleAfterPlayback = computed({
  get: () => !form.auto_listen,
  set: (value) => {
    form.auto_listen = !value
  }
})

const deviceSelectDisabled = computed(() => props.lockDevice && !!props.defaultDeviceId)

const form = reactive({
  device_id: '',
  message: '',
  skip_llm: false,
  auto_listen: true,
  target: 'hardware_first'
})

const resolveLive = (device) => {
  if (typeof props.getLive === 'function') {
    return props.getLive(device)
  }
  return null
}

const rules = {
  device_id: [
    { required: true, message: '请选择设备', trigger: 'change' }
  ],
  message: [
    { required: true, message: '请输入推送内容', trigger: 'blur' },
    { min: 1, max: 500, message: '推送内容需在 1-500 个字符之间', trigger: 'blur' }
  ]
}

const getDeviceNickName = (device) => {
  const nickName = String(device?.nick_name || '').trim()
  if (nickName) return nickName
  return String(device?.device_name || '').trim() || '未命名设备'
}

const getDeviceIdText = (device) => String(device?.device_name || '').trim() || '-'

const getDeviceOptionLabel = (device) => {
  const nickName = getDeviceNickName(device)
  const deviceId = getDeviceIdText(device)
  return `${nickName} (${deviceId})`
}

const resetForm = () => {
  form.device_id = props.defaultDeviceId || ''
  form.message = ''
  form.skip_llm = false
  form.auto_listen = true
  form.target = 'hardware_first'
  formRef.value?.clearValidate?.()
}

watch(
  () => [props.modelValue, props.defaultDeviceId],
  ([visible]) => {
    if (!visible) return
    resetForm()
  }
)

const closeDialog = () => {
  visible.value = false
}

const handleSubmit = async () => {
  if (!formRef.value) return

  try {
    await formRef.value.validate()
  } catch {
    return
  }

  submitting.value = true
  try {
    const response = await api.post('/user/devices/inject-message', {
      device_id: form.device_id,
      message: form.message,
      skip_llm: form.skip_llm,
      auto_listen: form.auto_listen,
      target: form.target
    })
    const payload = response.data?.data
    if (payload?.success !== false) {
      ElMessage.success('语音推送成功')
      emit('success', response.data?.data || null)
      closeDialog()
    }
  } catch (error) {
    ElMessage.error(error.response?.data?.error || '语音推送失败')
  } finally {
    submitting.value = false
  }
}

const handleClose = () => {
  resetForm()
  closeDialog()
}
</script>

<style scoped>
.dialog-footer {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
}

.inject-device-endpoints {
  margin-top: 4px;
}

.device-option {
  padding: 8px 0;
}

.device-option-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 4px;
  gap: 12px;
}

.device-name {
  font-weight: 600;
  color: var(--apple-text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.device-code,
.device-agent {
  font-size: 12px;
  color: rgba(107, 114, 128, 0.72);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.switch-field {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  width: 100%;
  padding: 14px 16px;
  border-radius: 18px;
  background: rgba(248, 250, 252, 0.9);
  border: 1px solid rgba(229, 229, 234, 0.72);
}

.switch-copy {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.switch-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--apple-text);
}

.switch-desc {
  font-size: 12px;
  line-height: 1.5;
  color: var(--apple-text-secondary);
}

:deep(.inject-device-select-popper .el-select-dropdown__item) {
  height: auto;
  line-height: 1.4;
  padding-top: 8px;
  padding-bottom: 8px;
  white-space: normal;
}

@media (max-width: 768px) {
  .dialog-footer {
    flex-wrap: wrap;
  }

  .dialog-footer .el-button {
    flex: 1;
    min-width: 120px;
  }

  .switch-field {
    align-items: flex-start;
  }
}
</style>
