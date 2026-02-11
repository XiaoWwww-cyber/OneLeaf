<script setup lang="ts">
import { ref, nextTick, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import { ElMessage } from 'element-plus'
import {
  ChatDotRound, Document, VideoPlay, Position,
  Plus, Fold, Expand, Delete, FolderOpened
} from '@element-plus/icons-vue'
import VideoTranscriptDialog from '../components/VideoTranscriptDialog.vue'

// ========== 侧边栏状态 ==========
const leftCollapsed = ref(false)
const rightCollapsed = ref(true) // 知识库默认收起

// ========== 对话历史 ==========
interface Conversation {
  id: string
  title: string
  time: string
}
const conversations = ref<Conversation[]>([])
const activeConversationId = ref('')

// ========== 消息 ==========
interface Message {
  id: string
  role: 'user' | 'assistant' | 'system'
  content: string
}
const messages = ref<Message[]>([])
const inputText = ref('')
const isLoading = ref(false)
const chatAreaRef = ref<HTMLElement>()

// ========== 知识库文档 ==========
interface KbDoc {
  id: string
  name: string
  category: string
}
const kbDocuments = ref<KbDoc[]>([])
const videoDialogRef = ref()

// ========== 初始化 ==========
onMounted(async () => {
  try {
    await invoke('init_knowledge_base', { dbPath: '' })
  } catch (e) {
    console.warn('KB init:', e)
  }
  await refreshDocuments()
})

// ========== 对话逻辑 ==========
const handleSend = async () => {
  const text = inputText.value.trim()
  if (!text || isLoading.value) return

  // 如果没有对话，创建一个
  if (!activeConversationId.value) {
    const conv: Conversation = {
      id: Date.now().toString(),
      title: text.slice(0, 20) + (text.length > 20 ? '...' : ''),
      time: new Date().toLocaleTimeString()
    }
    conversations.value.unshift(conv)
    activeConversationId.value = conv.id
  }

  // 添加用户消息
  messages.value.push({
    id: Date.now().toString(),
    role: 'user',
    content: text
  })
  inputText.value = ''
  isLoading.value = true
  await scrollToBottom()

  try {
    const chatMessages = messages.value.map(m => ({
      role: m.role,
      content: m.content
    }))
    const reply = await invoke<string>('chat_with_ai', { messages: chatMessages })
    messages.value.push({
      id: (Date.now() + 1).toString(),
      role: 'assistant',
      content: reply
    })
  } catch (e) {
    messages.value.push({
      id: (Date.now() + 1).toString(),
      role: 'assistant',
      content: `错误: ${e}`
    })
  } finally {
    isLoading.value = false
    await scrollToBottom()
  }
}

const scrollToBottom = async () => {
  await nextTick()
  if (chatAreaRef.value) {
    chatAreaRef.value.scrollTop = chatAreaRef.value.scrollHeight
  }
}

const startNewChat = () => {
  activeConversationId.value = ''
  messages.value = []
}

const switchConversation = (conv: Conversation) => {
  activeConversationId.value = conv.id
  // 简化：新对话无历史消息
  messages.value = []
}

// ========== 知识库操作 ==========
const refreshDocuments = async () => {
  try {
    const docs = await invoke<KbDoc[]>('list_documents')
    kbDocuments.value = docs
  } catch (e) {
    console.warn('加载文档列表失败:', e)
  }
}

const handleUploadFile = async () => {
  const selected = await open({
    multiple: false,
    filters: [{ name: '文档', extensions: ['txt', 'md', 'docx', 'pdf'] }]
  })
  if (!selected || Array.isArray(selected)) return
  const filePath = selected
  if (!filePath) return

  try {
    await invoke('add_document_to_kb', { filePath, category: 'documents' })
    const fileName = filePath.split(/[\\/]/).pop() || filePath
    ElMessage.success(`已添加 ${fileName}`)
    await refreshDocuments()
  } catch (err) {
    ElMessage.error(`添加失败: ${err}`)
  }
}

const handleUploadVideo = async () => {
  const selected = await open({
    multiple: false,
    filters: [{ name: '视频', extensions: ['mp4', 'avi', 'mkv', 'mov', 'flv', 'wmv'] }]
  })
  if (!selected || Array.isArray(selected)) return
  const filePath = selected
  if (!filePath) return
  videoDialogRef.value?.open(filePath)
}

const handleDeleteDoc = async (id: string) => {
  try {
    await invoke('delete_document', { id })
    ElMessage.success('已删除')
    await refreshDocuments()
  } catch (e) {
    ElMessage.error(`删除失败: ${e}`)
  }
}
</script>

<template>
  <div class="app-layout">
    <!-- ========== 左侧边栏：历史对话 ========== -->
    <aside class="sidebar sidebar-left" :class="{ collapsed: leftCollapsed }">
      <div class="sidebar-header">
        <div class="brand">
          <span class="brand-icon">🍃</span>
          <span class="brand-name">OneLeaf</span>
        </div>
      </div>

      <div class="sidebar-content">
        <div class="new-chat-btn" @click="startNewChat">
          <el-icon><Plus /></el-icon>
          <span>新对话</span>
        </div>

        <div class="conversation-list">
          <div
            v-for="conv in conversations"
            :key="conv.id"
            class="conversation-item"
            :class="{ active: conv.id === activeConversationId }"
            @click="switchConversation(conv)"
          >
            <el-icon><ChatDotRound /></el-icon>
            <span class="conv-title">{{ conv.title }}</span>
          </div>
          <div v-if="conversations.length === 0" class="empty-hint">
            暂无对话记录
          </div>
        </div>
      </div>
    </aside>

    <!-- ========== 中间主区域 ========== -->
    <main class="main-area">
      <!-- 左侧边栏展开/收起按钮 -->
      <div class="edge-toggle left-edge" @click="leftCollapsed = !leftCollapsed">
        <el-icon><Expand v-if="leftCollapsed" /><Fold v-else /></el-icon>
      </div>

      <!-- 消息区域 -->
      <div ref="chatAreaRef" class="chat-area">
        <!-- 空状态 -->
        <div v-if="messages.length === 0" class="welcome">
          <h1 class="gradient-text">有什么我能帮你的吗？</h1>
          <p class="welcome-sub">基于知识库的智能问答助手</p>
        </div>

        <!-- 消息列表 -->
        <div v-else class="message-list">
          <div
            v-for="msg in messages"
            :key="msg.id"
            class="message-row"
            :class="msg.role"
          >
            <div class="message-avatar">
              {{ msg.role === 'user' ? '👤' : '🍃' }}
            </div>
            <div class="message-bubble">
              <div class="message-content">{{ msg.content }}</div>
            </div>
          </div>

          <!-- Loading -->
          <div v-if="isLoading" class="message-row assistant">
            <div class="message-avatar">🍃</div>
            <div class="message-bubble">
              <div class="typing-indicator">
                <span></span><span></span><span></span>
              </div>
            </div>
          </div>
        </div>
      </div>

      <!-- 底部输入框 -->
      <div class="input-area">
        <div class="input-wrapper">
          <div class="input-tools">
            <el-tooltip content="上传文档" placement="top">
              <el-icon class="tool-btn" @click="handleUploadFile"><Document /></el-icon>
            </el-tooltip>
            <el-tooltip content="视频转写" placement="top">
              <el-icon class="tool-btn" @click="handleUploadVideo"><VideoPlay /></el-icon>
            </el-tooltip>
          </div>
          <input
            v-model="inputText"
            class="chat-input"
            placeholder="发消息或输入 / 选择技能"
            @keyup.enter="handleSend"
          />
          <el-icon
            class="send-btn"
            :class="{ active: inputText.trim() }"
            @click="handleSend"
          >
            <Position />
          </el-icon>
        </div>
      </div>

      <!-- 右侧边栏展开/收起按钮 -->
      <div class="edge-toggle right-edge" @click="rightCollapsed = !rightCollapsed">
        <el-icon><Fold v-if="rightCollapsed" /><Expand v-else /></el-icon>
      </div>
    </main>

    <!-- ========== 右侧边栏：知识库 ========== -->
    <aside class="sidebar sidebar-right" :class="{ collapsed: rightCollapsed }">
      <div class="sidebar-header">
        <span class="sidebar-title">知识库</span>
      </div>

      <div class="sidebar-content">
        <div class="kb-actions">
          <el-button size="small" @click="handleUploadFile">
            <el-icon><Document /></el-icon>添加文档
          </el-button>
          <el-button size="small" @click="handleUploadVideo">
            <el-icon><VideoPlay /></el-icon>视频转写
          </el-button>
        </div>

        <div class="kb-list">
          <div
            v-for="doc in kbDocuments"
            :key="doc.id"
            class="kb-item"
          >
            <el-icon class="kb-icon"><FolderOpened /></el-icon>
            <span class="kb-name" :title="doc.name">{{ doc.name }}</span>
            <el-icon class="kb-delete" @click.stop="handleDeleteDoc(doc.id)">
              <Delete />
            </el-icon>
          </div>
          <div v-if="kbDocuments.length === 0" class="empty-hint">
            知识库为空，请添加文档
          </div>
        </div>
      </div>
    </aside>

    <VideoTranscriptDialog ref="videoDialogRef" />
  </div>
</template>

<style scoped>
/* ========== 整体布局 ========== */
.app-layout {
  display: flex;
  height: 100vh;
  width: 100vw;
  overflow: hidden;
  background-color: #1a1a2e;
}

/* ========== 侧边栏通用 ========== */
.sidebar {
  display: flex;
  flex-direction: column;
  background-color: #16163a;
  border-right: 1px solid rgba(255,255,255,0.06);
  transition: width 0.25s ease;
  width: 260px;
  flex-shrink: 0;
  overflow: hidden;
}

.sidebar.collapsed {
  width: 0;
  border: none;
}

.sidebar-right {
  border-right: none;
  border-left: 1px solid rgba(255,255,255,0.06);
}

.sidebar-header {
  display: flex;
  align-items: center;
  padding: 16px 12px;
  gap: 10px;
  border-bottom: 1px solid rgba(255,255,255,0.06);
  min-height: 56px;
  white-space: nowrap;
}

.brand {
  display: flex;
  align-items: center;
  gap: 8px;
  flex: 1;
}

.brand-icon {
  font-size: 1.4rem;
}

.brand-name {
  font-size: 1.1rem;
  font-weight: 600;
  color: #e3e3e3;
  letter-spacing: 0.5px;
}

.sidebar-title {
  flex: 1;
  font-size: 1rem;
  font-weight: 600;
  color: #e3e3e3;
  white-space: nowrap;
}

.sidebar-content {
  flex: 1;
  overflow-y: auto;
  padding: 12px 8px;
}

/* ========== 边缘切换按钮 ========== */
.main-area {
  position: relative;
}

.edge-toggle {
  position: absolute;
  top: 50%;
  transform: translateY(-50%);
  width: 20px;
  height: 48px;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  color: #555;
  z-index: 10;
  border-radius: 6px;
  transition: all 0.2s;
  background: rgba(255,255,255,0.03);
}

.edge-toggle:hover {
  color: #e3e3e3;
  background: rgba(255,255,255,0.08);
}

.edge-toggle.left-edge {
  left: 2px;
}

.edge-toggle.right-edge {
  right: 2px;
}

/* ========== 左侧 - 新对话按钮 ========== */
.new-chat-btn {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 14px;
  margin-bottom: 12px;
  border-radius: 10px;
  cursor: pointer;
  color: #ccc;
  font-size: 0.9rem;
  border: 1px dashed rgba(255,255,255,0.15);
  transition: all 0.2s;
}

.new-chat-btn:hover {
  background: rgba(79, 172, 254, 0.1);
  border-color: rgba(79, 172, 254, 0.3);
  color: #4facfe;
}

/* ========== 左侧 - 对话列表 ========== */
.conversation-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.conversation-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 14px;
  border-radius: 10px;
  cursor: pointer;
  color: #aaa;
  font-size: 0.88rem;
  transition: all 0.15s;
  white-space: nowrap;
  overflow: hidden;
}

