<script setup lang="ts">
import { ref, reactive, nextTick, onMounted, computed, triggerRef } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { open } from '@tauri-apps/plugin-dialog'
import { ElMessage } from 'element-plus'
import { marked } from 'marked'
import {
  ChatDotRound, Document, VideoPlay,
  Fold, Expand, Delete, Search, ArrowDown, EditPen,
  Paperclip, Lightning, Phone, CopyDocument, More, Top, Setting, Loading,
  Collection
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
  thinkingDone?: boolean
}
const messages = ref<Message[]>([])
const inputText = ref('')

const loadingMap = reactive<Record<string, boolean>>({})
const chatAreaRef = ref<HTMLElement>()
const isWebSearchEnabled = ref(false)
const currentAiProvider = ref<string>('')

const isDoubao = computed(() => currentAiProvider.value === 'doubao')

// 当前会话是否正在加载
const isCurrentLoading = computed(() => {
  return !!loadingMap[activeConversationId.value]
})

// 用于中断流式生成
let stopStreamFlag = false
const isTranscribing = ref(false)

const handleStopGenerate = () => {
  stopStreamFlag = true
  // 立即关闭 loading 状态
  if (activeConversationId.value) {
    loadingMap[activeConversationId.value] = false
  }
}

const thinkingDepth = ref('none')
const thinkingDepthLabel = computed(() => {
  const map: Record<string, string> = {
    none: '关闭',
    low: '低',
    medium: '中',
    high: '高'
  }
  return map[thinkingDepth.value]
})
const handleThinkingDepth = (command: string) => {
  thinkingDepth.value = command
}

const textareaRef = ref<HTMLTextAreaElement>()
const adjustTextareaHeight = () => {
  if (textareaRef.value) {
    textareaRef.value.style.height = 'auto'
    textareaRef.value.style.height = Math.min(textareaRef.value.scrollHeight, 180) + 'px'
  }
}

// ========== Markdown 渲染 ==========
marked.setOptions({
  breaks: true,
  gfm: true,
})
const renderMarkdown = (text: string): string => {
  if (!text) return ''
  try {
    return marked.parse(text) as string
  } catch {
    return text
  }
}

// ========== 聊天附件（直传 AI） ==========
interface ChatAttachment {
  fileId: string
  fileType: string // "file" | "image" | "video"
  fileName: string
}
const attachedFiles = ref<ChatAttachment[]>([])

const handleChatUploadFile = async () => {
  const selected = await open({
    multiple: false,
    filters: [{ name: '文档', extensions: ['pdf', 'txt', 'md', 'doc', 'docx'] }]
  })
  if (!selected || Array.isArray(selected)) return
  const filePath = selected
  if (!filePath) return

  const fileName = filePath.split(/[\\/]/).pop() || filePath

  try {
    ElMessage.info(`正在上传 ${fileName}...`)
    const fileId = await invoke<string>('upload_file_to_ai', { filePath })
    attachedFiles.value.push({
      fileId,
      fileType: 'file',
      fileName,
    })
    ElMessage.success(`${fileName} 已添加`)
  } catch (e) {
    ElMessage.error(`上传失败: ${e}`)
  }
}

const handleChatUploadVideo = async () => {
  const selected = await open({
    multiple: false,
    filters: [{ name: '视频', extensions: ['mp4', 'avi', 'mkv', 'mov', 'flv', 'wmv'] }]
  })
  if (!selected || Array.isArray(selected)) return
  const filePath = selected
  if (!filePath) return

  const fileName = filePath.split(/[\\/]/).pop() || filePath

  try {
    isTranscribing.value = true
    ElMessage.info(`正在分析视频并转写文本: ${fileName}...`)
    // 调用视频转写逻辑
    const res = await invoke<{ text: string }>('transcribe_video', { videoPath: filePath })
    if (!res.text) {
      throw new Error("转写结果为空")
    }

    // 将转写文本保存为临时文件，以便作为 LOCAL_FILE 处理
    const tempFileName = `${fileName}_transcript_${Date.now()}.txt`
    const fileId = await invoke<string>('save_text_as_temp_file', {
      content: res.text,
      fileName: tempFileName
    })

    attachedFiles.value.push({
      fileId,
      fileType: 'file', // 转写后作为文本文件发送
      fileName: `[视频转写] ${fileName}`,
    })
    ElMessage.success(`${fileName} 转写并添加成功`)
  } catch (e) {
    ElMessage.error(`分析失败: ${e}`)
  } finally {
    isTranscribing.value = false
  }
}

