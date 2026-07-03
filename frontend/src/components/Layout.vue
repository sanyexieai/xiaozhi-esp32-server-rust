<template>
  <div v-if="!isMobileDevice" class="layout-shell" :class="{ 'is-form-heavy-route': isFormHeavyRoute }">
    <aside class="sidebar-shell">
      <div class="sidebar-card apple-surface">
        <div class="brand-panel">
          <img class="brand-mark" :src="appLogo" alt="小智管理系统" />
          <div class="brand-copy">
            <p class="brand-eyebrow">Control Center</p>
            <h3>小智管理系统</h3>
            <p>{{ authStore.showAdminConsole ? 'AI 服务与设备管理台' : '设备与智能体工作台' }}</p>
          </div>
        </div>

        <div class="sidebar-meta">
          <span class="apple-chip is-primary">{{ workspaceBadge }}</span>
          <span class="apple-chip is-success">在线中</span>
          <button
            v-if="authStore.isAdmin"
            type="button"
            class="workspace-switch-btn"
            @click="handleWorkspaceSwitch"
          >
            {{ switchShortLabel }}
          </button>
        </div>

        <el-scrollbar class="sidebar-scroll">
          <el-menu
            :default-active="$route.path"
            class="sidebar-menu"
            router
            unique-opened
            :collapse-transition="false"
          >
            <el-menu-item v-if="authStore.showAdminConsole" index="/dashboard">
              <el-icon><House /></el-icon>
              <span>仪表板</span>
            </el-menu-item>

            <el-menu-item v-if="authStore.showUserWorkspace" index="/agents">
              <el-icon><Connection /></el-icon>
              <span>智能体管理</span>
            </el-menu-item>

            <el-menu-item v-if="authStore.showUserWorkspace" index="/user/devices">
              <el-icon><Iphone /></el-icon>
              <span>设备列表</span>
            </el-menu-item>

            <el-menu-item v-if="authStore.showUserWorkspace" index="/user/history">
              <el-icon><ChatDotRound /></el-icon>
              <span>对话记录</span>
            </el-menu-item>

            <el-menu-item v-if="authStore.showUserWorkspace" index="/user/roles">
              <el-icon><User /></el-icon>
              <span>我的角色</span>
            </el-menu-item>

            <el-menu-item v-if="authStore.showUserWorkspace" index="/speakers">
              <el-icon><Microphone /></el-icon>
              <span>声纹管理</span>
            </el-menu-item>

            <el-menu-item v-if="authStore.showUserWorkspace" index="/voice-clones">
              <el-icon><Microphone /></el-icon>
              <span>声音复刻</span>
            </el-menu-item>

            <el-menu-item v-if="authStore.showUserWorkspace" index="/user/knowledge-bases">
              <el-icon><Document /></el-icon>
              <span>我的知识库</span>
            </el-menu-item>

            <el-sub-menu v-if="authStore.showAdminConsole" index="/admin/service-config">
              <template #title>
                <el-icon><Tools /></el-icon>
                <span>服务配置</span>
              </template>
              <el-menu-item index="/admin/ota-config">OTA 配置</el-menu-item>
              <el-menu-item index="/admin/mqtt-config">MQTT 配置</el-menu-item>
              <el-menu-item index="/admin/mqtt-server-config">MQTT Server 配置</el-menu-item>
              <el-menu-item index="/admin/udp-config">UDP 配置</el-menu-item>
              <el-sub-menu index="/admin/mcp-config-group" class="menu-sub-child">
                <template #title>MCP 配置</template>
                <el-menu-item class="menu-grandchild" index="/admin/mcp-config">配置</el-menu-item>
                <el-menu-item class="menu-grandchild" index="/admin/mcp-market">MCP 市场</el-menu-item>
              </el-sub-menu>
              <el-menu-item index="/admin/speaker-config">声纹识别配置</el-menu-item>
              <el-menu-item index="/admin/chat-settings">聊天设置</el-menu-item>
              <el-menu-item index="/admin/device-chat-simulator">设备对话模拟</el-menu-item>
            </el-sub-menu>

            <el-sub-menu v-if="authStore.showAdminConsole" index="/admin/ai-config">
              <template #title>
                <el-icon><Cpu /></el-icon>
                <span>AI 配置</span>
              </template>
              <el-menu-item index="/admin/vad-config">VAD 配置</el-menu-item>
              <el-menu-item index="/admin/asr-config">ASR 配置</el-menu-item>
              <el-menu-item index="/admin/llm-config">LLM 配置</el-menu-item>
              <el-menu-item index="/admin/tts-config">TTS 配置</el-menu-item>
              <el-menu-item index="/admin/vision-config">Vision 配置</el-menu-item>
              <el-menu-item index="/admin/memory-config">Memory 配置</el-menu-item>
              <el-menu-item index="/admin/knowledge-search-config">知识库检索配置</el-menu-item>
            </el-sub-menu>

            <el-menu-item v-if="authStore.showAdminConsole" index="/admin/pool-stats">
              <el-icon><DataAnalysis /></el-icon>
              <span>资源池统计</span>
            </el-menu-item>

            <el-menu-item v-if="authStore.showAdminConsole" index="/admin/global-roles">
              <el-icon><Setting /></el-icon>
              <span>全局角色</span>
            </el-menu-item>

            <el-menu-item v-if="authStore.showAdminConsole" index="/admin/users">
              <el-icon><UserFilled /></el-icon>
              <span>用户管理</span>
            </el-menu-item>

            <el-menu-item v-if="authStore.showAdminConsole" index="/admin/devices">
              <el-icon><Iphone /></el-icon>
              <span>设备管理</span>
            </el-menu-item>

            <el-menu-item v-if="authStore.showAdminConsole" index="/admin/agents">
              <el-icon><Connection /></el-icon>
              <span>智能体管理</span>
            </el-menu-item>

            <el-menu-item v-if="authStore.showAdminConsole" index="/admin/chat-history">
              <el-icon><ChatDotRound /></el-icon>
              <span>对话记录</span>
            </el-menu-item>
          </el-menu>
        </el-scrollbar>
      </div>
    </aside>

    <div class="content-shell">
      <AppHeader
        :title="currentPageTitle"
        :eyebrow="authStore.showAdminConsole ? 'Admin Console' : 'User Workspace'"
        :username="authStore.user?.username || ''"
        :role-label="headerRoleLabel"
        :initial="usernameInitial"
        :is-admin="authStore.isAdmin"
        :show-admin-shortcuts="authStore.showAdminConsole"
        :show-workspace-switch="authStore.isAdmin"
        :workspace-switch-label="switchLabel"
        @command="handleCommand"
        @workspace-switch="handleWorkspaceSwitch"
      />

      <main class="main-shell">
        <router-view />
      </main>
    </div>
  </div>

  <MobileLayout v-else />
