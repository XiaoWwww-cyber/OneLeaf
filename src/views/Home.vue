<script setup lang="ts">
import { ref, reactive, nextTick, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import { ElMessage } from 'element-plus'
import {
  ChatDotRound, Document, VideoPlay, Position,
  Plus, Fold, Expand, Delete, Search
} from '@element-plus/icons-vue'
import VideoTranscriptDialog from '../components/VideoTranscriptDialog.vue'

// @ts-ignore
import { Thinking } from 'vue-element-plus-x'

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
  thinking?: string
  answer?: string
}
const messages = ref<Message[]>([])
const inputText = ref('')

const isLoading = ref(false)
const loadingMap = reactive<Record<string, boolean>>({})
const chatAreaRef = ref<HTMLElement>()
const isWebSearchEnabled = ref(false)

/** 更加鲁棒的解析函数 */
const parseMessage = (msg: {role: string, content: string, id?: string}): Message => {
    const base: Message = { 
        id: msg.id || Date.now().toString(), 
        role: msg.role as any, 
        content: msg.content 
    }
    if (msg.role !== 'assistant') {
        return { ...base, answer: msg.content };
    }
    
    // 支持不区分大小写、空格以及转义字符: <think> or &lt;think&gt;
    const thinkRegex = /(?:<\s*think\s*>|&lt;\s*think\s*&gt;)([\s\S]*?)(?:<\/\s*think\s*>|&lt;\/\s*think\s*&gt;)/i
    const match = msg.content.match(thinkRegex)
    
    if (match) {
        return {
            ...base,
            thinking: match[1].trim(),
            answer: msg.content.replace(thinkRegex, '').trim()
        }
    }
    return { ...base, answer: msg.content }
}

// ========== 知识库文档 ==========
interface KbDoc {
  id: string
  name: string
  category: string
  content: string
  source_path: string | null
  backup_path: string | null
  file_type: string
}
const kbDocuments = ref<KbDoc[]>([])
const videoDialogRef = ref()

// ========== 知识库搜索 ==========
interface SearchResult {
  document: KbDoc
  relevance: number
  snippet: string
}
const searchQuery = ref('')
const searchResults = ref<SearchResult[]>([])
const isSearching = ref(false)
const hasSearched = ref(false)

let searchDebounceTimer: ReturnType<typeof setTimeout> | null = null

const handleSearch = () => {
  if (searchDebounceTimer) clearTimeout(searchDebounceTimer)
  if (!searchQuery.value.trim()) {
    searchResults.value = []
    hasSearched.value = false
    return
  }
  searchDebounceTimer = setTimeout(async () => {
    isSearching.value = true
    hasSearched.value = true
    try {
      searchResults.value = await invoke<SearchResult[]>('search_knowledge_base', {
        query: searchQuery.value.trim(),
        limit: 10,
      })
    } catch (e) {
      console.warn('搜索失败:', e)
      searchResults.value = []
    } finally {
      isSearching.value = false
    }
  }, 300)
}

const clearSearch = () => {
  searchQuery.value = ''
  searchResults.value = []
  hasSearched.value = false
}

const detailDialogVisible = ref(false)
const selectedDoc = ref<KbDoc | null>(null)

const showDocDetail = (doc: KbDoc) => {
  selectedDoc.value = doc
  detailDialogVisible.value = true
}

/** 根据文件类型获取图标名 */
const getDocIcon = (doc: KbDoc) => {
  if (doc.category === 'video-transcript') return '🎬'
  const ft = doc.file_type?.toLowerCase() || ''
  if (['pdf'].includes(ft)) return '📄'
  if (['doc', 'docx'].includes(ft)) return '📝'
  if (['xls', 'xlsx'].includes(ft)) return '📊'
  if (['txt', 'md'].includes(ft)) return '📃'
  return '📁'
}

/** 根据文件类型获取标签名 */
const getDocTag = (doc: KbDoc) => {
  if (doc.category === 'video-transcript') return '视频转写'
  const ft = doc.file_type?.toLowerCase() || ''
  if (['pdf'].includes(ft)) return 'PDF 文档'
  if (['doc', 'docx'].includes(ft)) return 'Word 文档'
  if (['xls', 'xlsx'].includes(ft)) return 'Excel 表格'
  if (['txt'].includes(ft)) return '文本文件'
  if (['md'].includes(ft)) return 'Markdown'
  return '文档'
}

/** 是否可以用系统程序打开 */
const canOpenExternal = (doc: KbDoc | null) => {
  if (!doc) return false
  const ft = doc.file_type?.toLowerCase() || ''
  return ['pdf', 'doc', 'docx', 'xls', 'xlsx'].includes(ft)
}