// ========== 知识库附件（直接挂载引用） ==========
const kbFileDialogVisible = ref(false)
const handleChatAttachKbFile = () => {
  kbFileDialogVisible.value = true
}

const selectKbDocForChat = (doc: KbDoc) => {
  // 防止重复添加
  if (attachedFiles.value.some(a => a.fileId === `KB_FILE:${doc.id}`)) {
    ElMessage.warning('该文档已经添加')
    return
  }
  attachedFiles.value.push({
    fileId: `KB_FILE:${doc.id}`,
    fileType: 'kb_document',
    fileName: `[KB] ${doc.name}`,
  })
  ElMessage.success(`${doc.name} 已选为参考上下文`)
  kbFileDialogVisible.value = false
}

const removeAttachment = (index: number) => {
  attachedFiles.value.splice(index, 1)
}

/** 更加鲁棒的解析函数 */
const parseMessage = (msg: { role: string, content: string, id?: string }): Message => {
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
      answer: msg.content.replace(thinkRegex, '').trim(),
      thinkingDone: true,
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

  // 3. 获取 AI 配置，判断是否显示高级选项
  try {
    const aiConfig = await invoke<any>('get_ai_settings')
    currentAiProvider.value = aiConfig?.provider || ''
  } catch (e) {
    console.warn('load ai settings:', e)
  }

  // 监听后端 provider 切换事件，实时同步
  listen<string>('ai-provider-changed', (event) => {
    console.log('AI provider changed:', event.payload)
    currentAiProvider.value = event.payload || ''
  })

  // 监听缓存清除事件，刷新前端状态
  listen('cache-cleared', () => {
    kbDocuments.value = []
    conversations.value = []
    messages.value = []
    activeConversationId.value = ''
    searchQuery.value = ''
    searchResults.value = []
    hasSearched.value = false
  })

  await refreshDocuments()
})

