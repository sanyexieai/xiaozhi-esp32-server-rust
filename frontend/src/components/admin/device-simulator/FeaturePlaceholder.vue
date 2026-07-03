<template>
  <div class="placeholder-panel">
    <div class="placeholder-icon">
      <el-icon :size="40"><component :is="icon" /></el-icon>
    </div>
    <h3>{{ title }}</h3>
    <p class="placeholder-desc">{{ description }}</p>
    <el-alert type="info" :closable="false" show-icon>
      <template #title>开发中</template>
      {{ hint }}
    </el-alert>
    <ul v-if="planned.length" class="planned-list">
      <li v-for="item in planned" :key="item">{{ item }}</li>
    </ul>
  </div>
</template>

<script setup>
defineProps({
  title: { type: String, required: true },
  description: { type: String, default: '' },
  hint: { type: String, default: '该模块已预留接口，后续版本实现。' },
  icon: { type: [String, Object], default: 'Picture' },
  planned: { type: Array, default: () => [] }
})
</script>

<style scoped>
.placeholder-panel {
  padding: 32px 24px;
  text-align: center;
  max-width: 560px;
  margin: 0 auto;
}

.placeholder-icon {
  color: var(--el-color-info);
  margin-bottom: 12px;
}

.placeholder-panel h3 {
  margin: 0 0 8px;
  font-size: 18px;
}

.placeholder-desc {
  color: var(--el-text-color-secondary);
  margin: 0 0 20px;
  line-height: 1.6;
}

.planned-list {
  text-align: left;
  margin: 20px auto 0;
  max-width: 420px;
  color: var(--el-text-color-regular);
  line-height: 1.8;
}
</style>