/** 使用系统默认程序打开文件 */
const handleOpenFile = async (doc: KbDoc) => {
  try {
    await invoke('open_document_file', { id: doc.id })
  } catch (e) {
    ElMessage.error(`打开文件失败: ${e}`)
  }
}

// ========== 初始化 ==========
onMounted(async () => {
  // 1. 知识库初始化
  try {
    await invoke('init_knowledge_base', { dbPath: '' })
  } catch (e) {
    console.warn('KB init:', e)
  }

  // 2. 恢复本地对话记录
  try {
    const storedConvs = await invoke<any[]>('load_conversations')
    conversations.value = storedConvs.map(c => ({
      id: c.id,
      title: c.title,
      time: new Date(c.updated_at).toLocaleTimeString()
    }))

    if (conversations.value.length > 0) {
      const firstConv = conversations.value[0]
      await switchConversation(firstConv)
    }
  } catch (e) {
    console.warn('加载本地对话失败:', e)
  }

  await refreshDocuments()
})

// ========== 对话逻辑 ==========
const handleSend = async () => {
  const text = inputText.value.trim()
  if (!text || isLoading.value) return

  // 如果没有对话，创建一个
  if (!activeConversationId.value) {
    const convId = Date.now().toString()
    const convTitle = text.slice(0, 20) + (text.length > 20 ? '...' : '')
    const conv: Conversation = {
      id: convId,
      title: convTitle,
      time: new Date().toLocaleTimeString()
    }
    
    try {
      await invoke('save_conversation', { id: convId, title: convTitle })
    } catch(e) {
      console.error('保存新会话失败', e)
    }
    
    conversations.value.unshift(conv)
    activeConversationId.value = conv.id
    messages.value = []
  }

  // 1. 屏幕新增打字并落库
  const msgId = Date.now().toString()
  messages.value.push(parseMessage({
    id: msgId,
    role: 'user',
    content: text
  }))
  
  try {
    await invoke('save_message', { 
      id: msgId, 
      sessionId: activeConversationId.value, 
      role: 'user', 
      content: text 
    })
  } catch(e) {
    console.error('保存用户消息失败', e)
  }
  inputText.value = ''
  const currentSessionId = activeConversationId.value
  loadingMap[currentSessionId] = true
  await scrollToBottom()

  try {
    // 上下文管理（短期记忆）：过滤有效消息，并仅截取最近 10 条（5轮交互）作为上下文
    const validMessages = messages.value.filter(m => m.role === 'user' || m.role === 'assistant')
    const contextMessages = validMessages.slice(-10).map(m => ({
      role: m.role,
      content: m.content
    }))
    
    console.log('Sending messages to AI:', contextMessages)
    const reply = await invoke<string>('chat_with_ai', { 
        messages: contextMessages,
        sessionId: currentSessionId,
        enableWebSearch: isWebSearchEnabled.value
    })
    console.log('Received AI reply:', reply)
    const replyId = (Date.now() + 1).toString()
    
    // 只有当当前活跃的对话还是发送时的那个对话时，才推入内存列表
    // 否则，用户切换回来时会通过 switchConversation 重新从数据库加载新保存的消息
    if (activeConversationId.value === currentSessionId) {
      messages.value.push(parseMessage({
        id: replyId,
        role: 'assistant',
        content: reply
      }))
    }
    
    // AI 回复落库
    try {
      await invoke('save_message', { 
        id: replyId, 
        sessionId: currentSessionId, 
        role: 'assistant', 
        content: reply 
      })
      // 刷新一下标题的时间展示
      const c = conversations.value.find(c => c.id === currentSessionId)
      if (c) c.time = new Date().toLocaleTimeString()
    } catch(e) {
      console.error('保存AI消息失败', e)
    }
    
  } catch (e) {
    if (activeConversationId.value === currentSessionId) {
      messages.value.push(parseMessage({
        id: (Date.now() + 1).toString(),
        role: 'system',
        content: `错误: ${e}`
      }))
    }
  } finally {
    loadingMap[currentSessionId] = false
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

const switchConversation = async (conv: Conversation) => {
  activeConversationId.value = conv.id
  messages.value = []
  try {
    const rawMsgs = await invoke<any[]>('load_messages', { sessionId: conv.id })
    messages.value = rawMsgs.map(m => parseMessage({
      id: m.id,
      role: m.role,
      content: m.content
    }))
  } catch(e) {
    console.error('拉取历史消息失败', e)
  }
  scrollToBottom()
}

const deleteConversation = async (id: string) => {
  try {
    await invoke('delete_conversation_record', { sessionId: id })
    conversations.value = conversations.value.filter(c => c.id !== id)
    if (activeConversationId.value === id) {
      activeConversationId.value = ''
      messages.value = []
      if (conversations.value.length > 0) {
        switchConversation(conversations.value[0])
      }
    }
    ElMessage.success('会话已删除')
  } catch(e) {
    ElMessage.error(`删除失败: ${e}`)
  }
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
            <span class="conv-title" :title="conv.title">{{ conv.title }}</span>
            <el-icon class="conv-delete" @click.stop="deleteConversation(conv.id)" title="删除对话">
              <Delete />
            </el-icon>
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
              <div class="message-content">
                <template v-if="msg.role === 'assistant'">
                   <Thinking
                        v-if="msg.thinking"
                        status="end"
                        :model-value="false"
                        :content="msg.thinking"
                        button-width="180px"
                        max-width="100%"
                        background-color="#2a2a2a"
                        color="#d1d1d1"
                        style="margin-bottom: 8px;"
                    />
                    <div class="answer-text">{{ msg.answer || msg.content }}</div>
                </template>
                <template v-else>{{ msg.answer || msg.content }}</template>
              </div>
            </div>
          </div>

          <!-- Loading (基于 sessionId 隔离) -->
          <div v-if="loadingMap[activeConversationId]" class="message-row assistant">
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
            <el-tooltip :content="isWebSearchEnabled ? '关闭联网搜索' : '开启联网搜索'" placement="top">
              <el-icon class="tool-btn web-search-btn" :class="{ active: isWebSearchEnabled }" @click="isWebSearchEnabled = !isWebSearchEnabled">
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <circle cx="12" cy="12" r="10"></circle>
                  <line x1="2" y1="12" x2="22" y2="12"></line>
                  <path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"></path>
                </svg>
              </el-icon>
            </el-tooltip>
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
        <!-- 搜索框 -->
        <div class="kb-search">
          <el-input
            v-model="searchQuery"
            placeholder="搜索知识库..."
            :prefix-icon="Search"
            clearable
            size="small"
            @input="handleSearch"
            @clear="clearSearch"
          />
        </div>

        <!-- 搜索结果 -->
        <div v-if="hasSearched" class="kb-search-results">
          <div class="search-header">
            <span>搜索结果</span>
            <el-button link size="small" @click="clearSearch">返回列表</el-button>
          </div>
          <div v-if="isSearching" class="empty-hint">搜索中...</div>
          <div v-else-if="searchResults.length === 0" class="empty-hint">未找到匹配内容</div>
          <div
            v-for="result in searchResults"
            :key="result.document.id"
            class="search-result-item"
            @click="showDocDetail(result.document)"
          >
            <div class="sr-title">
              <span class="kb-emoji-icon">{{ getDocIcon(result.document) }}</span>
              <span class="sr-name">{{ result.document.name }}</span>
              <span class="sr-score">{{ (result.relevance * 100).toFixed(0) }}%</span>
            </div>
            <div class="sr-snippet">{{ result.snippet }}</div>
          </div>
        </div>

        <!-- 操作按钮和文档列表（非搜索状态） -->
        <template v-else>
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
              class="kb-item clickable"
              @click="showDocDetail(doc)"
            >
              <span class="kb-emoji-icon">{{ getDocIcon(doc) }}</span>
              <span class="kb-name" :title="doc.name">{{ doc.name }}</span>
              <el-icon class="kb-delete" @click.stop="handleDeleteDoc(doc.id)">
                <Delete />
              </el-icon>
            </div>
            <div v-if="kbDocuments.length === 0" class="empty-hint">
              知识库为空，请添加文档
            </div>
          </div>
        </template>
      </div>
    </aside>

    <VideoTranscriptDialog ref="videoDialogRef" @saved="refreshDocuments" />

    <!-- 文档详情对话框 -->
    <el-dialog v-model="detailDialogVisible" :title="selectedDoc?.name || '文档详情'" width="60%" class="doc-preview-dialog">
      <div v-if="selectedDoc" class="doc-detail-content">
        <div class="doc-meta-bar">
          <el-tag size="small" :type="selectedDoc.category === 'video-transcript' ? 'warning' : 'info'">
            {{ getDocTag(selectedDoc) }}
          </el-tag>
          <el-button
            v-if="canOpenExternal(selectedDoc)"
            size="small"
            type="primary"
            plain
            @click="handleOpenFile(selectedDoc)"
          >
            用系统程序打开原件
          </el-button>
        </div>
        <el-input
          :model-value="selectedDoc.content"
          type="textarea"
          :rows="18"
          readonly
          class="detail-textarea"
        />
      </div>
      <template #footer>
        <el-button @click="detailDialogVisible = false">关闭</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<style scoped>
.web-search-btn {
  color: #888;
}
.web-search-btn.active {
  color: #4facfe;
  filter: drop-shadow(0 0 8px rgba(79, 172, 254, 0.4));
}

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
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
}

.conv-delete {
  opacity: 0;
  transition: opacity 0.2s, color 0.2s;
  color: #888;
}

.conversation-item:hover .conv-delete {
  opacity: 1;
}

.conv-delete:hover {
  color: #ff6b6b;
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
  font-size: 1.6rem;
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
  font-size: 1.6rem;
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

.kb-search {
  margin-bottom: 12px;
}

.kb-search :deep(.el-input__wrapper) {
  background: rgba(255,255,255,0.04);
  border-color: rgba(255,255,255,0.08);
  border-radius: 10px;
}

.kb-search :deep(.el-input__inner) {
  color: #e3e3e3;
}

.kb-search :deep(.el-input__inner::placeholder) {
  color: #555;
}

.kb-search-results {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.search-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 8px;
  font-size: 0.8rem;
  color: #888;
}

.search-result-item {
  padding: 10px 10px;
  border-radius: 10px;
  cursor: pointer;
  transition: all 0.15s;
  border: 1px solid rgba(255,255,255,0.04);
}

.search-result-item:hover {
  background: rgba(79, 172, 254, 0.08);
  border-color: rgba(79, 172, 254, 0.2);
}

.sr-title {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-bottom: 4px;
}

.sr-name {
  flex: 1;
  font-size: 0.84rem;
  color: #ccc;
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.sr-score {
  font-size: 0.72rem;
  color: #4facfe;
  flex-shrink: 0;
}

.sr-snippet {
  font-size: 0.76rem;
  color: #777;
  line-height: 1.4;
  display: -webkit-box;
  -webkit-line-clamp: 3;
  -webkit-box-orient: vertical;
  overflow: hidden;
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

.kb-item.clickable {
  cursor: pointer;
}

.kb-item.clickable:hover {
  background: rgba(79, 172, 254, 0.1);
  color: #4facfe;
}

.kb-emoji-icon {
  font-size: 1rem;
  flex-shrink: 0;
  line-height: 1;
}

/* ========== 文档预览对话框 ========== */
.doc-meta-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 12px;
}

.detail-textarea :deep(.el-textarea__inner) {
  background-color: #1a1a2e;
  color: #e3e3e3;
  border-color: rgba(255, 255, 255, 0.1);
  font-family: 'Consolas', 'Microsoft YaHei', monospace;
  font-size: 0.9rem;
  line-height: 1.65;
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
/* ========== Thinking 组件暗色适配 ========== */
:deep(.el-thinking) {
  width: fit-content;
  margin: 0 !important; /* 强制左对齐，防止展开后居中 */
  margin-bottom: 12px !important;
}

:deep(.el-thinking .trigger) {
  background: rgba(255, 255, 255, 0.04) !important;
  border: 1px solid rgba(255, 255, 255, 0.08) !important;
  color: #9e9ea3 !important;
  height: 32px !important;
  padding: 0 12px !important;
  border-radius: 8px !important;
  transition: all 0.3s cubic-bezier(0.4, 0, 0.2, 1) !important;
}

:deep(.el-thinking .trigger:hover) {
  background: rgba(255, 255, 255, 0.08) !important;
  border-color: rgba(79, 172, 254, 0.3) !important;
  color: #e3e3e3 !important;
}

:deep(.el-thinking .trigger .label) {
  font-size: 13px !important;
  font-weight: 500 !important;
  color: inherit !important;
}

:deep(.el-thinking .trigger .status-icon) {
  font-size: 14px !important;
}

:deep(.el-thinking .trigger .arrow) {
  font-size: 12px !important;
  opacity: 0.6;
}

/* 展开后的内容区域样式 */
:deep(.el-thinking .content-wrapper) {
  margin-top: 8px;
}

:deep(.el-thinking .content pre) {
  background: #141414 !important;
  border: 1px solid #2a2a2a !important;
  color: #b1b1b1 !important;
  font-size: 13px !important;
  line-height: 1.6 !important;
  padding: 12px !important;
  border-radius: 8px !important;
  box-shadow: inset 0 2px 4px rgba(0,0,0,0.2) !important;
}

/* 消息内容调整 */
.answer-text {
  word-break: break-all;
  white-space: pre-wrap;
}
</style>
