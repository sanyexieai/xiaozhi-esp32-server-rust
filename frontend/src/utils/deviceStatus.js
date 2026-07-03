const ONLINE_TTL_MS = 5 * 60 * 1000

/**
 * 判断设备是否在线：优先使用服务端 online 字段，并结合 last_active_at 防止僵尸在线状态。
 */
export function isDeviceOnline(deviceOrLastActiveAt, maybeOnline) {
  let online
  let lastActiveAt

  if (deviceOrLastActiveAt != null && typeof deviceOrLastActiveAt === 'object') {
    online = deviceOrLastActiveAt.online
    lastActiveAt = deviceOrLastActiveAt.last_active_at
  } else {
    lastActiveAt = deviceOrLastActiveAt
    online = maybeOnline
  }

  if (online === false) {
    return false
  }

  if (!lastActiveAt) {
    return online === true
  }

  const last = new Date(lastActiveAt)
  if (Number.isNaN(last.getTime())) {
    return online === true
  }

  return Date.now() - last.getTime() < ONLINE_TTL_MS
}

/**
 * 设备是否有活跃会话（ChatManager 在线，含多端 endpoint）。
 */
export function isDeviceSessionOnline(device, liveStatus) {
  if (liveStatus?.online) return true
  return isDeviceOnline(device)
}

export function deviceOnlineLabel(device, liveStatus) {
  if (isDeviceSessionOnline(device, liveStatus)) return '在线'
  return '离线'
}

export function deviceOnlineTagType(device, liveStatus) {
  return isDeviceSessionOnline(device, liveStatus) ? 'success' : 'danger'
}
