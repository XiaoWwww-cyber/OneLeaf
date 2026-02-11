<script setup lang="ts">
import { onMounted } from 'vue'
import { useKnowledgeBaseStore } from './stores/knowledgeBase'

const kbStore = useKnowledgeBaseStore()

onMounted(() => {
  kbStore.init()
})
</script>

<template>
  <router-view />
</template>

<style>
/* 全局样式 - 暗色主题 */
*,
*::before,
*::after {
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}

html, body, #app {
  width: 100%;
  height: 100%;
  overflow: hidden;
}

:root {
  font-family: 'Inter', 'Segoe UI', system-ui, -apple-system, sans-serif;
  font-size: 16px;
  line-height: 1.5;
  font-weight: 400;
  color: #e3e3e3;
  background-color: #1a1a2e;
  font-synthesis: none;
  text-rendering: optimizeLegibility;
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
}

/* Element Plus 暗色主题覆盖 */
.el-input__wrapper {
  background-color: #2c2c3e !important;
  box-shadow: none !important;
}

.el-input__inner {
  color: #e3e3e3 !important;
}

.el-button--primary {
  --el-button-bg-color: #4facfe;
  --el-button-border-color: #4facfe;
}

/* 滚动条样式 */
::-webkit-scrollbar {
  width: 6px;
}

::-webkit-scrollbar-track {
  background: transparent;
}

::-webkit-scrollbar-thumb {
  background-color: rgba(255, 255, 255, 0.15);
  border-radius: 3px;
}

::-webkit-scrollbar-thumb:hover {
  background-color: rgba(255, 255, 255, 0.25);
}
</style>