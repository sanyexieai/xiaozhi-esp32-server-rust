export const BUILTIN_MCP_SERVER = '内置服务'
export const DEVICE_MCP_SERVER = '设备 MCP'
export const UNKNOWN_MCP_SERVER = '其他服务'

function groupToolsByServerName(tools = []) {
  const order = []
  const map = new Map()
  tools.forEach((tool) => {
    const serverName = String(tool?.server_name || UNKNOWN_MCP_SERVER).trim() || UNKNOWN_MCP_SERVER
    if (!map.has(serverName)) {
      order.push(serverName)
      map.set(serverName, [])
    }
    map.get(serverName).push(tool)
  })
  return order.map((serverName) => ({
    server_name: serverName,
    tools: map.get(serverName) || [],
  }))
}

export function normalizeMcpToolsResponse(data = {}) {
  const tools = Array.isArray(data.tools) ? data.tools : []
  const toolGroups = Array.isArray(data.tool_groups) && data.tool_groups.length > 0
    ? data.tool_groups
    : groupToolsByServerName(tools)

  return { tools, toolGroups }
}

export function buildMcpCascaderOptions(toolGroups = []) {
  return toolGroups
    .filter((group) => Array.isArray(group.tools) && group.tools.length > 0)
    .map((group) => ({
      value: group.server_name,
      label: `${group.server_name}（${group.tools.length}）`,
      children: group.tools.map((tool) => ({
        value: tool.name,
        label: tool.name,
      })),
    }))
}

export function findToolServerName(toolGroups = [], toolName = '') {
  if (!toolName) return ''
  for (const group of toolGroups) {
    if ((group.tools || []).some((tool) => tool.name === toolName)) {
      return group.server_name
    }
  }
  return ''
}

export function buildMcpToolCascaderValue(toolGroups = [], toolName = '') {
  const serverName = findToolServerName(toolGroups, toolName)
  if (!serverName || !toolName) return []
  return [serverName, toolName]
}

export function resolveMcpToolNameFromCascader(value) {
  if (!Array.isArray(value) || value.length === 0) return ''
  return String(value[value.length - 1] || '').trim()
}

export function countMcpToolGroups(toolGroups = []) {
  const serverCount = toolGroups.filter((group) => (group.tools || []).length > 0).length
  const toolCount = toolGroups.reduce((sum, group) => sum + (group.tools || []).length, 0)
  return { serverCount, toolCount }
}
