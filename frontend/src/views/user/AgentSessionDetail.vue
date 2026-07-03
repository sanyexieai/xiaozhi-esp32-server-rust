<template>
  <ChatSessionDetailView
    scope="user"
    :agent-id="agentId"
  />
</template>

<script setup>
import { computed } from 'vue'
import { useRoute } from 'vue-router'
import ChatSessionDetailView from '@/components/chat/ChatSessionDetailView.vue'

const route = useRoute()
const agentId = computed(() => route.params.id || null)
</script>
