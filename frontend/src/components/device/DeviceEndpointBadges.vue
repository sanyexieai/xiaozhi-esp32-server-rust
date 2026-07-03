<template>
  <div class="device-endpoint-badges">
    <template v-if="live?.online">
      <el-tag v-if="live.has_hardware" size="small" type="success" effect="plain">硬件</el-tag>
      <el-tag v-if="live.has_web" size="small" type="primary" effect="plain">Web</el-tag>
      <el-tag
        v-if="Number(live.endpoint_count) > 1"
        size="small"
        type="info"
        effect="plain"
      >
        {{ live.endpoint_count }} 端点
      </el-tag>
      <el-tag
        v-else-if="!live.has_hardware && !live.has_web && live.endpoint_count"
        size="small"
        type="info"
        effect="plain"
      >
        {{ live.endpoint_count }} 端点
      </el-tag>
    </template>
    <span v-else-if="showOffline" class="endpoint-empty">{{ offlineText }}</span>
  </div>
</template>

<script setup>
defineProps({
  live: {
    type: Object,
    default: null
  },
  showOffline: {
    type: Boolean,
    default: true
  },
  offlineText: {
    type: String,
    default: '—'
  }
})
</script>

<style scoped>
.device-endpoint-badges {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  align-items: center;
}

.endpoint-empty {
  color: var(--el-text-color-placeholder);
  font-size: 12px;
}
</style>
