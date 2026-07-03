/**
 * 设备 TTS 下行：解包 BinaryProtocol v1/v2/v3 → Opus 解码 → Web Audio 播放。
 * 对齐 crates/xiaozhi-protocol/src/binary_audio.rs
 */

import { OpusDecoder } from 'opus-decoder'

const BINARY_PROTOCOL2_HEADER = 16
const BINARY_PROTOCOL3_HEADER = 4

export function unpackDeviceAudio(data, protocolVersion = 1) {
  const bytes = data instanceof Uint8Array ? data : new Uint8Array(data)
  if (!bytes.length) return bytes

  if (protocolVersion === 2) {
    return unpackV2(bytes) || bytes
  }
  if (protocolVersion === 3) {
    return unpackV3(bytes) || bytes
  }
  return bytes
}

function unpackV2(data) {
  if (data.length < BINARY_PROTOCOL2_HEADER) return null
  const version = (data[0] << 8) | data[1]
  if (version !== 2) return null
  const payloadSize =
    (data[12] << 24) | (data[13] << 16) | (data[14] << 8) | data[15]
  const end = BINARY_PROTOCOL2_HEADER + payloadSize
  if (end !== data.length) return null
  return data.subarray(BINARY_PROTOCOL2_HEADER, end)
}

function unpackV3(data) {
  if (data.length < BINARY_PROTOCOL3_HEADER) return null
  if (data[0] !== 0) return null
  const payloadSize = (data[2] << 8) | data[3]
  const end = BINARY_PROTOCOL3_HEADER + payloadSize
  if (end !== data.length) return null
  return data.subarray(BINARY_PROTOCOL3_HEADER, end)
}

export class OpusTtsPlayer {
  constructor(options = {}) {
    this.sampleRate = options.sampleRate || 16000
    this.protocolVersion = options.protocolVersion || 1
    this.enabled = options.enabled !== false
    this.audioContext = null
    this.decoder = null
    this.readyPromise = null
    this.nextPlayTime = 0
    this.activeSources = new Set()
    this.frameCount = 0
    this.error = ''
  }

  setProtocolVersion(version) {
    this.protocolVersion = version || 1
  }

  setSampleRate(rate) {
    if (rate && rate !== this.sampleRate) {
      this.sampleRate = rate
      this.decoder = null
      this.readyPromise = null
    }
  }

  setEnabled(enabled) {
    this.enabled = enabled !== false
    if (!this.enabled) {
      this.stop()
    }
  }

  async ensureReady() {
    if (!this.enabled) return false
    if (this.readyPromise) {
      await this.readyPromise
      return true
    }

    this.readyPromise = (async () => {
      if (!this.audioContext || this.audioContext.state === 'closed') {
        this.audioContext = new (window.AudioContext || window.webkitAudioContext)()
      }
      if (this.audioContext.state === 'suspended') {
        await this.audioContext.resume()
      }
      if (!this.decoder) {
        this.decoder = new OpusDecoder({
          sampleRate: this.sampleRate,
          channels: 1
        })
        await this.decoder.ready
      }
      if (this.nextPlayTime < this.audioContext.currentTime) {
        this.nextPlayTime = this.audioContext.currentTime
      }
    })()

    try {
      await this.readyPromise
      this.error = ''
      return true
    } catch (e) {
      this.error = e?.message || '音频初始化失败'
      this.readyPromise = null
      return false
    }
  }

  onTtsState(state) {
    if (state === 'start') {
      this.resetSchedule()
    }
    if (state === 'stop') {
      // 保留已排队帧自然播完
    }
  }

  resetSchedule() {
    if (this.audioContext && this.audioContext.state !== 'closed') {
      this.nextPlayTime = this.audioContext.currentTime
    } else {
      this.nextPlayTime = 0
    }
  }

  stop() {
    for (const source of this.activeSources) {
      try {
        source.stop()
      } catch {
        // already stopped
      }
    }
    this.activeSources.clear()
    this.resetSchedule()
  }

  async playBinaryFrame(arrayBuffer) {
    if (!this.enabled) return false
    const ok = await this.ensureReady()
    if (!ok || !this.decoder || !this.audioContext) return false

    const opus = unpackDeviceAudio(arrayBuffer, this.protocolVersion)
    if (!opus?.length) return false

    let decoded
    try {
      decoded = this.decoder.decodeFrame(opus)
      if (decoded?.then) {
        decoded = await decoded
      }
    } catch (e) {
      this.error = e?.message || 'Opus 解码失败'
      return false
    }

    const samplesDecoded = decoded?.samplesDecoded || 0
    const channel = decoded?.channelData?.[0]
    if (!samplesDecoded || !channel) return false

    const playRate = decoded.sampleRate || this.sampleRate
    const buffer = this.audioContext.createBuffer(1, samplesDecoded, playRate)
    buffer.copyToChannel(channel, 0)

    const source = this.audioContext.createBufferSource()
    source.buffer = buffer
    source.connect(this.audioContext.destination)

    const startAt = Math.max(this.nextPlayTime, this.audioContext.currentTime)
    source.start(startAt)
    this.nextPlayTime = startAt + buffer.duration

    this.activeSources.add(source)
    source.onended = () => {
      this.activeSources.delete(source)
    }

    this.frameCount += 1
    return true
  }

  async dispose() {
    this.stop()
    if (this.decoder?.free) {
      try {
        await this.decoder.free()
      } catch {
        // ignore
      }
    }
    this.decoder = null
    if (this.audioContext && this.audioContext.state !== 'closed') {
      try {
        await this.audioContext.close()
      } catch {
        // ignore
      }
    }
    this.audioContext = null
    this.readyPromise = null
    this.frameCount = 0
  }
}
