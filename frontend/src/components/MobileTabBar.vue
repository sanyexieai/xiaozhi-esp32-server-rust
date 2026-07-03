<template>
  <van-tabbar
    v-model="activeTab"
    @change="handleTabChange"
    fixed
    placeholder
    safe-area-inset-bottom
    class="mobile-tabbar"
  >
    <van-tabbar-item
      v-for="tab in tabs"
      :key="tab.name"
      :icon="tab.icon"
      :name="tab.name"
    >
      {{ tab.label }}
    </van-tabbar-item>
  </van-tabbar>
</template>

<script setup>
import { ref, computed, watch, onMounted } from 'vue'
import { useRouter, useRoute } from 'vue-router'
import { useAuthStore } from '../stores/auth'

const router = useRouter()
const route = useRoute()
const authStore = useAuthStore()

const activeTab = ref('')

// 根据当前工作台视图定义标签栏
const tabs = computed(() => {
  if (authStore.showAdminConsole) {
    // 管理员标签栏
    return [
      { name: 'dashboard', label: '首页', icon: 'home-o', path: '/dashboard' },
      { name: 'config', label: '配置', icon: 'setting-o', path: '/admin/vad-config' },
      { name: 'manage', label: '管理', icon: 'apps-o', path: '/admin/users' },
      { name: 'more', label: '更多', icon: 'ellipsis', path: '/more' }
    ]
  }

  // 普通用户 / 管理员用户工作台
  return [
    { name: 'agents', label: '智能体', icon: 'apps-o', path: '/agents' },
    { name: 'speakers', label: '声纹', icon: 'user-o', path: '/user/speakers' },
    { name: 'more', label: '更多', icon: 'ellipsis', path: '/more' }
  ]
})

// 根据当前路由设置活动标签
const updateActiveTab = () => {
  const currentPath = route.path
  const currentTab = tabs.value.find(tab => {
    if (tab.path === currentPath) {
      return true
    }
    // 支持路径前缀匹配
    if (currentPath.startsWith(tab.path)) {
      return true
    }
    return false
  })
  
  if (currentTab) {
    activeTab.value = currentTab.name
  }
}

// 标签切换处理
const handleTabChange = (name) => {
  const tab = tabs.value.find(item => item.name === name)
  if (tab && tab.path !== route.path) {
    router.push(tab.path)
  }
}

// 监听路由变化
watch(
  () => route.path,
  () => {
    updateActiveTab()
  },
  { immediate: true }
)

onMounted(() => {
  updateActiveTab()
})
</script>

<style scoped>
.mobile-tabbar {
  background: transparent;
}

:deep(.van-tabbar) {
  z-index: 1200;
  left: 12px;
  right: 12px;
  bottom: 12px;
  width: auto;
  border-radius: 22px;
  border: 1px solid rgba(255, 255, 255, 0.84);
  box-shadow: var(--apple-shadow-lg);
  overflow: hidden;
}

:deep(.van-tabbar-item--active) {
  color: var(--apple-primary);
}

:deep(.van-tabbar-item) {
  min-height: 58px;
}
</style>
