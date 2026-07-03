<template>
  <div class="config-page">
    <div class="page-actions">
      <el-button @click="loadSettings" :loading="loading">刷新</el-button>
      <el-button type="primary" @click="saveSettings" :loading="saving">保存设置</el-button>
    </div>

    <el-card v-loading="loading">
      <el-form ref="formRef" :model="form" :rules="rules" label-width="180px" style="max-width: 720px;">
        <el-divider content-position="left">身份验证</el-divider>
        <el-form-item label="启用设备激活验证" prop="auth.enable">
          <el-switch v-model="form.auth.enable" />
        </el-form-item>
        <el-form-item label="登录数字验证" prop="auth.login_captcha_enabled">
          <el-switch
            v-model="form.auth.login_captcha_enabled"
            active-text="开启"
            inactive-text="关闭"
          />
          <div class="form-help">
            开启后登录页需要完成数字算术题；关闭后登录只校验用户名和密码。默认开启。
          </div>
        </el-form-item>

        <el-divider content-position="left">聊天参数</el-divider>
        <el-form-item label="会话最大空闲时间(ms)" prop="chat.max_idle_duration">
          <el-input-number v-model="form.chat.max_idle_duration" :min="0" :step="1000" style="width: 100%;" />
          <div class="form-help">
            单位毫秒。设置为 0 表示不限制会话空闲时长（不会因空闲自动断开）。建议值：30000~120000。
          </div>
        </el-form-item>
        <el-form-item label="句子结束静音阈值(ms)" prop="chat.chat_max_silence_duration">
          <el-input-number v-model="form.chat.chat_max_silence_duration" :min="0" :step="10" style="width: 100%;" />
          <div class="form-help">
            用于判定一句话结束：从“有声”转为“静音”持续达到该阈值后，认为句子结束并触发后续处理。默认 400ms。阈值越小响应越快但更易截断，阈值越大更稳但响应更慢，建议 300~600ms。
          </div>
        </el-form-item>
        <el-form-item label="实时打断模式" prop="chat.realtime_mode">
          <el-select v-model="form.chat.realtime_mode" style="width: 100%;">
            <el-option :value="1" label="1 - vad打断模式" />
            <el-option :value="2" label="2 - asr打断模式" />
            <el-option :value="3" label="3 - asr识别到声纹时打断" />
            <el-option :value="4" label="4 - asr出结果打断" />
          </el-select>
        </el-form-item>
        <el-form-item label="全局System Prompt描述" prop="chat.global_system_prompt">
          <el-input
            v-model="form.chat.global_system_prompt"
            type="textarea"
            :rows="6"
            maxlength="8000"
            show-word-limit
            placeholder="该内容会在系统提示词最前面拼接，建议填写平台级约束与身份设定。"
          />
          <div class="form-help">
            生效顺序：全局System Prompt描述 → 角色/设备提示词 → 时间/记忆等运行时信息。
          </div>
        </el-form-item>
      </el-form>
    </el-card>
  </div>
</template>

<script setup>
import { onMounted, reactive, ref } from 'vue'
import { ElMessage } from 'element-plus'
import api from '../../utils/api'

const loading = ref(false)
const saving = ref(false)
const formRef = ref()

const form = reactive({
  auth: {
    enable: false,
    login_captcha_enabled: true
  },
  chat: {
    max_idle_duration: 30000,
    chat_max_silence_duration: 400,
    realtime_mode: 4,
    global_system_prompt: ''
  }
})

const rules = {
  'chat.max_idle_duration': [
    { required: true, message: '请输入会话最大空闲时间', trigger: 'blur' }
  ],
  'chat.chat_max_silence_duration': [
    { required: true, message: '请输入句子结束静音阈值', trigger: 'blur' }
  ],
  'chat.realtime_mode': [
    { required: true, message: '请选择实时打断模式', trigger: 'change' }
  ],
  'chat.global_system_prompt': [
    { max: 8000, message: '全局System Prompt描述不能超过8000个字符', trigger: 'blur' }
  ]
}

const loadSettings = async () => {
  loading.value = true
  try {
    const res = await api.get('/admin/chat-settings')
    const data = res.data?.data || {}
    form.auth.enable = !!data.auth?.enable
    form.auth.login_captcha_enabled = data.auth?.login_captcha_enabled !== false
    form.chat.max_idle_duration = Number(data.chat?.max_idle_duration ?? 30000)
    form.chat.chat_max_silence_duration = Number(data.chat?.chat_max_silence_duration ?? 400)
    form.chat.realtime_mode = Number(data.chat?.realtime_mode ?? 4)
    form.chat.global_system_prompt = String(data.chat?.global_system_prompt ?? '')
  } catch (error) {
    ElMessage.error('加载聊天设置失败')
    console.error(error)
  } finally {
    loading.value = false
  }
}

const saveSettings = async () => {
  if (!formRef.value) return
  const valid = await formRef.value.validate().catch(() => false)
  if (!valid) return

  saving.value = true
  try {
    await api.put('/admin/chat-settings', {
      auth: {
        enable: !!form.auth.enable,
        login_captcha_enabled: form.auth.login_captcha_enabled !== false
      },
      chat: {
        max_idle_duration: Number(form.chat.max_idle_duration),
        chat_max_silence_duration: Number(form.chat.chat_max_silence_duration),
        realtime_mode: Number(form.chat.realtime_mode),
        global_system_prompt: String(form.chat.global_system_prompt || '')
      }
    })
    ElMessage.success('聊天设置保存成功')
  } catch (error) {
    ElMessage.error('聊天设置保存失败')
    console.error(error)
  } finally {
    saving.value = false
  }
}

onMounted(() => {
  loadSettings()
})
</script>

<style scoped>
.config-page {
  padding: 20px;
}

.page-actions {
  display: flex;
  justify-content: flex-end;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  margin-bottom: 20px;
}

.form-help {
  margin-top: 6px;
  color: #909399;
  font-size: 12px;
  line-height: 1.5;
}
</style>
