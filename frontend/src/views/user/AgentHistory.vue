<template>
  <ChatSessionsView
    scope="user"
    :agent-id="agentId"
    show-back
  />
</template>

<script setup>
import { computed } from 'vue'
import { useRoute } from 'vue-router'
import ChatSessionsView from '@/components/chat/ChatSessionsView.vue'

const route = useRoute()
const agentId = computed(() => route.params.id || null)
</script>