// ========== 对话逻辑 ==========
const handleSend = async () => {
  const text = inputText.value.trim()
  if (!text || isCurrentLoading.value) return

  stopStreamFlag = false

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
    } catch (e) {
      console.error('保存新会话失败', e)
    }

    conversations.value.unshift(conv)
    activeConversationId.value = conv.id
    messages.value = []
  }

  // 1. 屏幕新增用户消息并落库
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
  } catch (e) {
    console.error('保存用户消息失败', e)
  }

  // 收集附件并清空
  const currentAttachments = attachedFiles.value.map(a => ({
    file_id: a.fileId,
    file_type: a.fileType,
    file_name: a.fileName,
  }))
  attachedFiles.value = []
  inputText.value = ''

  const currentSessionId = activeConversationId.value
  loadingMap[currentSessionId] = true
  await scrollToBottom()

  // 流式响应的消息（在收到第一个 chunk 时再推入）
  const replyId = (Date.now() + 1).toString()
  const streamMsg: Message = {
    id: replyId,
    role: 'assistant',
    content: '',
    thinking: '',
    answer: '',
  }

  // 设置流式监听
  let streamFullText = ''
  let streamThinking = ''
  let streamStarted = false
  let msgProxy: Message = streamMsg // 初始指向原始对象，push 后重新指向 proxy
  const unlisten = await listen<any>('ai-stream-chunk', (event) => {
    const chunk = event.payload
    if (chunk.session_id !== currentSessionId) return

    if (chunk.done) {
      return
    }

    // 检查是否被用户中断
    if (stopStreamFlag) return

    // 第一个 chunk 到来时，关闭 loading 并推入消息
    if (!streamStarted) {
      streamStarted = true
      loadingMap[currentSessionId] = false
      if (activeConversationId.value === currentSessionId) {
        messages.value.push(streamMsg)
        // 取回 reactive proxy 引用，确保后续修改能触发 Vue re-render
        msgProxy = messages.value[messages.value.length - 1]
      }
    }

    if (chunk.chunk_type === 'text') {
      const delta = chunk.delta
      streamFullText += delta

      // 兼容本地模型：如果 delta 包含 </think>，通常意味着思考结束
      if (delta.includes('</think>') || streamFullText.includes('</think>')) {
        if (!msgProxy.thinkingDone) {
          msgProxy.thinkingDone = true
        }
      }

      msgProxy.answer = streamFullText
      msgProxy.content = (streamThinking ? `<think>${streamThinking}</think>\n\n` : '') + streamFullText

      // 兼容云端模型：收到第一个正文 chunk 时（如果非思考内容），折叠思考面板
      if (!msgProxy.thinkingDone && streamThinking && !delta.includes('<think>')) {
        msgProxy.thinkingDone = true
      }
    } else if (chunk.chunk_type === 'thinking') {
      streamThinking += chunk.delta
      msgProxy.thinking = streamThinking
      msgProxy.thinkingDone = false // 确保在思考中
    }

    triggerRef(messages)
    scrollToBottom()
  })

  try {
    // 上下文管理（短期记忆）
    const validMessages = messages.value
      .filter(m => m.role === 'user' || m.role === 'assistant')
      .filter(m => m.id !== replyId) // 排除刚创建的空消息
    const contextMessages = validMessages.slice(-10).map(m => ({
      role: m.role,
      content: m.content
    }))

    console.log('Sending messages to AI:', contextMessages)
    const reply = await invoke<string>('chat_with_ai', {
      messages: contextMessages,
      sessionId: currentSessionId,
      enableWebSearch: isWebSearchEnabled.value,
      thinkingDepth: thinkingDepth.value !== 'none' ? thinkingDepth.value : null,
      attachments: currentAttachments.length > 0 ? currentAttachments : null,
    })

    // 对于非流式 Provider，reply 是完整文本
    // 对于豆包(流式)，reply 也是完整文本（流式 callback 已实时填充）
    // 确保最终 streamMsg.content 有值
    if (!streamStarted && reply) {
      // 非流式情况下，用完整 reply 填充并推入列表
      const parsed = parseMessage({ id: replyId, role: 'assistant', content: reply })
      streamMsg.content = parsed.content
      streamMsg.thinking = parsed.thinking
      streamMsg.answer = parsed.answer
      streamMsg.thinkingDone = parsed.thinkingDone // 核心修复：同步思考状态
      if (activeConversationId.value === currentSessionId) {
        messages.value.push(streamMsg)
      }
    }

    // AI 回复落库
    try {
      await invoke('save_message', {
        id: replyId,
        sessionId: currentSessionId,
        role: 'assistant',
        content: streamMsg.content || reply
      })
      const c = conversations.value.find(c => c.id === currentSessionId)
      if (c) c.time = new Date().toLocaleTimeString()
    } catch (e) {
      console.error('保存AI消息失败', e)
    }

  } catch (e) {
    // 如果流式没有接收到任何文本，显示错误
    if (!streamFullText) {
      streamMsg.content = `错误: ${e}`
      streamMsg.answer = `错误: ${e}`
    }
  } finally {
    unlisten()
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
  } catch (e) {
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
  } catch (e) {
    ElMessage.error(`删除失败: ${e}`)
  }
}

// ========== 对话标题编辑 ==========
const currentChatTitle = computed(() => {
  const c = conversations.value.find(c => c.id === activeConversationId.value)
  return c ? c.title : '新对话'
})
const isEditingTitle = ref(false)
const editTitleValue = ref('')

const startEditTitle = () => {
  if (activeConversationId.value) {
    editTitleValue.value = currentChatTitle.value
    isEditingTitle.value = true
    nextTick(() => {
      document.getElementById('titleInput')?.focus()
    })
  }
}

