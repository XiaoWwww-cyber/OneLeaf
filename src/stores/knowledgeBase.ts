import { defineStore } from 'pinia';
import { ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';

export interface Document {
  id: string;
  name: string;
  category: string;
  content: string;
  source_path?: string;
  created_at: string;
}

export interface SearchResult {
  document: Document;
  relevance: number;
  snippet: string;
}

export const useKnowledgeBaseStore = defineStore('knowledgeBase', () => {
  const documents = ref<Document[]>([]);
  const searchResults = ref<SearchResult[]>([]);
  const isLoading = ref(false);

  async function init() {
    try {
      await invoke('init_knowledge_base', { dbPath: '' });
      await listDocuments();
    } catch (e) {
      console.error('Failed to init knowledge base:', e);
    }
  }

  async function listDocuments() {
    try {
      const docs = await invoke<Document[]>('list_documents');
      documents.value = docs;
    } catch (e) {
      console.error('Failed to list documents:', e);
    }
  }

  async function addDocument(filePath: string | null, content: string | null, category: string = 'default') {
    isLoading.value = true;
    try {
      await invoke('add_document_to_kb', { filePath, content, category });
      await listDocuments();
    } catch (e) {
      console.error('Failed to add document:', e);
      throw e;
    } finally {
      isLoading.value = false;
    }
  }

  async function deleteDocument(id: string) {
    try {
      await invoke('delete_document', { id });
      await listDocuments();
    } catch (e) {
      console.error('Failed to delete document:', e);
    }
  }

  async function search(query: string) {
    if (!query) return;
    try {
      const results = await invoke<SearchResult[]>('search_knowledge_base', { query, limit: 5 });
      searchResults.value = results;
    } catch (e) {
      console.error('Search failed:', e);
    }
  }

  return {
    documents,
    searchResults,
    isLoading,
    init,
    listDocuments,
    addDocument,
    deleteDocument,
    search
  };
});