</template>

<script setup>
import { computed } from 'vue'
import { useRouter, useRoute } from 'vue-router'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useAuthStore } from '../stores/auth'
import { useWorkspaceSwitch } from '../composables/useWorkspaceSwitch'
import { isMobile } from '../utils/device'
import AppHeader from './AppHeader.vue'
import MobileLayout from './MobileLayout.vue'
import appLogo from '@/assets/brand/app-logo.svg'
import {
  House,
  Setting,
  User,
  Tools,
  Cpu,
  UserFilled,
  Iphone,
  Connection,
  Microphone,
  DataAnalysis,
  Document,
  ChatDotRound
} from '@element-plus/icons-vue'

const router = useRouter()
const route = useRoute()
const authStore = useAuthStore()
const { switchLabel, switchShortLabel, workspaceBadge, handleWorkspaceSwitch } = useWorkspaceSwitch()

const isMobileDevice = computed(() => isMobile())

const currentPageTitle = computed(() => route.meta?.title || (authStore.showAdminConsole ? '仪表板' : '我的智能体'))
const headerRoleLabel = computed(() => {
  if (!authStore.isAdmin) return '普通用户'
  return authStore.workspaceView === 'admin' ? '管理员' : '管理员 · 用户视角'
})
const isFormHeavyRoute = computed(() => ['AgentEdit', 'UserAgentEdit'].includes(String(route.name || '')))

const usernameInitial = computed(() => {
  const username = authStore.user?.username || 'U'
  return username.slice(0, 1).toUpperCase()
})

const handleCommand = async (command) => {
  if (command === 'workspace-switch') {
    await handleWorkspaceSwitch()
    return
  }

  if (command === 'api-tokens') {
    router.push('/user/api-tokens')
    return
  }

  if (command === 'logout') {
    try {
      await ElMessageBox.confirm('确定要退出登录吗？', '提示', {
        confirmButtonText: '确定',
        cancelButtonText: '取消',
        type: 'warning'
      })

      authStore.logout()
      ElMessage.success('已退出登录')
      router.push('/login')
    } catch {
      // 用户取消
    }
  }
}
</script>

<style scoped>
.layout-shell {
  height: 100dvh;
  min-height: 0;
  padding: 20px;
  display: grid;
  grid-template-columns: 220px minmax(0, 1fr);
  gap: 16px;
  overflow: hidden;
}

.sidebar-shell {
  min-width: 0;
}

