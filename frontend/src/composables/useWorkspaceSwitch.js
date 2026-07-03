import { computed } from 'vue'
import { useRouter } from 'vue-router'
import { ElMessage } from 'element-plus'
import { useAuthStore } from '../stores/auth'

export function useWorkspaceSwitch() {
  const authStore = useAuthStore()
  const router = useRouter()

  const switchLabel = computed(() => (
    authStore.workspaceView === 'admin' ? '切换到用户工作台' : '切换到管理控制台'
  ))

  const switchShortLabel = computed(() => (
    authStore.workspaceView === 'admin' ? '用户工作台' : '管理控制台'
  ))

  const workspaceBadge = computed(() => {
    if (!authStore.isAdmin) return '用户模式'
    return authStore.workspaceView === 'admin' ? '管理员模式' : '用户工作台'
  })

  const handleWorkspaceSwitch = async () => {
    if (!authStore.isAdmin) return
    const next = authStore.workspaceView === 'admin' ? 'user' : 'admin'
    authStore.setWorkspaceView(next)
    await router.push(next === 'admin' ? '/dashboard' : '/agents')
    ElMessage.success(next === 'admin' ? '已切换到管理控制台' : '已切换到用户工作台')
  }

  return {
    switchLabel,
    switchShortLabel,
    workspaceBadge,
    handleWorkspaceSwitch
  }
}
