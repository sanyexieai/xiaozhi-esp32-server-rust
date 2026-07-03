/**
 * 设备分步调试：封装 Manager → Server → 设备 链路调用，并记录通信流水。
 */
import { computed, ref } from 'vue'
import api from '@/utils/api'

const PIPELINE_BASE = [
  { key: 'ui', labelAdmin: '管理后台', labelUser: '用户工作台' },
  { key: 'manager', labelAdmin: 'xiaozhi-manager', labelUser: 'xiaozhi-manager' },
  { key: 'server', labelAdmin: 'xiaozhi-server', labelUser: 'xiaozhi-server' },
  { key: 'device', labelAdmin: '设备', labelUser: '设备' }
]

let flowSeq = 0

function nowIso() {
  return new Date().toISOString()
}

function buildPipeline(scope) {
  const isUser = scope === 'user'
  return PIPELINE_BASE.map((node) => ({
    key: node.key,
    label: isUser ? node.labelUser : node.labelAdmin
  }))
}

function deviceApiPrefix(scope) {
  return scope === 'user' ? '/user/devices' : '/admin/devices'
}

/**
 * 根据端点快照与 WS 状态推导各调试按钮是否可用及提示文案。
 */
export function deriveDebugCapabilities(status, ws = {}) {
  const s = status || {}
  const wsConnected = !!ws.connected
  const wsTtsPlaying = !!ws.ttsPlaying

  const online = !!s.online
  const hasHardware = !!s.has_hardware
  const hasWeb = !!s.has_web
  const mqttOnline = s.mqtt_broker_online === true
  const mqttKnown = s.mqtt_broker_online != null
  const hasUdp = s.has_udp_session === true
  const helloInited = s.hello_inited === true
  const ttsActive = !!s.tts_active || !!s.is_speaking
  const serverError = s.error && !online

  const canHotSpeak = hasHardware && online && (hasUdp || helloInited)
  const canColdWake = hasHardware && mqttOnline && !canHotSpeak
  const canWake = canHotSpeak || canColdWake
  const canSpeakWeb = hasWeb && online
  const canSpeak = canHotSpeak || canSpeakWeb
  const canChat = online && canSpeak
  const canAbortHw = hasHardware && online && ttsActive
  const canGoodbyeHw = hasHardware && online && (mqttOnline || helloInited)
  const canWsAbort = wsConnected && wsTtsPlaying
  const canWsGoodbye = wsConnected
  const canWsSend = wsConnected

  const speakTarget = canHotSpeak ? 'hardware_first' : canSpeakWeb ? 'web' : 'hardware_first'

  let wakeHint = ''
  if (!hasHardware) {
    wakeHint = '无硬件端点，请用下方 WebSocket 模拟'
  } else if (canHotSpeak) {
    wakeHint = 'UDP/MQTT 热链路已就绪，可直接播报'
  } else if (canColdWake) {
    wakeHint = '冷启动：依赖 speak_request→speak_ready（当前固件可能超时）'
  } else if (mqttKnown && !mqttOnline) {
    wakeHint = 'MQTT broker 离线，无法远程唤醒'
  } else {
    wakeHint = '硬件未在线或未建立会话'
  }

  let speakHint = ''
  if (!canSpeak) {
    if (hasHardware && !canHotSpeak) {
      speakHint = '需设备已 hello 且 UDP 热链路，或本地唤醒后再试'
    } else if (!online) {
      speakHint = '设备离线，请先本地唤醒或连接 Web 端点'
    } else {
      speakHint = '无可用播报端点'
    }
  }

  let abortHint = ''
  if (!canAbortHw) {
    abortHint = ttsActive ? '状态未同步，请先刷新状态' : '当前无硬件播放中会话'
  } else {
    abortHint = '下发 abort，停止 TTS 播放'
  }

  let goodbyeHint = ''
  if (!canGoodbyeHw) {
    goodbyeHint = hasHardware
      ? '需 MQTT 在线或已 hello，且硬件端点存在'
      : '无硬件端点'
  } else {
    goodbyeHint = '下发 goodbye，设备应关闭通道并回主页'
  }

  return {
    online,
    hasHardware,
    hasWeb,
    mqttOnline,
    hasUdp,
    helloInited,
    ttsActive,
    canHotSpeak,
    canColdWake,
    canWake,
    canSpeak,
    canSpeakWeb,
    canChat,
    canAbortHw,
    canGoodbyeHw,
    canWsAbort,
    canWsGoodbye,
    canWsSend,
    speakTarget,
    serverError,
    wakeHint,
    speakHint,
    abortHint,
    goodbyeHint,
    wsAbortHint: canWsAbort ? '向 WS 会话发送 abort' : '需已连接且正在播放 TTS',
    wsGoodbyeHint: canWsGoodbye ? '向 WS 会话发送 goodbye 并断开' : '需先连接 WebSocket'
  }
}

