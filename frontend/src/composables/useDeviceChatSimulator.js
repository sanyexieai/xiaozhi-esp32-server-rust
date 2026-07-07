/**
 * 设备对话模拟器 — 协议常量与 WebSocket 会话管理。
 * 文本模式已实现；语音 / 多模态 / MCP Skill 预留扩展点。
 */

import { computed, ref, shallowRef } from 'vue'
import { OpusTtsPlayer } from '@/utils/opusTtsPlayer'

export const SIMULATOR_FEATURES = {
  textChat: true,
  voiceChat: false,
  multimodal: false,
  mcpSkill: false,
  ttsPlayback: true
}

export const DEFAULT_AUDIO_PARAMS = {
  format: 'opus',
  sample_rate: 16000,
  channels: 1,
  frame_duration: 60
}

export function buildHelloMessage(protocolVersion = 1, features = {}) {
  return {
    type: 'hello',
    version: protocolVersion,
    transport: 'websocket',
    audio_params: { ...DEFAULT_AUDIO_PARAMS },
    features: {
      mcp: false,
      ...features
    }
  }
}

export function buildListenDetect(text) {
  return {
    type: 'listen',
    state: 'detect',
    text: String(text || '').trim()
  }
}

export function buildListenText(text) {
  return {
    type: 'listen',
    state: 'text',
    text: String(text || '').trim()
  }
}

export function buildListenStart(mode = 'auto') {
  return {
    type: 'listen',
    state: 'start',
    mode
  }
}

export function buildListenStop() {
  return {
    type: 'listen',
    state: 'stop'
  }
}

export function buildAbort() {
  return { type: 'abort' }
}

export function buildGoodbye() {
  return { type: 'goodbye' }
}

export function parseServerMessage(raw) {
  try {
    const msg = typeof raw === 'string' ? JSON.parse(raw) : raw
    if (!msg || typeof msg !== 'object') return null
    return msg
  } catch {
    return null
  }
}

