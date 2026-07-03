import { ref } from 'vue'
import api from '@/utils/api'

function resolveDeviceKey(device) {
  if (!device) return ''
  return String(device.device_name || device.device_id || '').trim()
}

/**
 * 从 xiaozhi-server 批量拉取设备会话端点状态（硬件 / Web 多端在线）。
 */
export function useDeviceLiveStatus(apiPath = '/admin/devices/live-status') {
  const liveByDeviceId = ref({})
  const loading = ref(false)

  async function refresh(deviceList = [], { deviceIds } = {}) {
    const ids = (deviceIds?.length
      ? deviceIds
      : (Array.isArray(deviceList) ? deviceList : []).map(resolveDeviceKey)
    ).filter(Boolean)

    if (!ids.length) {
      liveByDeviceId.value = {}
      return liveByDeviceId.value
    }

    loading.value = true
    try {
      const res = await api.post(
        apiPath,
        { device_ids: ids },
        { timeout: 20000, silentError: true }
      )
      liveByDeviceId.value = res.data?.data?.devices || {}
    } catch {
      liveByDeviceId.value = {}
    } finally {
      loading.value = false
    }
    return liveByDeviceId.value
  }

  function getLive(device) {
    const key = resolveDeviceKey(device)
    if (!key) return null
    return liveByDeviceId.value[key] || null
  }

  function isSessionOnline(device) {
    const live = getLive(device)
    return Boolean(live?.online)
  }

  return {
    liveByDeviceId,
    loading,
    refresh,
    getLive,
    isSessionOnline,
    resolveDeviceKey
  }
}