export function useDeviceDebug(options = {}) {
  const scope = options.scope === 'user' ? 'user' : 'admin'
  const apiPrefix = deviceApiPrefix(scope)
  const PIPELINE = computed(() => buildPipeline(scope))

  const flowLog = ref([])
  const endpointStatus = ref(null)
  const busy = ref(false)
  const activePhase = ref('ui')

  function appendFlow(entry) {
    flowSeq += 1
    flowLog.value.push({
      id: flowSeq,
      ts: nowIso(),
      ok: true,
      ...entry
    })
    if (entry.phase) {
      activePhase.value = entry.phase
    }
  }

  function clearFlow() {
    flowLog.value = []
    flowSeq = 0
    activePhase.value = 'ui'
    endpointStatus.value = null
  }

  async function runStep(step, phase, fn) {
    busy.value = true
    const t0 = performance.now()
    try {
      const result = await fn()
      appendFlow({
        step,
        phase,
        direction: 'out',
        summary: result.summary,
        ok: result.ok !== false,
        request: result.request,
        response: result.response,
        durationMs: Math.round(performance.now() - t0)
      })
      return result
    } catch (e) {
      const msg = e?.response?.data?.error || e?.message || String(e)
      appendFlow({
        step,
        phase,
        direction: 'out',
        summary: `失败: ${msg}`,
        ok: false,
        response: e?.response?.data || { error: msg },
        durationMs: Math.round(performance.now() - t0)
      })
      throw e
    } finally {
      busy.value = false
    }
  }

  async function refreshStatus(deviceDbId) {
    const path = `${apiPrefix}/${deviceDbId}/endpoints`
    return runStep('status', 'manager', async () => {
      const res = await api.get(path)
      const data = res.data?.data || {}
      endpointStatus.value = data
      const hw = data.has_hardware ? '硬件在线' : '硬件离线'
      const web = data.has_web ? 'Web在线' : 'Web离线'
      const mqtt =
        data.mqtt_broker_online === true
          ? 'MQTT在线'
          : data.mqtt_broker_online === false
            ? 'MQTT离线'
            : 'MQTT未知'
      const udp = data.has_udp_session ? 'UDP已建立' : 'UDP未建立'
      const play = data.tts_active || data.is_speaking ? ' · 播放中' : ''
      return {
        summary: `端点 ${data.endpoint_count ?? 0} · ${hw} · ${web} · ${mqtt} · ${udp}${play}`,
        request: { method: 'GET', path },
        response: data
      }
    })
  }

  async function wakeDevice(deviceDbId) {
    const path = `${apiPrefix}/${deviceDbId}/speak`
    return runStep('wake', 'server', async () => {
      const body = {
        text: '你好',
        target: 'hardware_first',
        auto_listen: false
      }
      const res = await api.post(path, body, { timeout: 45000 })
      const data = res.data?.data || {}
      const ok = data.success !== false && !data.error
      return {
        ok,
        summary: ok
          ? '已下发唤醒播报（auto_listen:false，播完将 goodbye）'
          : data.error || '唤醒失败',
        request: { method: 'POST', path, body },
        response: data
      }
    })
  }

  async function playText(deviceDbId, text, { target, autoListen = false } = {}) {
    const trimmed = String(text || '').trim()
    if (!trimmed) {
      throw new Error('播报文本不能为空')
    }
    const status = endpointStatus.value
    const resolvedTarget =
      target || deriveDebugCapabilities(status).speakTarget
    const path = `${apiPrefix}/${deviceDbId}/speak`
    return runStep('speak', 'server', async () => {
      const body = { text: trimmed, target: resolvedTarget, auto_listen: autoListen }
      const res = await api.post(path, body, { timeout: 60000 })
      const data = res.data?.data || {}
      const ok = data.success !== false && !data.error
      return {
        ok,
        summary: ok ? `TTS 播报已下发: 「${trimmed.slice(0, 40)}」` : data.error || '播报失败',
        request: { method: 'POST', path, body },
        response: data
      }
    })
  }

  async function chatViaApi(
    deviceId,
    message,
    { skipLlm = false, autoListen = true, target } = {}
  ) {
    const trimmed = String(message || '').trim()
    if (!trimmed) {
      throw new Error('对话内容不能为空')
    }
    const resolvedTarget =
      target || deriveDebugCapabilities(endpointStatus.value).speakTarget
    return runStep(skipLlm ? 'speak' : 'chat_api', 'server', async () => {
      const body = {
        device_id: deviceId,
        message: trimmed,
        skip_llm: skipLlm,
        auto_listen: autoListen,
        target: resolvedTarget
      }
      const res = await api.post('/user/devices/inject-message', body, { timeout: 90000 })
      const data = res.data?.data || {}
      const ok = data.success !== false && !data.error
      return {
        ok,
        summary: ok
          ? skipLlm
            ? `已注入 TTS: 「${trimmed.slice(0, 40)}」`
            : `已提交 LLM 对话: 「${trimmed.slice(0, 40)}」`
          : data.error || '对话失败',
        request: { method: 'POST', path: '/user/devices/inject-message', body },
        response: data
      }
    })
  }

  async function abortPlayback(deviceDbId) {
    const path = `${apiPrefix}/${deviceDbId}/abort`
    return runStep('abort', 'server', async () => {
      const res = await api.post(path, {}, { timeout: 15000 })
      const data = res.data?.data || {}
      const ok = data.success !== false && !data.error
      return {
        ok,
        summary: ok ? '已下发打断（abort / tts stop）' : data.error || '打断失败',
        request: { method: 'POST', path },
        response: data
      }
    })
  }

  async function returnToHome(deviceDbId) {
    const path = `${apiPrefix}/${deviceDbId}/goodbye`
    return runStep('goodbye', 'server', async () => {
      const res = await api.post(path, {}, { timeout: 15000 })
      const data = res.data?.data || {}
      const ok = data.success !== false && !data.error
      return {
        ok,
        summary: ok ? '已下发 goodbye，设备应回主页' : data.error || '回主页失败',
        request: { method: 'POST', path },
        response: data
      }
    })
  }

  function logWsEvent(summary, payload, ok = true) {
    appendFlow({
      step: 'chat_ws',
      phase: 'device',
      direction: 'in',
      summary,
      ok,
      response: payload
    })
  }

  function logWsSend(summary, payload) {
    appendFlow({
      step: 'chat_ws',
      phase: 'server',
      direction: 'out',
      summary,
      ok: true,
      request: payload
    })
  }

  const signalLog = ref([])
  let lastSignalId = 0
  let signalPollTimer = null

  function appendSignals(entries) {
    if (!Array.isArray(entries) || entries.length === 0) return
    for (const row of entries) {
      signalLog.value.push(row)
      if (row.id > lastSignalId) {
        lastSignalId = row.id
      }
    }
    if (signalLog.value.length > 600) {
      signalLog.value = signalLog.value.slice(-500)
    }
  }

  async function fetchSignals(deviceDbId, { clear = false } = {}) {
    const params = clear ? { clear: true } : { after_id: lastSignalId }
    const path = `${apiPrefix}/${deviceDbId}/signals`
    const res = await api.get(path, { params, timeout: 10000 })
    const data = res.data?.data || {}
    if (clear) {
      signalLog.value = []
      lastSignalId = 0
    }
    appendSignals(data.signals || [])
    return data
  }

  function startSignalPolling(deviceDbId, intervalMs = 1000) {
    stopSignalPolling()
    if (!deviceDbId) return
    const tick = async () => {
      try {
        await fetchSignals(deviceDbId)
      } catch {
        /* 轮询失败不打断调试 */
      }
    }
    tick()
    signalPollTimer = setInterval(tick, intervalMs)
  }

  function stopSignalPolling() {
    if (signalPollTimer) {
      clearInterval(signalPollTimer)
      signalPollTimer = null
    }
  }

  function clearSignalLogLocal() {
    signalLog.value = []
    lastSignalId = 0
  }

  async function clearSignals(deviceDbId) {
    clearSignalLogLocal()
    if (deviceDbId) {
      await fetchSignals(deviceDbId, { clear: true })
    }
  }

  return {
    PIPELINE,
    flowLog,
    signalLog,
    endpointStatus,
    busy,
    activePhase,
    clearFlow,
    refreshStatus,
    wakeDevice,
    playText,
    chatViaApi,
    abortPlayback,
    returnToHome,
    logWsEvent,
    logWsSend,
    fetchSignals,
    startSignalPolling,
    stopSignalPolling,
    clearSignals
  }
}