.sidebar-card {
  height: calc(100dvh - 40px);
  padding: 12px;
  border-radius: 26px;
  display: flex;
  flex-direction: column;
}

.brand-panel {
  display: flex;
  align-items: center;
  gap: 9px;
  margin-bottom: 14px;
}

.brand-mark {
  width: 38px;
  height: 38px;
  border-radius: 14px;
  display: block;
  object-fit: cover;
  background: rgba(255, 255, 255, 0.88);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.36), 0 12px 22px rgba(0, 122, 255, 0.24);
  flex: none;
}

.brand-copy h3 {
  margin: 0;
  font-size: 15px;
  line-height: 1.2;
}

.brand-copy {
  min-width: 0;
}

.brand-copy h3,
.brand-copy p {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.brand-copy p {
  margin: 3px 0 0;
  color: var(--apple-text-secondary);
  font-size: 11px;
  line-height: 1.35;
}

.brand-eyebrow {
  margin: 0 0 4px !important;
  color: var(--apple-primary) !important;
  font-size: 9px !important;
  font-weight: 700;
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

.sidebar-meta {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-bottom: 14px;
}

.workspace-switch-btn {
  border: 1px solid rgba(0, 122, 255, 0.18);
  background: rgba(0, 122, 255, 0.08);
  color: var(--apple-primary);
  border-radius: 999px;
  padding: 4px 10px;
  font-size: 11px;
  font-weight: 700;
  cursor: pointer;
  transition: background 0.2s ease, transform 0.2s ease;
}

.workspace-switch-btn:hover {
  background: rgba(0, 122, 255, 0.14);
  transform: translateY(-1px);
}

.sidebar-scroll {
  flex: 1;
  min-height: 0;
  margin: 0 -4px -4px;
  padding-right: 4px;
}

.sidebar-menu {
  background: transparent;
  border-right: 0;
  padding: 2px 0 12px;
}

.sidebar-menu :deep(.el-menu-item),
.sidebar-menu :deep(.el-sub-menu__title) {
  height: 40px;
  margin-bottom: 4px;
  border-radius: 14px;
  color: var(--apple-text-secondary);
  font-weight: 600;
  padding: 0 12px !important;
  font-size: 13px;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
}

.sidebar-menu :deep(.el-sub-menu__title) {
  padding-right: 28px !important;
}

.sidebar-menu :deep(.el-menu-item span),
.sidebar-menu :deep(.el-sub-menu__title span) {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.sidebar-menu :deep(.el-menu-item:hover),
.sidebar-menu :deep(.el-sub-menu__title:hover) {
  color: var(--apple-text);
  background: rgba(255, 255, 255, 0.82);
}

.sidebar-menu :deep(.el-menu-item.is-active) {
  color: var(--apple-primary);
  background: rgba(0, 122, 255, 0.1);
  box-shadow: inset 0 0 0 1px rgba(0, 122, 255, 0.08);
}

.sidebar-menu :deep(.el-sub-menu .el-menu-item) {
  height: 36px;
  margin: 3px 0 3px 6px;
  border-radius: 12px;
  background: rgba(255, 255, 255, 0.48);
  padding-left: 46px !important;
}

.sidebar-menu :deep(.menu-sub-child > .el-sub-menu__title) {
  height: 36px;
  margin: 3px 0 3px 6px;
  border-radius: 12px;
  background: rgba(255, 255, 255, 0.48);
  padding-left: 46px !important;
  padding-right: 28px !important;
}

.sidebar-menu :deep(.menu-sub-child .menu-grandchild) {
  margin-left: 24px;
  padding-left: 46px !important;
  background: rgba(255, 255, 255, 0.34);
}

.sidebar-menu :deep(.el-menu-item .el-icon),
.sidebar-menu :deep(.el-sub-menu__title .el-icon) {
  margin-right: 8px;
  font-size: 15px;
  flex: none;
}

.sidebar-menu :deep(.el-sub-menu__icon-arrow) {
  right: 8px;
}

.content-shell {
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 16px;
  overflow: hidden;
}

.main-shell {
  flex: 1;
  min-width: 0;
  min-height: 0;
  overflow-y: auto;
  overscroll-behavior: contain;
  padding: 0 4px 4px 0;
  scrollbar-gutter: stable;
  -webkit-overflow-scrolling: touch;
}

.layout-shell.is-form-heavy-route :deep(.apple-surface) {
  backdrop-filter: none;
  -webkit-backdrop-filter: none;
  box-shadow: var(--apple-shadow-sm);
}

@media (max-width: 1360px) {
  .layout-shell {
    grid-template-columns: 208px minmax(0, 1fr);
  }
}
</style>
