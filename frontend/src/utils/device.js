/**
 * 设备检测工具
 * 用于判断当前访问设备类型，实现响应式布局
 */

/**
 * 判断是否为移动设备
 * @returns {boolean}
 */
export const isMobile = () => {
  // 通过User-Agent判断
  const userAgent = navigator.userAgent || navigator.vendor || window.opera
  const mobileRegex = /Android|webOS|iPhone|iPad|iPod|BlackBerry|IEMobile|Opera Mini/i
  const isMobileUA = mobileRegex.test(userAgent)
  
  // 通过屏幕宽度判断（备用方案）
  const isMobileWidth = window.innerWidth < 768
  
  return isMobileUA || isMobileWidth
}

/**
 * 判断是否为平板设备
 * @returns {boolean}
 */
export const isTablet = () => {
  const userAgent = navigator.userAgent || navigator.vendor || window.opera
  return /iPad|Android/i.test(userAgent) && window.innerWidth >= 768 && window.innerWidth < 1024
}

/**
 * 判断是否为桌面设备
 * @returns {boolean}
 */
export const isDesktop = () => {
  return !isMobile() && !isTablet()
}

/**
 * 判断是否为微信浏览器
 * @returns {boolean}
 */
export const isWeChat = () => {
  const userAgent = navigator.userAgent || ''
  return /MicroMessenger/i.test(userAgent)
}

/**
 * 获取设备类型
 * @returns {'mobile' | 'tablet' | 'desktop'}
 */
export const getDeviceType = () => {
  if (isMobile()) {
    return 'mobile'
  } else if (isTablet()) {
    return 'tablet'
  } else {
    return 'desktop'
  }
}

/**
 * 监听窗口大小变化
 * @param {Function} callback 回调函数
 * @returns {Function} 取消监听的函数
 */
export const onResize = (callback) => {
  let ticking = false
  
  const handler = () => {
    if (!ticking) {
      window.requestAnimationFrame(() => {
        callback(getDeviceType())
        ticking = false
      })
      ticking = true
    }
  }
  
  window.addEventListener('resize', handler)
  
  // 返回取消监听的函数
  return () => {
    window.removeEventListener('resize', handler)
  }
}