export function messageToTranscriptEntry(msg, meta = {}) {
  if (!msg?.type) return null
  const base = {
    id: `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
    ts: new Date().toISOString(),
    rawType: msg.type,
    ...meta
  }

  switch (msg.type) {
    case 'hello':
      return {
        ...base,
        role: 'system',
        kind: 'event',
        title: '握手成功',
        content: msg.session_id ? `session_id: ${msg.session_id}` : '已建立会话'
      }
    case 'stt':
      return {
        ...base,
        role: 'user',
        kind: 'text',
        content: msg.text || ''
      }
    case 'llm':
      return {
        ...base,
        role: 'assistant',
        kind: 'text',
        content: msg.text || ''
      }
    case 'text':
      return {
        ...base,
        role: 'assistant',
        kind: 'text',
        content: msg.text || ''
      }
    case 'tts':
      return {
        ...base,
        role: 'assistant',
        kind: 'event',
        title: 'TTS',
        content: describeTtsState(msg)
      }
    case 'mcp':
      return {
        ...base,
        role: 'system',
        kind: 'mcp',
        title: 'MCP',
        content: JSON.stringify(msg, null, 2)
      }
    case 'speak_request':
      return {
        ...base,
        role: 'system',
        kind: 'event',
        title: 'Speak Request',
        content: msg.text || JSON.stringify(msg)
      }
    case 'goodbye':
      return {
        ...base,
        role: 'system',
        kind: 'event',
        title: '会话结束',
        content: msg.reason || 'goodbye'
      }
    default:
      return {
        ...base,
        role: 'system',
        kind: 'event',
        title: msg.type,
        content: JSON.stringify(msg)
      }
  }
}

function describeTtsState(msg) {
  const state = msg.state || ''
  if (state === 'start') return '开始播报'
  if (state === 'stop') return '播报结束'
  if (state === 'sentence_start') return msg.text ? `句子: ${msg.text}` : '新句子'
  return state || JSON.stringify(msg)
}

function buildProxyUrl({ proxyPath, deviceId, protocolVersion, wsUrlOverride, token }) {
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
  const path = proxyPath.startsWith('/api') ? proxyPath : `/api${proxyPath.startsWith('/') ? '' : '/'}${proxyPath}`
  const params = new URLSearchParams()
  params.set('device_id', deviceId)
  params.set('protocol_version', String(protocolVersion || 1))
  if (wsUrlOverride) params.set('ws_url', wsUrlOverride)
  if (token) params.set('token', token)
  return `${protocol}//${window.location.host}${path}?${params.toString()}`
}

export function useDeviceChatSimulator(options = {}) {
  const configPath = options.configPath || '/admin/device-simulator/config'
  const quietConnect = options.quietConnect === true
  const config = shallowRef(null)
  const connectionState = ref('idle') // idle | connecting | connected | error
  const sessionId = ref('')
  const transcript = ref([])
  const lastError = ref('')
  const binaryFrameCount = ref(0)
  const ttsEnabled = ref(true)
  const ttsPlaying = ref(false)
  const ttsPlayerError = ref('')

  let socket = null
  let ttsPlayer = null
  let protocolVersion = 1
  let activeConnect = null

  const isConnected = computed(() => connectionState.value === 'connected')

  function pushEntry(entry) {
    if (!entry) return
    transcript.value.push(entry)
  }

  function pushSystem(text, title = '系统') {
    pushEntry({
      id: `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
      ts: new Date().toISOString(),
      role: 'system',
      kind: 'event',
      title,
      content: text
    })
  }

  function ensureTtsPlayer(sampleRate = DEFAULT_AUDIO_PARAMS.sample_rate) {
    if (!ttsPlayer) {
      ttsPlayer = new OpusTtsPlayer({
        sampleRate,
        protocolVersion,
        enabled: ttsEnabled.value
      })
    } else {
      ttsPlayer.setProtocolVersion(protocolVersion)
      ttsPlayer.setSampleRate(sampleRate)
      ttsPlayer.setEnabled(ttsEnabled.value)
    }
    return ttsPlayer
  }

  async function disposeTtsPlayer() {
    if (ttsPlayer) {
      await ttsPlayer.dispose()
      ttsPlayer = null
    }
    ttsPlaying.value = false
  }

  function setTtsEnabled(enabled) {
    ttsEnabled.value = enabled !== false
    ttsPlayer?.setEnabled(ttsEnabled.value)
    if (!ttsEnabled.value) {
      ttsPlaying.value = false
    }
  }

  function handleIncomingText(raw) {
    const msg = parseServerMessage(raw)
    if (!msg) {
      pushSystem(String(raw), '原始消息')
      return
    }
    if (msg.type === 'hello') {
      if (msg.session_id) {
        sessionId.value = msg.session_id
      }
      const rate = msg.audio_params?.sample_rate
      if (rate) {
        ensureTtsPlayer(rate)
      }
    }
    if (msg.type === 'tts') {
      ensureTtsPlayer().onTtsState(msg.state)
      if (msg.state === 'start') {
        ttsPlaying.value = true
      }
      if (msg.state === 'stop') {
        ttsPlaying.value = false
      }
    }
    const entry = messageToTranscriptEntry(msg)
    if (entry) pushEntry(entry)
  }

  async function handleIncomingBinary(data) {
    binaryFrameCount.value += 1
    const player = ensureTtsPlayer()
    const played = await player.playBinaryFrame(data)
    if (played) {
      ttsPlaying.value = true
      ttsPlayerError.value = ''
    } else if (player.error) {
      ttsPlayerError.value = player.error
    }
  }

  async function loadConfig(apiClient, overridePath) {
    const path = overridePath || configPath
    const res = await apiClient.get(path)
    config.value = res.data?.data || null
    if (!config.value?.ws_proxy_path) {
      throw new Error('对话配置不完整，请重新编译并重启 xiaozhi-manager')
    }
    return config.value
  }

  function connect({
    deviceId,
    protocolVersion: pv = 1,
    wsUrlOverride = '',
    resumeSessionId = '',
    helloFeatures = {}
  }) {
    if (activeConnect) {
      return activeConnect
    }

    activeConnect = doConnect({
      deviceId,
      protocolVersion: pv,
      wsUrlOverride,
      resumeSessionId,
      helloFeatures
    }).finally(() => {
      activeConnect = null
    })

    return activeConnect
  }

  function doConnect({
    deviceId,
    protocolVersion: pv = 1,
    wsUrlOverride = '',
    resumeSessionId = '',
    helloFeatures = {}
  }) {
    if (!deviceId?.trim()) {
      lastError.value = '请选择或填写设备 ID'
      return Promise.reject(new Error(lastError.value))
    }
    if (!config.value?.ws_proxy_path) {
      lastError.value = '模拟器配置未加载'
      return Promise.reject(new Error(lastError.value))
    }

    connectionState.value = 'connecting'
    lastError.value = ''
    sessionId.value = ''
    binaryFrameCount.value = 0
    ttsPlayerError.value = ''
    protocolVersion = pv
    ensureTtsPlayer()

    const token = localStorage.getItem('token') || ''
    const url = buildProxyUrl({
      proxyPath: config.value.ws_proxy_path,
      deviceId: deviceId.trim(),
      protocolVersion: pv,
      wsUrlOverride: wsUrlOverride?.trim() || '',
      token
    })

    if (socket) {
      try {
        socket.close()
      } catch {
        // ignore
      }
      socket = null
    }

    return new Promise((resolve, reject) => {
      let settled = false
      let helloTimer = null

      const finish = () => {
        if (settled) return
        settled = true
        if (helloTimer) clearTimeout(helloTimer)
        resolve()
      }

      const fail = (error) => {
        if (settled) return
        settled = true
        if (helloTimer) clearTimeout(helloTimer)
        reject(error)
      }

      const onHello = () => finish()

      socket = new WebSocket(url)

      socket.onopen = () => {
        connectionState.value = 'connected'
        if (!quietConnect) {
          pushSystem('已通过管理台代理连接 xiaozhi-server', '已连接')
        }
        const features = { ...helloFeatures }
        const resume = String(resumeSessionId || '').trim()
        if (resume) {
          features.resume_session_id = resume
        }
        const hello = buildHelloMessage(pv, features)
        socket.send(JSON.stringify(hello))
        void ensureTtsPlayer().ensureReady()
        helloTimer = setTimeout(() => {
          fail(
            new Error(
              '等待服务端 hello 响应超时，请确认 xiaozhi-server 已启动（默认 ws://127.0.0.1:8989/xiaozhi/v1/）'
            )
          )
        }, 15000)
      }

      socket.onmessage = (event) => {
        if (typeof event.data === 'string') {
          const msg = parseServerMessage(event.data)
          if (msg?.type === 'hello') {
            onHello()
          }
          handleIncomingText(event.data)
        } else if (event.data instanceof Blob) {
          event.data.arrayBuffer().then((buf) => handleIncomingBinary(buf))
        } else if (event.data instanceof ArrayBuffer) {
          handleIncomingBinary(event.data)
        }
      }

      socket.onerror = () => {
        lastError.value = 'WebSocket 连接异常，请确认 xiaozhi-server 已启动且 OTA 中配置了 WebSocket 地址'
        connectionState.value = 'error'
        fail(new Error(lastError.value))
      }

      socket.onclose = (event) => {
        if (!settled && connectionState.value === 'connecting') {
          lastError.value = `WebSocket 连接已关闭 (${event.code})，请检查服务与设备权限`
          connectionState.value = 'error'
          fail(new Error(lastError.value))
          return
        }
        if (connectionState.value === 'connected' && !quietConnect) {
          pushSystem(`连接已关闭 (${event.code})`, '断开')
        }
        connectionState.value = 'idle'
        socket = null
      }
    })
  }

  function disconnect() {
    activeConnect = null
    if (socket) {
      try {
        socket.close()
      } catch {
        // ignore
      }
      socket = null
    }
    ttsPlayer?.stop()
    void disposeTtsPlayer()
    connectionState.value = 'idle'
  }

  function sendJson(payload) {
    if (!socket || socket.readyState !== WebSocket.OPEN) {
      lastError.value = '未连接'
      return false
    }
    socket.send(JSON.stringify(payload))
    return true
  }

  function sendText(text) {
    const trimmed = String(text || '').trim()
    if (!trimmed) return false
    // Web 端本地回显；MQTT 硬件路径可能不下发 STT
    pushEntry({
      id: `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
      ts: new Date().toISOString(),
      role: 'user',
      kind: 'text',
      content: trimmed
    })
    return sendJson(buildListenText(trimmed))
  }

  function sendAbort() {
    ttsPlayer?.stop()
    ttsPlaying.value = false
    return sendJson(buildAbort())
  }

  function sendGoodbye() {
    const ok = sendJson(buildGoodbye())
    disconnect()
    return ok
  }

  function clearTranscript() {
    transcript.value = []
    binaryFrameCount.value = 0
  }

  /** 预留：语音上行 Opus 帧 */
  function sendAudioFrame(_opusPayload) {
    console.warn('[DeviceChatSimulator] sendAudioFrame 尚未实现')
    return false
  }

  /** 预留：Vision 多模态请求 */
  async function sendVisionRequest(_file, _question) {
    console.warn('[DeviceChatSimulator] sendVisionRequest 尚未实现')
    return { ok: false, message: '多模态能力尚未实现' }
  }

  /** 预留：MCP Skill 工具调用 */
  async function invokeMcpSkill(_toolName, _args) {
    console.warn('[DeviceChatSimulator] invokeMcpSkill 尚未实现')
    return { ok: false, message: 'MCP Skill 尚未实现' }
  }

  return {
    config,
    connectionState,
    sessionId,
    transcript,
    lastError,
    binaryFrameCount,
    ttsEnabled,
    ttsPlaying,
    ttsPlayerError,
    isConnected,
    loadConfig,
    connect,
    disconnect,
    sendText,
    sendAbort,
    sendGoodbye,
    clearTranscript,
    setTtsEnabled,
    sendAudioFrame,
    sendVisionRequest,
    invokeMcpSkill,
    notifySystem: pushSystem,
    buildListenStart,
    buildListenStop
  }
}
