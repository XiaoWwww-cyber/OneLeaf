<script setup lang="ts">
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import type { UploadFile } from 'element-plus'

const visible = ref(false)
const transcript = ref('')
const loading = ref(false)
const currentVideoPath = ref('')

const open = async (file: UploadFile) => {
    if (!file.raw) return null
    // In a real app we'd get the full path, but web API limits this.
    // Tauri drag-drop returns paths, but file input might not.
    // Assuming we have a path for now or mocking it.
    visible.value = true
    loading.value = true
    
    // Mock path for demo if not available
    const path = (file.raw as any).path || file.name
    currentVideoPath.value = path

    try {
        const res = await invoke<{text: string}>('transcribe_video', { videoPath: path })
        transcript.value = res.text
    } catch (e) {
        transcript.value = `Error: ${e}`
    } finally {
        loading.value = false
    }
}

const handleSave = async () => {
    try {
        await invoke('add_document_to_kb', { 
            content: transcript.value,
            category: 'video-transcript',
            filePath: currentVideoPath.value 
        })
        visible.value = false
    } catch (e) {
        console.error(e)
    }
}

defineExpose({ open })
</script>

<template>
  <el-dialog v-model="visible" title="Review Transcript" width="60%">
    <div v-loading="loading">
        <el-input 
            v-model="transcript" 
            type="textarea" 
            :rows="15" 
            placeholder="Transcript will appear here..."
        />
    </div>
    <template #footer>
        <span class="dialog-footer">
            <el-button @click="visible = false">Cancel</el-button>
            <el-button type="primary" @click="handleSave">Save to Knowledge Base</el-button>
        </span>
    </template>
  </el-dialog>
</template>
