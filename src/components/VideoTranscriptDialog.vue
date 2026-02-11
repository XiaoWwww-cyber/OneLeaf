<script setup lang="ts">
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { ElMessage } from 'element-plus'

const visible = ref(false)
const transcript = ref('')
const loading = ref(false)
const currentVideoPath = ref('')

// 接收真实文件路径（从 Tauri dialog 获取）
const open = async (filePath: string) => {
    if (!filePath) return

    visible.value = true
    loading.value = true
    currentVideoPath.value = filePath

    try {
        const res = await invoke<{text: string}>('transcribe_video', { videoPath: filePath })
        transcript.value = res.text
    } catch (e) {
        transcript.value = `转写失败: ${e}`
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
        ElMessage.success('已保存到知识库')
        visible.value = false
    } catch (e: any) {
        ElMessage.error(`保存失败: ${e}`)
    }
}

defineExpose({ open })
</script>

<template>
  <el-dialog v-model="visible" title="视频转写结果" width="60%">
    <div v-loading="loading" element-loading-text="正在转写中，请稍候...">
        <el-input 
            v-model="transcript" 
            type="textarea" 
            :rows="15" 
            placeholder="转写结果将在此显示..."
        />
    </div>
    <template #footer>
        <span class="dialog-footer">
            <el-button @click="visible = false">取消</el-button>
            <el-button type="primary" @click="handleSave" :disabled="!transcript">保存到知识库</el-button>
        </span>
    </template>
  </el-dialog>
</template>
