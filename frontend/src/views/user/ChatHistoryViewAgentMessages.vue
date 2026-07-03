<template>
  <ChatHistoryView
    scope="user"
    :agent-id="agentId"
    show-back
  />
</template>

<script setup>
import { computed } from 'vue'
import { useRoute } from 'vue-router'
import ChatHistoryView from '@/components/chat/ChatHistoryView.vue'

const route = useRoute()
const agentId = computed(() => route.params.id || null)
</script>
