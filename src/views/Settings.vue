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

// ========== Embedding 模型状态 ==========
interface EmbeddingModelStatus {
  name: string
  description: string
  size_mb: number
  is_installed: boolean
  model_dir: string
}

const embeddingStatus = ref<EmbeddingModelStatus | null>(null)
const embeddingLoading = ref(true)
const embeddingDownloading = ref(false)
const embeddingProgress = ref(0)
const embeddingProgressFile = ref('')
const embeddingProgressBytes = ref('')

// ========== AI 设置 ==========
interface AiSettings {
  provider: string
  doubao_api_key: string | null
  openai_api_key: string | null
  deepseek_api_key: string | null
  lm_studio_url: string
}

const aiSettings = ref<AiSettings>({
  provider: 'lmstudio',
  doubao_api_key: null,
  openai_api_key: null,
  deepseek_api_key: null,
  lm_studio_url: 'http://localhost:1234',
})
const aiSaving = ref(false)
const lmStudioRunning = ref<boolean | null>(null)
const lmStudioChecking = ref(false)

const providerOptions = [
  { label: 'LM Studio (本地)', value: 'lmstudio' },
  { label: '豆包', value: 'doubao' },
  { label: 'OpenAI', value: 'openai' },
  { label: 'DeepSeek', value: 'deepseek' },
]

