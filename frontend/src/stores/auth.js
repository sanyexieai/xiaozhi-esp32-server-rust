import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import api from '../utils/api'

const WORKSPACE_VIEW_KEY = 'admin_workspace_view'

export const useAuthStore = defineStore('auth', () => {
  const token = ref(localStorage.getItem('token'))
  const user = ref(JSON.parse(localStorage.getItem('user') || 'null'))
  const isValidating = ref(false) // 添加验证状态标记
  const workspaceView = ref(localStorage.getItem(WORKSPACE_VIEW_KEY) || 'admin')

  const isAuthenticated = computed(() => !!token.value)
  const isAdmin = computed(() => user.value?.role === 'admin')
  const showAdminConsole = computed(() => isAdmin.value && workspaceView.value === 'admin')
  const showUserWorkspace = computed(() => !isAdmin.value || workspaceView.value === 'user')

  const setWorkspaceView = (view) => {
    if (!isAdmin.value) return
    const next = view === 'user' ? 'user' : 'admin'
    workspaceView.value = next
    localStorage.setItem(WORKSPACE_VIEW_KEY, next)
  }

  const resetWorkspaceView = () => {
    workspaceView.value = 'admin'
    localStorage.removeItem(WORKSPACE_VIEW_KEY)
  }

  const login = async (credentials) => {
    try {
      const response = await api.post('/login', credentials)
      const { token: newToken, user: userData } = response.data
      
      token.value = newToken
      user.value = userData
      
      localStorage.setItem('token', newToken)
      localStorage.setItem('user', JSON.stringify(userData))
      
      return { success: true, user: userData }
    } catch (error) {
      return { 
        success: false, 
        message: error.response?.data?.error || '登录失败' 
      }
    }
  }

  const register = async (userData) => {
    try {
      await api.post('/register', userData)
      return { success: true }
    } catch (error) {
      return { 
        success: false, 
        message: error.response?.data?.error || '注册失败' 
      }
    }
  }

  const logout = () => {
    token.value = null
    user.value = null
    resetWorkspaceView()
    localStorage.removeItem('token')
    localStorage.removeItem('user')
  }

  const getProfile = async () => {
    // 如果正在验证中，避免重复调用
    if (isValidating.value) {
      return
    }
    
    isValidating.value = true
    try {
      const response = await api.get('/profile')
      const userData = response.data.user ?? response.data
      user.value = userData
      localStorage.setItem('user', JSON.stringify(userData))
    } catch (error) {
      logout()
      throw error // 重新抛出错误，让路由守卫能够处理
    } finally {
      isValidating.value = false
    }
  }

  return {
    token,
    user,
    isAuthenticated,
    isAdmin,
    workspaceView,
    showAdminConsole,
    showUserWorkspace,
    isValidating,
    login,
    register,
    logout,
    getProfile,
    setWorkspaceView,
    resetWorkspaceView
  }
})