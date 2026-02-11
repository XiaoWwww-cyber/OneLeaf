<script setup lang="ts">
import { ref } from 'vue'
import { useRouter } from 'vue-router'
import { VideoPlay, Document, Position } from '@element-plus/icons-vue'
import { invoke } from '@tauri-apps/api/core'
import { ElMessage } from 'element-plus'
import VideoTranscriptDialog from '../components/VideoTranscriptDialog.vue'

const router = useRouter()
const input = ref('')
const videoDialogRef = ref()

const handleUploadFile = async () => {
    // Basic file upload trigger (mock implementation as browser standard input isn't directly usable here without Tauri dialog)
    // In a real Tauri app, invoke('tauri-plugin-dialog:open')
    // For now, let's just show a message or use input type=file hidden trick
    const input = document.createElement('input')
    input.type = 'file'
    input.accept = '.txt,.md,.docx,.pdf'
    input.onchange = async (e: any) => {
        const file = e.target.files[0]
        if (!file) return
        
        // Read file content (for text files) to pass to backend if needed, or just pass path if backend can access it
        // Note: Browser JS cannot give full path due to security, but Tauri has ways.
        // Assuming we are in Tauri and drag-drop gives path, or we use tauri-plugin-dialog.
        // Since we didn't add the dialog plugin to Cargo.toml yet, we'll assume we can't get path easily from input.
        // BUT, we can read the file content in JS and send it.
        
        if (file) {
            const reader = new FileReader()
            reader.onload = async (e) => {
                const content = e.target?.result as string
                try {
                    await invoke('add_document_to_kb', { 
                        content, 
                        category: 'documents',
                        filePath: file.name // Just name for ID
                    })
                     ElMessage.success(`Added ${file.name} to Knowledge Base`)
                } catch (err) {
                    ElMessage.error(`Failed: ${err}`)
                }
            }
            reader.readAsText(file) // Simplification for text/md
        }
    }
    input.click()
}

const handleUploadVideo = async () => {
    const input = document.createElement('input')
    input.type = 'file'
    input.accept = 'video/*'
    input.onchange = (e: any) => {
        const file = e.target.files[0]
        if (file) {
             // Pass the file object to the dialog component which handles the logic
             // Note: The dialog expects a path property for backend.
             // If we can't get it, we just pass the name and mock it for now.
             videoDialogRef.value?.open({ raw: file })
        }
    }
    input.click()
}

const handleSend = () => {
    if (input.value.trim()) {
        router.push({ name: 'chat', query: { q: input.value } })
    }
}
</script>

<template>
  <div class="home-container">
    <div class="welcome-section">
      <h1 class="gradient-text">你好, OneLeaf</h1>
      <p class="subtitle">今天我可以帮你做些什么？</p>
    </div>

    <div class="input-area">
      <el-input
        v-model="input"
        class="main-input"
        placeholder="输入问题或上传文件..."
        @keyup.enter="handleSend"
      >
        <template #prefix>
            <el-icon class="icon-btn" @click="handleUploadFile"><Document /></el-icon>
            <el-icon class="icon-btn" @click="handleUploadVideo"><VideoPlay /></el-icon>
        </template>
        <template #suffix>
            <el-button type="primary" circle @click="handleSend">
                <el-icon><Position /></el-icon>
            </el-button>
        </template>
      </el-input>
    </div>
    
    <VideoTranscriptDialog ref="videoDialogRef" />
  </div>
</template>

<style scoped>
.home-container {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100vh;
    background-color: #1a1a1a;
    color: white;
}

.welcome-section {
    text-align: center;
    margin-bottom: 40px;
}

.gradient-text {
    font-size: 3rem;
    background: linear-gradient(to right, #4facfe 0%, #00f2fe 100%);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    margin-bottom: 10px;
}

.subtitle {
    font-size: 1.5rem;
    color: #888;
}

.input-area {
    width: 60%;
    max-width: 800px;
}

.main-input :deep(.el-input__wrapper) {
    background-color: #2c2c2c;
    box-shadow: none;
    border-radius: 24px;
    padding: 12px 20px;
}

.main-input :deep(.el-input__inner) {
    color: white;
    font-size: 1.1rem;
}

.icon-btn {
    font-size: 1.2rem;
    margin-right: 10px;
    cursor: pointer;
    color: #aaa;
    transition: color 0.3s;
}

.icon-btn:hover {
    color: white;
}
</style>