const saveTitle = async () => {
  if (!isEditingTitle.value) return
  isEditingTitle.value = false
  const newTitle = editTitleValue.value.trim()
  if (newTitle && newTitle !== currentChatTitle.value) {
    const c = conversations.value.find(c => c.id === activeConversationId.value)
    if (c) {
      c.title = newTitle
      try {
        await invoke('save_conversation', { id: c.id, title: newTitle })
      } catch (e) {
        console.error('更新标题失败', e)
      }
    }
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
    filters: [{ name: '文档', extensions: ['txt', 'doc', 'docx'] }]
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

const openSettings = async () => {
  try {
    // 使用 Tauri JS API 直接创建窗口，绕过 Rust 端 run_on_main_thread 死锁
    const { WebviewWindow } = await import('@tauri-apps/api/webviewWindow')
    const existing = await WebviewWindow.getByLabel('settings')
    if (existing) {
      await existing.show()
      await existing.setFocus()
      return
    }
    const webview = new WebviewWindow('settings', {
      url: '/#/settings',
      title: 'OneLeaf 设置',
      width: 700,
      height: 600,
      minWidth: 500,
      minHeight: 400,
      resizable: true,
      center: true,
    })
    webview.once('tauri://error', (e) => {
      console.error('设置窗口创建失败:', e)
    })
  } catch (e) {
    console.error('打开设置失败', e)
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
        <div class="conversation-list">
          <div v-for="conv in conversations" :key="conv.id" class="conversation-item"
            :class="{ active: conv.id === activeConversationId }" @click="switchConversation(conv)">
            <el-icon>
              <ChatDotRound />
            </el-icon>
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

      <div class="sidebar-footer">
        <div class="settings-btn" @click="openSettings" title="设置">
          <el-icon>
            <Setting />
          </el-icon>
          <span>设置</span>
        </div>
      </div>
    </aside>

    <!-- ========== 中间主区域 ========== -->
    <main class="main-area">
      <!-- 固定的聊天头部 -->
      <header class="chat-header">
        <div class="header-left">
          <el-tooltip :content="leftCollapsed ? '展开侧栏' : '收起侧栏'" placement="bottom">
            <div class="control-btn" @click="leftCollapsed = !leftCollapsed">
              <el-icon>
                <Expand v-if="leftCollapsed" />
                <Fold v-else />
              </el-icon>
            </div>
          </el-tooltip>
          <el-tooltip content="新对话" placement="bottom">
            <div class="control-btn" @click="startNewChat">
              <el-icon>
                <EditPen />
              </el-icon>
            </div>
          </el-tooltip>
        </div>

        <div class="header-center">
          <template v-if="activeConversationId">
            <div v-if="!isEditingTitle" class="chat-title" @click="startEditTitle" title="点击修改标题">
              {{ currentChatTitle }}
            </div>
            <input v-else v-model="editTitleValue" class="title-edit-input" @blur="saveTitle" @keyup.enter="saveTitle"
              id="titleInput" />
            <div class="chat-subtitle">内容由 AI 生成</div>
          </template>
          <template v-else>
            <div class="chat-title">新对话</div>
            <div class="chat-subtitle">有什么我能帮你的吗？</div>
          </template>
        </div>

        <div class="header-right">
          <el-icon class="control-btn">
            <Phone />
          </el-icon>
          <el-icon class="control-btn">
            <CopyDocument />
          </el-icon>
          <el-icon class="control-btn" @click="rightCollapsed = !rightCollapsed"
            :title="rightCollapsed ? '展开知识库' : '收起知识库'">
            <More />
          </el-icon>
        </div>
      </header>

      <!-- 消息区域 -->
      <div ref="chatAreaRef" class="chat-area">
        <!-- 空状态 -->
        <div v-if="messages.length === 0" class="welcome">
          <h1 class="gradient-text">有什么我能帮你的吗？</h1>
          <p class="welcome-sub">基于知识库的智能问答助手</p>
        </div>

        <!-- 消息列表 -->
        <div v-else class="message-list">
          <div v-for="msg in messages" :key="msg.id" class="message-row" :class="msg.role">
            <div class="message-avatar">
              {{ msg.role === 'user' ? '👤' : '🍃' }}
            </div>
            <div class="message-bubble">
              <div class="message-content">
                <template v-if="msg.role === 'assistant'">
                  <Thinking v-if="msg.thinking" :status="msg.thinkingDone ? 'end' : 'thinking'"
                    :model-value="!msg.thinkingDone" :content="msg.thinking" button-width="180px" max-width="100%"
                    background-color="#2a2a2a" color="#d1d1d1" style="margin-bottom: 8px;" />
                  <div class="answer-text markdown-body" v-html="renderMarkdown(msg.answer || msg.content)"></div>
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
          <textarea v-model="inputText" class="chat-input" placeholder="发消息或输入 / 选择技能"
            @keyup.enter.exact.prevent="handleSend" :rows="1" ref="textareaRef"
            @input="adjustTextareaHeight"></textarea>

          <!-- 附件标签展示 -->
          <div v-if="attachedFiles.length > 0" class="attachments-bar">
            <div v-for="(att, index) in attachedFiles" :key="att.fileId" class="attachment-tag">
              <span class="att-icon">{{ att.fileType === 'video' ? '🎬' : '📎' }}</span>
              <span class="att-name">{{ att.fileName }}</span>
              <span class="att-remove" @click="removeAttachment(index)">×</span>
            </div>
          </div>

          <div class="input-tools-row">
            <div class="input-tools-left">
              <div class="action-btn" @click="handleChatUploadVideo" :class="{ loading: isTranscribing }"
                :disabled="isTranscribing">
                <el-icon v-if="isTranscribing" class="is-loading">
                  <Loading />
                </el-icon>
                <el-icon v-else>
                  <Lightning />
                </el-icon>
                <span>{{ isTranscribing ? '转写中...' : '视频转写' }}</span>
              </div>

              <div class="action-btn" @click="handleChatUploadFile">
                <el-icon>
                  <Paperclip />
                </el-icon>
                <span>上传文件</span>
              </div>

              <div class="action-btn" @click="handleChatAttachKbFile">
                <el-icon>
                  <Collection />
                </el-icon>
                <span>知识库</span>
              </div>

              <template v-if="isDoubao">
                <div class="pill-btn web-search-btn" :class="{ active: isWebSearchEnabled }"
                  @click="isWebSearchEnabled = !isWebSearchEnabled">
                  <span>🌍 联网</span>
                </div>

                <el-dropdown trigger="click" @command="handleThinkingDepth">
                  <div class="pill-btn dropdown-trigger" :class="{ active: thinkingDepth !== 'none' }">
                    <span>🧠 深度思考: {{ thinkingDepthLabel }}</span>
                    <el-icon class="el-icon--right">
                      <ArrowDown />
                    </el-icon>
                  </div>
                  <template #dropdown>
                    <el-dropdown-menu>
                      <el-dropdown-item command="none"
                        :class="{ 'is-active': thinkingDepth === 'none' }">关闭</el-dropdown-item>
                      <el-dropdown-item command="low"
                        :class="{ 'is-active': thinkingDepth === 'low' }">低</el-dropdown-item>
                      <el-dropdown-item command="medium"
                        :class="{ 'is-active': thinkingDepth === 'medium' }">中</el-dropdown-item>
                      <el-dropdown-item command="high"
                        :class="{ 'is-active': thinkingDepth === 'high' }">高</el-dropdown-item>
                    </el-dropdown-menu>
                  </template>
                </el-dropdown>
              </template>
            </div>

            <div class="input-tools-right">
              <div v-if="isCurrentLoading" class="send-btn stop-btn active" @click="handleStopGenerate" title="停止生成">
                <span style="font-size: 18px; line-height: 1;">⏹</span>
              </div>
              <div v-else class="send-btn" :class="{ active: inputText.trim() }" @click="handleSend">
                <el-icon>
                  <Top />
                </el-icon>
              </div>
            </div>
          </div>
        </div>
      </div>

    </main>

    <!-- ========== 右侧边栏：知识库 ========== -->
    <aside class="sidebar sidebar-right" :class="{ collapsed: rightCollapsed }">
      <div class="sidebar-header">
        <span class="sidebar-title">本地知识库</span>
      </div>

      <div class="sidebar-content">
        <!-- 搜索框 -->
        <div class="kb-search">
          <el-input v-model="searchQuery" placeholder="搜索知识库..." :prefix-icon="Search" clearable size="small"
            @input="handleSearch" @clear="clearSearch" />
        </div>

        <!-- 搜索结果 -->
        <div v-if="hasSearched" class="kb-search-results">
          <div class="search-header">
            <span>搜索结果</span>
            <el-button link size="small" @click="clearSearch">返回列表</el-button>
          </div>
          <div v-if="isSearching" class="empty-hint">搜索中...</div>
          <div v-else-if="searchResults.length === 0" class="empty-hint">未找到匹配内容</div>
          <div v-for="result in searchResults" :key="result.document.id" class="search-result-item"
            @click="showDocDetail(result.document)">
            <div class="sr-title">
              <span class="kb-emoji-icon">{{ getDocIcon(result.document) }}</span>
              <span class="sr-name">{{ result.document.name }}</span>
              <span class="sr-score" :title="'Score: ' + result.relevance">{{ result.relevance >= 0.1 ?
                (result.relevance *
                  100).toFixed(0) + '%' : result.relevance.toFixed(3) }}</span>
            </div>
            <div class="sr-snippet">{{ result.snippet }}</div>
          </div>
        </div>

        <!-- 操作按钮和文档列表（非搜索状态） -->
        <template v-else>
          <div class="kb-actions">
            <el-button size="small" @click="handleUploadFile">
              <el-icon>
                <Document />
              </el-icon>添加文档
            </el-button>
            <el-button size="small" @click="handleUploadVideo">
              <el-icon>
                <VideoPlay />
              </el-icon>视频转写
            </el-button>
          </div>

          <div class="kb-list">
            <div v-for="doc in kbDocuments" :key="doc.id" class="kb-item clickable" @click="showDocDetail(doc)">
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
    <el-dialog v-model="detailDialogVisible" :title="selectedDoc?.name || '文档详情'" width="60%"
      class="doc-preview-dialog">
      <div v-if="selectedDoc" class="doc-detail-content">
        <div class="doc-meta-bar">
          <el-tag size="small" :type="selectedDoc.category === 'video-transcript' ? 'warning' : 'info'">
            {{ getDocTag(selectedDoc) }}
          </el-tag>
          <el-button v-if="canOpenExternal(selectedDoc)" size="small" type="primary" plain
            @click="handleOpenFile(selectedDoc)">
            用系统程序打开原件
          </el-button>
        </div>
        <el-input :model-value="selectedDoc.content" type="textarea" :rows="18" readonly class="detail-textarea" />
      </div>
      <template #footer>
        <el-button @click="detailDialogVisible = false">关闭</el-button>
      </template>
    </el-dialog>

    <!-- 聊天知识库选择弹窗 -->
    <el-dialog v-model="kbFileDialogVisible" title="选择知识库文件" width="500px">
      <div class="kb-dialog-list">
        <div v-if="kbDocuments.length === 0" class="empty-hint" style="color: #666;">暂无知识库文件可以引用</div>
        <div v-for="doc in kbDocuments" :key="doc.id" class="kb-doc-item" @click="selectKbDocForChat(doc)">
          <span class="kb-emoji-icon">{{ getDocIcon(doc) }}</span>
          <span class="kb-dialog-doc-name">{{ doc.name }}</span>
        </div>
      </div>
    </el-dialog>
  </div>
</template>

<style scoped>
.web-search-btn {
  color: #888;
}

.web-search-btn.active {
  color: #4facfe;
  border-color: rgba(79, 172, 254, 0.4);
  background: rgba(79, 172, 254, 0.1);
}

.pill-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 6px 12px;
  border-radius: 18px;
  border: 1px solid rgba(255, 255, 255, 0.1);
  background: rgba(255, 255, 255, 0.02);
  color: #888;
  font-size: 0.82rem;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.2s;
  user-select: none;
}

.pill-btn:hover {
  background: rgba(255, 255, 255, 0.08);
  color: #e3e3e3;
}

.dropdown-trigger {
  outline: none;
}

.dropdown-trigger.active {
  color: #a450ff;
  border-color: rgba(164, 80, 255, 0.4);
  background: rgba(164, 80, 255, 0.1);
}

:deep(.el-dropdown-menu__item.is-active) {
  color: #a450ff;
  background-color: rgba(164, 80, 255, 0.1);
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
  border-right: 1px solid rgba(255, 255, 255, 0.06);
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
  border-left: 1px solid rgba(255, 255, 255, 0.06);
}

.sidebar-header {
  display: flex;
  align-items: center;
  padding: 16px 12px;
  gap: 10px;
  border-bottom: 1px solid rgba(255, 255, 255, 0.06);
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

.sidebar-footer {
  padding: 8px;
  border-top: 1px solid rgba(255, 255, 255, 0.06);
  flex-shrink: 0;
}

.settings-btn {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 14px;
  border-radius: 10px;
  cursor: pointer;
  color: #888;
  font-size: 0.88rem;
  transition: all 0.15s;
  white-space: nowrap;
}

.settings-btn:hover {
  background: rgba(255, 255, 255, 0.06);
  color: #e3e3e3;
}

.settings-btn .el-icon {
  font-size: 1.1rem;
}

/* ========== 固定聊天头部 ========== */
.chat-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 24px;
  border-bottom: 1px solid rgba(255, 255, 255, 0.04);
  background: transparent;
  z-index: 10;
  flex-shrink: 0;
}

.header-left,
.header-right {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 120px;
}

.header-right {
  justify-content: flex-end;
}

.header-center {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
}

.chat-title {
  font-size: 1rem;
  font-weight: 600;
  color: #e3e3e3;
  cursor: pointer;
  padding: 2px 8px;
  border-radius: 6px;
  transition: background 0.2s;
}

.chat-title:hover {
  background: rgba(255, 255, 255, 0.06);
}

.title-edit-input {
  background: rgba(255, 255, 255, 0.08);
  border: 1px solid rgba(79, 172, 254, 0.4);
  color: #e3e3e3;
  font-size: 1rem;
  font-weight: 600;
  padding: 2px 8px;
  border-radius: 6px;
  outline: none;
  text-align: center;
  width: 200px;
  font-family: inherit;
}

.chat-subtitle {
  font-size: 0.75rem;
  color: #888;
  margin-top: 2px;
}

.control-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border-radius: 6px;
  cursor: pointer;
  color: #adb5bd;
  transition: all 0.2s;
}

.control-btn:hover {
  background: rgba(255, 255, 255, 0.1);
  color: #e3e3e3;
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
  background: rgba(255, 255, 255, 0.06);
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
  padding: 16px 24px 16px;
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
  background: rgba(255, 255, 255, 0.06);
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
  background: rgba(255, 255, 255, 0.06);
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

.typing-indicator span:nth-child(1) {
  animation-delay: -0.32s;
}

.typing-indicator span:nth-child(2) {
  animation-delay: -0.16s;
}

.typing-indicator span:nth-child(3) {
  animation-delay: 0s;
}

@keyframes bounce {

  0%,
  80%,
  100% {
    transform: scale(0.6);
    opacity: 0.4;
  }

  40% {
    transform: scale(1);
    opacity: 1;
  }
}

@keyframes fadeIn {
  from {
    opacity: 0;
    transform: translateY(8px);
  }

  to {
    opacity: 1;
    transform: translateY(0);
  }
}

/* ========== 底部输入框 ========== */
.input-area {
  padding: 12px 24px 20px;
  border-top: 1px solid rgba(255, 255, 255, 0.04);
}

.input-wrapper {
  display: flex;
  flex-direction: column;
  gap: 12px;
  max-width: 780px;
  margin: 0 auto;
  background: #22223a;
  color: #e3e3e3;
  border: 1px solid rgba(255, 255, 255, 0.1);
  border-radius: 16px;
  padding: 16px;
  transition: border-color 0.2s;
}

.input-wrapper:focus-within {
  border-color: rgba(79, 172, 254, 0.4);
}

.chat-input {
  width: 100%;
  background: transparent;
  border: none;
  outline: none;
  color: #e3e3e3;
  font-size: 0.95rem;
  line-height: 1.5;
  font-family: inherit;
  resize: none;
  max-height: 180px;
  padding: 0;
}

.chat-input::placeholder {
  color: #555;
}

.attachments-bar {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.attachment-tag {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 4px 10px;
  border-radius: 8px;
  background: rgba(79, 172, 254, 0.1);
  border: 1px solid rgba(79, 172, 254, 0.2);
  font-size: 0.8rem;
  color: #bbb;
}

.att-icon {
  font-size: 0.9rem;
}

.att-name {
  max-width: 120px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.att-remove {
  cursor: pointer;
  color: #888;
  font-size: 1rem;
  line-height: 1;
  margin-left: 2px;
}

.att-remove:hover {
  color: #ff4d4f;
}

.input-tools-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.input-tools-left {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.input-tools-right {
  display: flex;
  align-items: center;
}

.action-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  color: #888;
  cursor: pointer;
  padding: 6px 12px;
  border-radius: 18px;
  font-size: 0.82rem;
  transition: all 0.2s;
  user-select: none;
  font-weight: 500;
  background: rgba(255, 255, 255, 0.02);
}

.action-btn .el-icon {
  font-size: 1rem;
}

.action-btn:hover {
  background: rgba(255, 255, 255, 0.08);
  color: #e3e3e3;
}

.action-btn.loading {
  cursor: not-allowed;
  opacity: 0.7;
}

.action-btn.loading:hover {
  background: rgba(255, 255, 255, 0.02);
  color: #888;
}

.send-btn {
  width: 32px;
  height: 32px;
  border-radius: 50%;
  background-color: #e5e5e5;
  color: #999;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: not-allowed;
  transition: all 0.2s;
}

.send-btn .el-icon {
  font-size: 1.2rem;
}

.send-btn.active {
  background-color: #0066ff;
  color: #fff;
  cursor: pointer;
}

.send-btn.active:hover {
  background-color: #005ce6;
}

.send-btn.stop-btn {
  background-color: #444;
  color: #fff;
  cursor: pointer;
}

.send-btn.stop-btn:hover {
  background-color: #555;
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
  border: 1px dashed rgba(255, 255, 255, 0.12);
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
  background: rgba(255, 255, 255, 0.04);
  border-color: rgba(255, 255, 255, 0.08);
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
  border: 1px solid rgba(255, 255, 255, 0.04);
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
  background: rgba(255, 255, 255, 0.05);
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
  margin: 0 !important;
  /* 强制左对齐，防止展开后居中 */
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
  box-shadow: inset 0 2px 4px rgba(0, 0, 0, 0.2) !important;
}

/* ========== Markdown 渲染样式 ========== */
.answer-text.markdown-body {
  word-break: break-word;
  line-height: 1.7;
  color: #e3e3e3;
}

.markdown-body h1,
.markdown-body h2,
.markdown-body h3,
.markdown-body h4 {
  margin: 16px 0 8px;
  font-weight: 600;
  color: #f0f0f0;
  line-height: 1.4;
}

.markdown-body h1 {
  font-size: 1.4em;
}

.markdown-body h2 {
  font-size: 1.25em;
}

.markdown-body h3 {
  font-size: 1.1em;
}

.markdown-body h4 {
  font-size: 1em;
}

.markdown-body p {
  margin: 8px 0;
}

.markdown-body ul,
.markdown-body ol {
  padding-left: 1.5em;
  margin: 8px 0;
}

.markdown-body li {
  margin: 4px 0;
}

.markdown-body li>p {
  margin: 4px 0;
}

.markdown-body code {
  background: rgba(255, 255, 255, 0.08);
  color: #e8b86d;
  padding: 2px 6px;
  border-radius: 4px;
  font-size: 0.88em;
  font-family: 'Cascadia Code', 'Fira Code', Consolas, monospace;
}

.markdown-body pre {
  background: #141422;
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 8px;
  padding: 14px;
  margin: 12px 0;
  overflow-x: auto;
}

.markdown-body pre code {
  background: none;
  color: #d1d1d1;
  padding: 0;
  font-size: 0.85em;
  line-height: 1.6;
}

.markdown-body blockquote {
  border-left: 3px solid rgba(79, 172, 254, 0.5);
  padding: 4px 12px;
  margin: 12px 0;
  color: #aaa;
  background: rgba(79, 172, 254, 0.04);
  border-radius: 0 6px 6px 0;
}

.markdown-body hr {
  border: none;
  border-top: 1px solid rgba(255, 255, 255, 0.08);
  margin: 16px 0;
}

.markdown-body table {
  border-collapse: collapse;
  width: 100%;
  margin: 12px 0;
}

.markdown-body th,
.markdown-body td {
  border: 1px solid rgba(255, 255, 255, 0.1);
  padding: 8px 12px;
  text-align: left;
}

.markdown-body th {
  background: rgba(255, 255, 255, 0.05);
  font-weight: 600;
}

.markdown-body strong {
  color: #f5f5f5;
  font-weight: 600;
}

.markdown-body a {
  color: #4facfe;
  text-decoration: none;
}

.markdown-body a:hover {
  text-decoration: underline;
}

.markdown-body img {
  max-width: 100%;
  border-radius: 8px;
}

.kb-dialog-list {
  max-height: 400px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.kb-doc-item {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px;
  background: #f8f9fa;
  border: 1px solid #ebeef5;
  border-radius: 8px;
  cursor: pointer;
  transition: all 0.2s;
}

.kb-doc-item:hover {
  background: #ecf5ff;
  border-color: #b3d8ff;
}

.kb-dialog-doc-name {
  flex: 1;
  font-weight: 500;
  color: #303133;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
