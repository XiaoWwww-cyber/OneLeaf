<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { ElMessage } from 'element-plus'
import { Download, Check, Loading } from '@element-plus/icons-vue'

// ========== ASR 模型状态 ==========
interface AsrModelStatus {
  name: string
  description: string
  size_mb: number
  is_installed: boolean
  model_dir: string
}

const asrStatus = ref<AsrModelStatus | null>(null)
const asrLoading = ref(true)
const asrDownloading = ref(false)
const asrProgress = ref(0)
const asrProgressFile = ref('')
const asrProgressBytes = ref('')

// ========== 初始化 ==========
onMounted(async () => {
  await checkAsrModel()

  listen<any>('model-download-progress', (event) => {
    const d = event.payload
    asrProgressFile.value = d.file_name
    asrProgress.value = Math.round(d.progress * 100)
    if (d.total_bytes > 0) {
      asrProgressBytes.value = `${formatBytes(d.downloaded_bytes)} / ${formatBytes(d.total_bytes)}`
    }
    if (d.status === 'completed' && (d.file_name === 'tokens.txt' || d.file_name === 'model.onnx')) {
      // 如果 tokens.txt 完成或者是唯一的跳过文件，尝试停止加载
      checkAsrModel()
      if (d.file_name === 'tokens.txt') {
        asrDownloading.value = false
        ElMessage.success('ASR 模型下载完成！')
      }
    }
    if (d.status === 'failed') {
      asrDownloading.value = false
      ElMessage.error(`下载失败: ${d.file_name}`)
    }
  })
})

const checkAsrModel = async () => {
  asrLoading.value = true
  try {
    asrStatus.value = await invoke<AsrModelStatus>('check_asr_model')
  } catch (e) {
    console.error('check asr model:', e)
  } finally {
    asrLoading.value = false
  }
}

const handleDownloadAsr = async () => {
  asrDownloading.value = true
  asrProgress.value = 0
  asrProgressBytes.value = ''
  try {
    await invoke('download_asr_model')
    asrDownloading.value = false
    await checkAsrModel()
  } catch (e) {
    asrDownloading.value = false
    ElMessage.error(`下载失败: ${e}`)
  }
}

const formatBytes = (bytes: number): string => {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}
</script>

<template>
  <div class="settings-page">
    <div class="settings-header">
      <h1>设置</h1>
      <p class="settings-sub">管理 OneLeaf 应用配置</p>
    </div>

    <div class="settings-body">
      <!-- ====== ASR 模型管理 ====== -->
      <div class="settings-card">
        <div class="card-header">
          <div class="card-icon asr">🎙️</div>
          <div>
            <h2>语音识别模型</h2>
            <p>SenseVoice — 用于视频转写和语音识别</p>
          </div>
        </div>

        <div class="card-body">
          <div v-if="asrLoading" class="status-loading">
            <el-icon class="is-loading"><Loading /></el-icon>
            <span>检测模型状态...</span>
          </div>

          <template v-else-if="asrStatus">
            <div class="model-info">
              <div class="model-name">
                {{ asrStatus.name }}
                <el-tag v-if="asrStatus.is_installed" type="success" size="small" effect="dark" round>
                  <el-icon><Check /></el-icon> 已安装
                </el-tag>
                <el-tag v-else type="warning" size="small" effect="dark" round>
                  未安装
                </el-tag>
              </div>
              <div class="model-desc">{{ asrStatus.description }}</div>
              <div class="model-meta">
                <span>大小: ~{{ asrStatus.size_mb }}MB</span>
                <span>语言: 中/英/日/韩/粤</span>
              </div>
            </div>

            <!-- 下载进度 -->
            <div v-if="asrDownloading" class="download-progress">
              <div class="progress-info">
                <span>正在下载: {{ asrProgressFile }}</span>
                <span>{{ asrProgressBytes }}</span>
              </div>
              <el-progress
                :percentage="asrProgress"
                :stroke-width="8"
                color="#4facfe"
              />
            </div>
          </template>
        </div>

        <div class="card-footer">
          <template v-if="asrStatus?.is_installed">
            <span class="model-dir" :title="asrStatus?.model_dir">
              📂 {{ asrStatus?.model_dir }}
            </span>
          </template>
          <template v-else>
            <el-button
              type="primary"
              :loading="asrDownloading"
              @click="handleDownloadAsr"
            >
              <el-icon v-if="!asrDownloading"><Download /></el-icon>
              {{ asrDownloading ? '下载中...' : '下载模型' }}
            </el-button>
          </template>
        </div>
      </div>

      <!-- ====== 关于 ====== -->
      <div class="settings-card">
        <div class="card-header">
          <div class="card-icon about">🍃</div>
          <div>
            <h2>关于 OneLeaf</h2>
            <p>智能知识库助手</p>
          </div>
        </div>
        <div class="card-body">
          <div class="about-info">
            <div class="about-row"><span>版本</span><span>0.1.0</span></div>
            <div class="about-row"><span>框架</span><span>Tauri 2 + Vue 3</span></div>
            <div class="about-row"><span>ASR 引擎</span><span>Sherpa-ONNX (SenseVoice)</span></div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.settings-page {
  height: 100vh;
  overflow-y: auto;
  background: #1a1a2e;
  color: #e3e3e3;
  padding: 32px 40px;
  font-family: 'Inter', 'Segoe UI', system-ui, sans-serif;
}

