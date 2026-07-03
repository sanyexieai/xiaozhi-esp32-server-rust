const OPENCLAW_CHANNEL_NAME = 'xiaozhi'
const OPENCLAW_CHANNEL_CONFIG_PREFIX = `channels.${OPENCLAW_CHANNEL_NAME}`

const EMPTY_COMMAND_DATA = {
  ready: false,
  url: '',
  token: '',
  steps: [],
  commands: [],
  copyText: ''
}

export function buildOpenClawCommands(endpoint) {
  const trimmedEndpoint = String(endpoint || '').trim()
  if (!trimmedEndpoint) {
    return EMPTY_COMMAND_DATA
  }

  try {
    const parsed = new URL(trimmedEndpoint)
    const token = String(parsed.searchParams.get('token') || '').trim()
    parsed.search = ''
    parsed.hash = ''

    const url = parsed.toString()
    if (!url || !token) {
      return EMPTY_COMMAND_DATA
    }

    const steps = [
      {
        title: '启用渠道',
        command: `openclaw config set ${OPENCLAW_CHANNEL_CONFIG_PREFIX}.enabled true --strict-json`
      },
      {
        title: '配置地址',
        command: `openclaw config set ${OPENCLAW_CHANNEL_CONFIG_PREFIX}.url "${url}"`
      },
      {
        title: '配置令牌',
        command: `openclaw config set ${OPENCLAW_CHANNEL_CONFIG_PREFIX}.token "${token}"`
      },
      {
        title: '重启网关',
        command: 'openclaw gateway restart'
      }
    ]
    const commands = steps.map((step) => step.command)

    return {
      ready: true,
      url,
      token,
      steps,
      commands,
      copyText: commands.join('\n')
    }
  } catch (error) {
    console.error('解析 OpenClaw endpoint 失败:', error)
    return EMPTY_COMMAND_DATA
  }
}
