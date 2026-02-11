use crate::ai::knowledge_base::{Document, KnowledgeBase, SearchResult};
use crate::ai::service::{AiService, ChatMessage};
// use crate::utils::paths::get_app_paths; // 需要实现 utils::paths
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tracing::info;

// 全局知识库实例
static KNOWLEDGE_BASE: Lazy<Arc<Mutex<Option<KnowledgeBase>>>> =
    Lazy::new(|| Arc::new(Mutex::new(None)));

// 全局 AI 服务实例
static AI_SERVICE: Lazy<Arc<Mutex<AiService>>> =
    Lazy::new(|| Arc::new(Mutex::new(AiService::new())));

#[tauri::command]
pub async fn init_knowledge_base(app: AppHandle, db_path: String) -> Result<(), String> {
    info!("初始化知识库...");
    let path = if db_path.is_empty() {
        app.path().app_data_dir().unwrap().join("knowledge_base.db")
    } else {
        PathBuf::from(db_path)
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    // 假设模型目录在 app_data/models/bge-small-zh
    let model_dir = app.path().app_data_dir().unwrap().join("models").join("bge-small-zh");
    let model_path = if model_dir.join("model.onnx").exists() {
        Some(model_dir)
    } else {
        None
    };

    let kb = KnowledgeBase::with_model_dir(&path, model_path.as_deref()).map_err(|e| e.to_string())?;
    *KNOWLEDGE_BASE.lock() = Some(kb);
    info!("知识库初始化完成");
    Ok(())
}

#[tauri::command]
pub async fn add_document_to_kb(file_path: Option<String>, content: Option<String>, category: String) -> Result<Document, String> {
    let kb = KNOWLEDGE_BASE.lock().as_ref().cloned().ok_or("知识库未初始化")?;
    
    let path_buf = file_path.map(PathBuf::from);
    let doc = kb.add_document(path_buf.as_ref(), content, &category).await.map_err(|e| e.to_string())?;
    Ok(doc)
}

#[tauri::command]
pub async fn search_knowledge_base(query: String, limit: usize) -> Result<Vec<SearchResult>, String> {
    let kb = KNOWLEDGE_BASE.lock().as_ref().cloned().ok_or("知识库未初始化")?;
    kb.search(&query, limit).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn chat_with_ai(messages: Vec<ChatMessage>) -> Result<String, String> {
    // 1. RAG 检索
    let last_user_msg = messages.last().filter(|m| m.role == "user").map(|m| m.content.clone());
    
    // Acquire KB clone and drop lock immediately
    let kb_opt = KNOWLEDGE_BASE.lock().clone();

    let context = if let Some(query) = last_user_msg {
        if let Some(kb) = kb_opt {
            if let Ok(results) = kb.search(&query, 3).await {
                if !results.is_empty() {
                    let ctx = results.iter().map(|r| format!("参考资料 [{}]: {}", r.document.name, r.snippet)).collect::<Vec<_>>().join("\n\n");
                    Some(ctx)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    let mut final_messages = messages.clone();
    if let Some(ctx) = context {
        let system_prompt = format!("你是一个智能助手。请参考以下知识库内容回答用户问题:\n\n{}", ctx);
        final_messages.insert(0, ChatMessage { role: "system".to_string(), content: system_prompt });
    }

    // Acquire Service clone and drop lock immediately
    let service = AI_SERVICE.lock().clone();
    service.chat(final_messages).await
}

#[tauri::command]
pub async fn list_documents() -> Result<Vec<Document>, String> {
    let kb = KNOWLEDGE_BASE.lock().as_ref().cloned().ok_or("知识库未初始化")?;
    kb.list_documents().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_document(id: String) -> Result<(), String> {
    let kb = KNOWLEDGE_BASE.lock().as_ref().cloned().ok_or("知识库未初始化")?;
    kb.delete_document(&id).await.map_err(|e| e.to_string())
}