// ========== 初始化 ==========
onMounted(async () => {
  await checkAsrModel()
  await checkEmbeddingModel()
  await loadAiSettings()

  listen<any>('model-download-progress', (event) => {
    const d = event.payload
    asrProgressFile.value = d.file_name
    asrProgress.value = Math.round(d.progress * 100)
    if (d.total_bytes > 0) {
      asrProgressBytes.value = `${formatBytes(d.downloaded_bytes)} / ${formatBytes(d.total_bytes)}`
    }
    if (d.status === 'completed' && (d.file_name === 'tokens.txt' || d.file_name === 'model.onnx')) {
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

  listen<any>('embedding-model-progress', (event) => {
    const d = event.payload
    embeddingProgressFile.value = d.file_name
    embeddingProgress.value = Math.round(d.progress * 100)
    if (d.total_bytes > 0) {
      embeddingProgressBytes.value = `${formatBytes(d.downloaded_bytes)} / ${formatBytes(d.total_bytes)}`
    }
    if (d.status === 'completed' && (d.file_name === 'vocab.txt' || d.file_name === 'model.onnx')) {
      checkEmbeddingModel()
      if (d.file_name === 'vocab.txt') {
        embeddingDownloading.value = false
        ElMessage.success('向量知识库模型下载完成！')
      }
    }
    if (d.status === 'failed') {
      embeddingDownloading.value = false
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

const checkEmbeddingModel = async () => {
  embeddingLoading.value = true
  try {
    embeddingStatus.value = await invoke<EmbeddingModelStatus>('get_embedding_model_status')
  } catch (e) {
    console.error('check embedding model:', e)
  } finally {
    embeddingLoading.value = false
  }
}

const handleDownloadEmbedding = async () => {
  embeddingDownloading.value = true
  embeddingProgress.value = 0
  embeddingProgressBytes.value = ''
  try {
    await invoke('download_embedding_model')
    embeddingDownloading.value = false
    await checkEmbeddingModel()
  } catch (e) {
    embeddingDownloading.value = false
    ElMessage.error(`下载失败: ${e}`)
  }
}

const formatBytes = (bytes: number): string => {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

// ========== AI 设置操作 ==========
const loadAiSettings = async () => {
  try {
    const settings = await invoke<AiSettings>('get_ai_settings')
    aiSettings.value = settings
  } catch (e) {
    console.warn('加载 AI 设置失败:', e)
  }
}

const saveAiSettings = async () => {
  aiSaving.value = true
  try {
    await invoke('update_ai_settings', { settings: aiSettings.value })
    ElMessage.success('AI 设置已保存')
  } catch (e) {
    ElMessage.error(`保存失败: ${e}`)
  } finally {
    aiSaving.value = false
  }
}

const checkLmStudio = async () => {
  lmStudioChecking.value = true
  try {
    lmStudioRunning.value = await invoke<boolean>('check_lm_studio')
    if (lmStudioRunning.value) {
      ElMessage.success('LM Studio 连接成功')
    } else {
      ElMessage.warning('LM Studio 未运行')
    }
  } catch (e) {
    lmStudioRunning.value = false
    ElMessage.error('检测失败')
  } finally {
    lmStudioChecking.value = false
  }
}
</script>

<template>
  <div class="settings-page">
    <div class="settings-header">
      <h1>设置</h1>
      <p class="settings-sub">管理 OneLeaf 应用配置</p>
    </div>

    <div class="settings-body">
      <!-- ====== AI 配置 ====== -->
      <div class="settings-card">
        <div class="card-header">
          <div class="card-icon ai">🤖</div>
          <div>
            <h2>AI 服务</h2>
            <p>配置智能问答引擎（不配置时使用全文搜索）</p>
          </div>
        </div>

        <div class="card-body">
          <div class="form-group">
            <label>AI 提供者</label>
            <el-select v-model="aiSettings.provider" style="width: 100%">
              <el-option
                v-for="opt in providerOptions"
                :key="opt.value"
                :label="opt.label"
                :value="opt.value"
              />
            </el-select>
          </div>

          <!-- 豆包 -->
          <div v-if="aiSettings.provider === 'doubao'" class="form-group">
            <label>豆包 API Key</label>
            <el-input
              v-model="aiSettings.doubao_api_key"
              placeholder="输入豆包 API Key"
              show-password
            />
          </div>

          <!-- OpenAI -->
          <div v-if="aiSettings.provider === 'openai'" class="form-group">
            <label>OpenAI API Key</label>
            <el-input
              v-model="aiSettings.openai_api_key"
              placeholder="输入 OpenAI API Key"
              show-password
            />
          </div>

          <!-- DeepSeek -->
          <div v-if="aiSettings.provider === 'deepseek'" class="form-group">
            <label>DeepSeek API Key</label>
            <el-input
              v-model="aiSettings.deepseek_api_key"
              placeholder="输入 DeepSeek API Key"
              show-password
            />
          </div>

          <!-- LM Studio -->
          <div v-if="aiSettings.provider === 'lmstudio'" class="form-group">
            <label>LM Studio 地址</label>
            <div style="display: flex; gap: 8px;">
              <el-input
                v-model="aiSettings.lm_studio_url"
                placeholder="http://localhost:1234"
                style="flex: 1"
              />
              <el-button :loading="lmStudioChecking" @click="checkLmStudio">
                检测
              </el-button>
            </div>
            <div v-if="lmStudioRunning !== null" class="lm-status">
              <el-tag v-if="lmStudioRunning" type="success" size="small" effect="dark" round>已连接</el-tag>
              <el-tag v-else type="danger" size="small" effect="dark" round>未运行</el-tag>
            </div>
          </div>
        </div>

        <div class="card-footer">
          <el-button type="primary" :loading="aiSaving" @click="saveAiSettings">
            保存设置
          </el-button>
        </div>
      </div>

      <!-- ====== Embedding 模型管理 ====== -->
      <div class="settings-card">
        <div class="card-header">
          <div class="card-icon embedding">🧠</div>
          <div>
            <h2>向量检索模型</h2>
            <p>BGE-small-zh — 用于知识库语义搜索，提升问答匹配准确率</p>
          </div>
        </div>

        <div class="card-body">
          <div v-if="embeddingLoading" class="status-loading">
            <el-icon class="is-loading"><Loading /></el-icon>
            <span>检测模型状态...</span>
          </div>

          <template v-else-if="embeddingStatus">
            <div class="model-info">
              <div class="model-name">
                {{ embeddingStatus.name }}
                <el-tag v-if="embeddingStatus.is_installed" type="success" size="small" effect="dark" round>
                  <el-icon><Check /></el-icon> 已安装
                </el-tag>
                <el-tag v-else type="warning" size="small" effect="dark" round>
                  未安装
                </el-tag>
              </div>
              <div class="model-desc">{{ embeddingStatus.description }}</div>
              <div class="model-meta">
                <span>大小: ~{{ embeddingStatus.size_mb }}MB</span>
                <span>语言: 中文</span>
              </div>
            </div>

            <div v-if="embeddingDownloading" class="download-progress">
              <div class="progress-info">
                <span>正在下载: {{ embeddingProgressFile }}</span>
                <span>{{ embeddingProgressBytes }}</span>
              </div>
              <el-progress :percentage="embeddingProgress" :stroke-width="8" color="#4facfe" />
            </div>
          </template>
        </div>

        <div class="card-footer">
          <template v-if="embeddingStatus?.is_installed">
            <span class="model-dir" :title="embeddingStatus?.model_dir">
              📂 {{ embeddingStatus?.model_dir }}
            </span>
          </template>
          <template v-else>
            <el-button type="primary" :loading="embeddingDownloading" @click="handleDownloadEmbedding">
              <el-icon v-if="!embeddingDownloading"><Download /></el-icon>
              {{ embeddingDownloading ? '下载中...' : '下载模型' }}
            </el-button>
          </template>
        </div>
      </div>

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

            <div v-if="asrDownloading" class="download-progress">
              <div class="progress-info">
                <span>正在下载: {{ asrProgressFile }}</span>
                <span>{{ asrProgressBytes }}</span>
              </div>
              <el-progress :percentage="asrProgress" :stroke-width="8" color="#4facfe" />
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
            <el-button type="primary" :loading="asrDownloading" @click="handleDownloadAsr">
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
            <div class="about-row"><span>搜索引擎</span><span>Tantivy (jieba 中文分词)</span></div>
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

.card-icon.ai { background: rgba(79, 172, 254, 0.12); }
.card-icon.asr { background: rgba(244, 63, 94, 0.12); }
.card-icon.embedding { background: rgba(167, 139, 250, 0.12); }
.card-icon.about { background: rgba(79, 172, 254, 0.12); }

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

.card-body { padding: 20px 24px; }

.card-footer {
  padding: 14px 24px;
  background: rgba(255,255,255,0.02);
  border-top: 1px solid rgba(255,255,255,0.04);
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 10px;
}

.form-group { margin-bottom: 16px; }

.form-group label {
  display: block;
  font-size: 0.82rem;
  color: #888;
  margin-bottom: 6px;
}

.lm-status { margin-top: 8px; }

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

.model-desc { font-size: 0.82rem; color: #888; }

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

.about-row span:first-child { color: #888; }
.about-row span:last-child { color: #ccc; }
</style>
