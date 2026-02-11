<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useRoute } from 'vue-router'
import { invoke } from '@tauri-apps/api/core'

const route = useRoute()
const messages = ref<{role: string, content: string}[]>([])
const input = ref('')
const loading = ref(false)

onMounted(() => {
    if (route.query.q) {
        input.value = route.query.q as string
        handleSend()
    }
})

const handleSend = async () => {
    if (!input.value.trim()) return
    
    const userMsg = input.value
    messages.value.push({ role: 'user', content: userMsg })
    input.value = ''
    loading.value = true
    
    try {
        const response = await invoke<string>('chat_with_ai', { 
            messages: messages.value 
        })
        messages.value.push({ role: 'assistant', content: response })
    } catch (e) {
        messages.value.push({ role: 'system', content: `Error: ${e}` })
    } finally {
        loading.value = false
    }
}
</script>

<template>
  <div class="chat-layout">
    <div class="sidebar">
       <!-- History list (Placeholder) -->
       <div class="history-item">Previous Chat 1</div>
       <div class="history-item">Previous Chat 2</div>
    </div>
    
    <div class="chat-area">
        <div class="messages">
            <div v-for="(msg, idx) in messages" :key="idx" :class="['message', msg.role]">
                <div class="bubble">{{ msg.content }}</div>
            </div>
             <div v-if="loading" class="message assistant">
                <div class="bubble">Thinking...</div>
            </div>
        </div>
        
        <div class="input-bar">
             <el-input v-model="input" @keyup.enter="handleSend" placeholder="Reply..." />
        </div>
    </div>
  </div>
</template>

<style scoped>
.chat-layout {
    display: flex;
    height: 100vh;
    background-color: #1a1a1a;
    color: white;
}

.sidebar {
    width: 260px;
    background-color: #131314;
    padding: 20px;
    border-right: 1px solid #333;
}

.chat-area {
    flex: 1;
    display: flex;
    flex-direction: column;
    padding: 20px;
}

.messages {
    flex: 1;
    overflow-y: auto;
    padding-bottom: 20px;
}

.message {
    margin-bottom: 15px;
    display: flex;
}

.message.user {
    justify-content: flex-end;
}

.bubble {
    padding: 10px 15px;
    border-radius: 12px;
    max-width: 70%;
    line-height: 1.5;
}

.user .bubble {
    background-color: #2b5cff;
    color: white;
}

.assistant .bubble {
    background-color: #2c2c2c;
    color: #e3e3e3;
}

.system .bubble {
    background-color: #333;
    color: #ff5c5c;
    font-size: 0.9em;
}

.input-bar {
    padding-top: 20px;
}
</style>