.conversation-item:hover {
  background: rgba(255,255,255,0.06);
  color: #e3e3e3;
}

.conversation-item.active {
  background: rgba(79, 172, 254, 0.12);
  color: #4facfe;
}

.conv-title {
  overflow: hidden;
  text-overflow: ellipsis;
}

.empty-hint {
  text-align: center;
  color: #555;
  font-size: 0.82rem;
  padding: 20px 8px;
}

/* ========== 中间主区域 ========== */
.main-area {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
  position: relative;
}

.chat-area {
  flex: 1;
  overflow-y: auto;
  padding: 32px 24px 16px;
  display: flex;
  flex-direction: column;
}

/* ========== 欢迎页 ========== */
.welcome {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 12px;
}

.gradient-text {
  font-size: 2.2rem;
  font-weight: 700;
  background: linear-gradient(135deg, #4facfe 0%, #00f2fe 50%, #a78bfa 100%);
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
  background-clip: text;
}

.welcome-sub {
  color: #666;
  font-size: 1rem;
}

/* ========== 消息列表 ========== */
.message-list {
  display: flex;
  flex-direction: column;
  gap: 20px;
  max-width: 780px;
  width: 100%;
  margin: 0 auto;
}

.message-row {
  display: flex;
  gap: 12px;
  align-items: flex-start;
  animation: fadeIn 0.3s ease;
}

.message-row.user {
  flex-direction: row-reverse;
}

.message-avatar {
  width: 36px;
  height: 36px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 1.1rem;
  background: rgba(255,255,255,0.06);
  flex-shrink: 0;
}

.message-bubble {
  max-width: 70%;
  padding: 12px 16px;
  border-radius: 16px;
  font-size: 0.92rem;
  line-height: 1.6;
  color: #e3e3e3;
}

.message-row.user .message-bubble {
  background: #2d4a7a;
  border-bottom-right-radius: 4px;
}

.message-row.assistant .message-bubble {
  background: rgba(255,255,255,0.06);
  border-bottom-left-radius: 4px;
}

/* ========== Typing 动画 ========== */
.typing-indicator {
  display: flex;
  gap: 4px;
  padding: 4px 0;
}

.typing-indicator span {
  width: 8px;
  height: 8px;
  background: #4facfe;
  border-radius: 50%;
  animation: bounce 1.4s infinite ease-in-out both;
}

.typing-indicator span:nth-child(1) { animation-delay: -0.32s; }
.typing-indicator span:nth-child(2) { animation-delay: -0.16s; }
.typing-indicator span:nth-child(3) { animation-delay: 0s; }

@keyframes bounce {
  0%, 80%, 100% { transform: scale(0.6); opacity: 0.4; }
  40% { transform: scale(1); opacity: 1; }
}

@keyframes fadeIn {
  from { opacity: 0; transform: translateY(8px); }
  to { opacity: 1; transform: translateY(0); }
}

/* ========== 底部输入框 ========== */
.input-area {
  padding: 12px 24px 20px;
  border-top: 1px solid rgba(255,255,255,0.04);
}

.input-wrapper {
  display: flex;
  align-items: center;
  gap: 12px;
  max-width: 780px;
  margin: 0 auto;
  background: #22223a;
  border: 1px solid rgba(255,255,255,0.1);
  border-radius: 20px;
  padding: 8px 16px;
  transition: border-color 0.2s;
}

.input-wrapper:focus-within {
  border-color: rgba(79, 172, 254, 0.4);
}

.input-tools {
  display: flex;
  gap: 6px;
}

.tool-btn {
  font-size: 1.15rem;
  color: #888;
  cursor: pointer;
  padding: 6px;
  border-radius: 8px;
  transition: all 0.2s;
}

.tool-btn:hover {
  color: #4facfe;
  background: rgba(79, 172, 254, 0.1);
}

.chat-input {
  flex: 1;
  background: transparent;
  border: none;
  outline: none;
  color: #e3e3e3;
  font-size: 0.95rem;
  padding: 6px 0;
  font-family: inherit;
}

.chat-input::placeholder {
  color: #555;
}

.send-btn {
  font-size: 1.2rem;
  color: #555;
  cursor: pointer;
  padding: 6px;
  border-radius: 8px;
  transition: all 0.2s;
}

.send-btn.active {
  color: #4facfe;
}

.send-btn:hover {
  background: rgba(79, 172, 254, 0.1);
}

/* ========== 右侧 - 知识库 ========== */
.kb-actions {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-bottom: 16px;
}

.kb-actions .el-button {
  justify-content: flex-start;
  background: transparent;
  border: 1px dashed rgba(255,255,255,0.12);
  color: #aaa;
}

.kb-actions .el-button:hover {
  color: #4facfe;
  border-color: rgba(79, 172, 254, 0.3);
}

.kb-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.kb-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
  border-radius: 8px;
  color: #aaa;
  font-size: 0.84rem;
  cursor: default;
  transition: all 0.15s;
}

.kb-item:hover {
  background: rgba(255,255,255,0.05);
}

.kb-icon {
  color: #4facfe;
  flex-shrink: 0;
}

.kb-name {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.kb-delete {
  color: #555;
  cursor: pointer;
  opacity: 0;
  transition: all 0.15s;
}

.kb-item:hover .kb-delete {
  opacity: 1;
}

.kb-delete:hover {
  color: #f56c6c;
}
</style>