.settings-header h1 {
  font-size: 1.6rem;
  font-weight: 700;
  margin: 0;
  background: linear-gradient(135deg, #4facfe, #a78bfa);
  background-clip: text;
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
}

.settings-sub {
  color: #666;
  font-size: 0.9rem;
  margin-top: 4px;
}

.settings-body {
  margin-top: 28px;
  display: flex;
  flex-direction: column;
  gap: 20px;
}

/* ====== Card ====== */
.settings-card {
  background: #22223a;
  border: 1px solid rgba(255,255,255,0.06);
  border-radius: 16px;
  overflow: hidden;
}

.card-header {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 20px 24px;
  border-bottom: 1px solid rgba(255,255,255,0.04);
}

.card-icon {
  width: 40px;
  height: 40px;
  border-radius: 12px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 1.3rem;
}

.card-icon.asr {
  background: rgba(244, 63, 94, 0.12);
}

.card-icon.about {
  background: rgba(79, 172, 254, 0.12);
}

.card-header h2 {
  font-size: 0.95rem;
  font-weight: 600;
  margin: 0;
  color: #e3e3e3;
}

.card-header p {
  font-size: 0.8rem;
  color: #777;
  margin: 2px 0 0;
}

.card-body {
  padding: 20px 24px;
}

.card-footer {
  padding: 14px 24px;
  background: rgba(255,255,255,0.02);
  border-top: 1px solid rgba(255,255,255,0.04);
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 10px;
}

/* ====== Model Info ====== */
.status-loading {
  display: flex;
  align-items: center;
  gap: 8px;
  color: #888;
  font-size: 0.85rem;
}

.model-info {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.model-name {
  font-size: 0.95rem;
  font-weight: 600;
  display: flex;
  align-items: center;
  gap: 10px;
}

.model-desc {
  font-size: 0.82rem;
  color: #888;
}

.model-meta {
  display: flex;
  gap: 16px;
  font-size: 0.78rem;
  color: #666;
  margin-top: 4px;
}

.model-dir {
  font-size: 0.75rem;
  color: #555;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 400px;
}

/* ====== Download Progress ====== */
.download-progress {
  margin-top: 16px;
  padding: 14px;
  background: rgba(255,255,255,0.03);
  border-radius: 12px;
}

.progress-info {
  display: flex;
  justify-content: space-between;
  font-size: 0.78rem;
  color: #888;
  margin-bottom: 8px;
}

/* ====== About ====== */
.about-info {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.about-row {
  display: flex;
  justify-content: space-between;
  font-size: 0.85rem;
}

.about-row span:first-child {
  color: #888;
}

.about-row span:last-child {
  color: #ccc;
}
</style>
