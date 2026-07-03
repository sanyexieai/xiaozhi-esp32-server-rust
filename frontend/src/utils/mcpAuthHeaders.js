/** MCP 导入服务鉴权：Token 与 Headers(JSON) 互相同步 */

export function normalizeRawToken(input) {
  const value = String(input || '').trim()
  if (!value) return ''
  return value.replace(/^bearer\s+/i, '').trim()
}

export function formatBearerHeader(token) {
  const raw = normalizeRawToken(token)
  return raw ? `Bearer ${raw}` : ''
}

export function extractTokenFromHeaders(headers) {
  if (!headers || typeof headers !== 'object' || Array.isArray(headers)) return ''
  for (const [key, value] of Object.entries(headers)) {
    if (key.toLowerCase() === 'authorization') {
      return normalizeRawToken(value)
    }
  }
  return ''
}

export function applyTokenToHeaders(headers, token) {
  const base = { ...(headers && typeof headers === 'object' && !Array.isArray(headers) ? headers : {}) }
  for (const key of Object.keys(base)) {
    if (key.toLowerCase() === 'authorization') {
      delete base[key]
    }
  }
  const bearer = formatBearerHeader(token)
  if (bearer) {
    base.Authorization = bearer
  }
  return Object.keys(base).length > 0 ? base : null
}

export function headersToText(headers) {
  if (!headers || typeof headers !== 'object' || Array.isArray(headers)) return ''
  if (Object.keys(headers).length === 0) return ''
  return JSON.stringify(headers, null, 2)
}

export function parseHeadersText(text) {
  const txt = String(text || '').trim()
  if (!txt) return null
  const parsed = JSON.parse(txt)
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
    throw new Error('headers 必须是 JSON 对象')
  }
  return parsed
}

export function buildHeadersFromAuth(token, headersText) {
  const parsed = parseHeadersText(headersText)
  return applyTokenToHeaders(parsed, token)
}

export function loadAuthFieldsFromHeaders(headers) {
  return {
    token: extractTokenFromHeaders(headers),
    headersText: headersToText(headers)
  }
}

export function syncHeadersTextFromToken(token, headersText) {
  let base = null
  try {
    base = parseHeadersText(headersText)
  } catch {
    base = null
  }
  return headersToText(applyTokenToHeaders(base, token))
}
