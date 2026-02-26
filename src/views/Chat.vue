<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useRoute } from 'vue-router'
import { invoke } from '@tauri-apps/api/core'
// @ts-ignore
import { Thinking } from 'vue-element-plus-x'

interface ChatMsg {
    role: string;
    content: string;
    thinking?: string;
    answer?: string;
}

const route = useRoute()
const messages = ref<ChatMsg[]>([])
const input = ref('')
const loading = ref(false)

onMounted(() => {
    if (route.query.q) {
        input.value = route.query.q as string
        handleSend()
    }
})

/** 更加鲁棒的解析函数 */
const parseMessage = (msg: {role: string, content: string}): ChatMsg => {
    if (msg.role !== 'assistant') {
        return { ...msg, answer: msg.content };
    }
    
    // 支持不区分大小写、空格以及转义字符: <think> or &lt;think&gt;
    const thinkRegex = /(?:<\s*think\s*>|&lt;\s*think\s*&gt;)([\s\S]*?)(?:<\/\s*think\s*>|&lt;\/\s*think\s*&gt;)/i
    const match = msg.content.match(thinkRegex)
    
    if (match) {
        return {
            ...msg,
            thinking: match[1].trim(),
            answer: msg.content.replace(thinkRegex, '').trim()
        }
    }
    return { ...msg, answer: msg.content }
}

const handleSend = async () => {
    if (!input.value.trim()) return
    
    console.log('--- 发送开始 ---');
    const userMsg = input.value
    messages.value.push({ role: 'user', content: userMsg, answer: userMsg })
    input.value = ''
    loading.value = true
    
    try {
        const history = messages.value.map(m => ({ 
            role: m.role, 
            content: m.content
        }))
        
        const response = await invoke<string>('chat_with_ai', { 
            messages: history
        })
        
        // 解析并将结果存入
        const parsed = parseMessage({ role: 'assistant', content: response })
        messages.value.push(parsed)
        
    } catch (e) {
        messages.value.push({ role: 'system', content: `Error: ${e}`, answer: `Error: ${e}` })
    } finally {
        loading.value = false
    }
}
</script>

<template>
  <div class="chat-layout">
    <div class="sidebar">
       <div style="padding: 10px; font-size: 10px; color: #666;">
            DEBUG: {{ messages.length }} msgs
       </div>
    </div>
    
    <div class="chat-area">
        <div class="messages" ref="msgContainer">
            <div v-for="(msg, idx) in messages" :key="idx" :class="['message', msg.role]">
                <div class="bubble">
                    <template v-if="msg.role === 'assistant'">
                        <!-- 如果解析到了思考内容 -->
                        <Thinking
                            v-if="msg.thinking"
                            status="end"
                            :model-value="false"
                            :content="msg.thinking"
                            button-width="200px"
                            max-width="100%"
                            background-color="#2a2a2a"
                            color="#d1d1d1"
                            style="margin-bottom: 8px;"
                        />
                        
                        <!-- 最终答案 -->
                        <div class="answer-content">{{ msg.answer }}</div>
                        
                        <!-- 备用调试：如果看到这段文字，说明正则没配上 -->
                        <div v-if="!msg.thinking && msg.content.includes('<think>')" 
                             style="color: red; font-size: 10px; margin-top: 10px;">
                            正则未匹配，原始内容：{{ msg.content.substring(0, 50) }}...
                        </div>
                    </template>
                    <template v-else>{{ msg.answer }}</template>
                </div>
            </div>
             <div v-if="loading" class="message assistant">
                <div class="bubble">
                    <Thinking
                        status="thinking"
                        content=""
                        button-width="200px"
                        max-width="100%"
                        background-color="#2a2a2a"
                    />
                </div>
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

.answer-content {
    white-space: pre-wrap;
}

.input-bar {
    padding-top: 20px;
}
</style>

