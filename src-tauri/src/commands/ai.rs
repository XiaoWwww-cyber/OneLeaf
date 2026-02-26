use crate::ai::knowledge_base::{Document, KnowledgeBase, SearchResult};
use crate::ai::service::{AiProviderType, AiService, AttachmentInfo, ChatMessage, StreamChunk};
use crate::utils::paths::get_kb_files_dir;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tracing::{error, info, warn};

// 全局知识库实例
static KNOWLEDGE_BASE: Lazy<Arc<Mutex<Option<KnowledgeBase>>>> =
    Lazy::new(|| Arc::new(Mutex::new(None)));

// 全局 AI 服务实例
pub static AI_SERVICE: Lazy<Arc<Mutex<AiService>>> =
    Lazy::new(|| Arc::new(Mutex::new(AiService::new())));

/// AI 设置结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiSettings {
    pub provider: String,
    pub doubao_api_key: Option<String>,
    pub doubao_model_id: Option<String>,
    pub openai_api_key: Option<String>,
    pub deepseek_api_key: Option<String>,
    pub lm_studio_url: String,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            provider: "lmstudio".to_string(),
            doubao_api_key: None,
            doubao_model_id: Some("doubao-seed-2-0-pro-260215".to_string()),
            openai_api_key: None,
            deepseek_api_key: None,
            lm_studio_url: "http://localhost:1234".to_string(),
        }
    }
}

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
    let model_file = model_dir.join("model.onnx");
    let model_path = if model_file.exists() {
        info!("找到向量模型文件: {:?}", model_file);
        Some(model_dir)
    } else {
        warn!("未找到向量模型文件: {:?}，将降级使用全文搜索", model_file);
        None
    };

    let kb = KnowledgeBase::with_model_dir(&path, model_path.as_deref()).map_err(|e| e.to_string())?;
    *KNOWLEDGE_BASE.lock() = Some(kb);
    info!("知识库初始化完成");
    Ok(())
}

#[tauri::command]
pub async fn add_document_to_kb(
    app: AppHandle,
    file_path: Option<String>,
    content: Option<String>,
    category: String,
) -> Result<Document, String> {
    let kb = KNOWLEDGE_BASE.lock().as_ref().cloned().ok_or("知识库未初始化")?;
    let kb_files_dir = get_kb_files_dir(&app);
    
    let path_buf = file_path.map(PathBuf::from);
    let doc = kb.add_document(
        path_buf.as_ref(), content, &category, Some(&kb_files_dir),
    ).await.map_err(|e| e.to_string())?;
    Ok(doc)
}

/// 搜索知识库 — 智能切换
/// 有 ONNX 嵌入模型时用向量语义搜索，否则用 Tantivy 全文搜索
#[tauri::command]
pub async fn search_knowledge_base(query: String, limit: usize) -> Result<Vec<SearchResult>, String> {
    let kb = KNOWLEDGE_BASE.lock().as_ref().cloned().ok_or("知识库未初始化")?;

    if kb.has_semantic_embedder() {
        info!("使用向量语义搜索: {}", query);
        kb.search(&query, limit).await.map_err(|e| e.to_string())
    } else {
        info!("使用 Tantivy 全文搜索: {}", query);
        kb.search_fulltext(&query, limit).await.map_err(|e| e.to_string())
    }
}

/// 上传文件到 AI（聊天直传，非知识库）
#[tauri::command]
pub async fn upload_file_to_ai(file_path: String) -> Result<String, String> {
    info!("上传文件到 AI: {}", file_path);
    let service = AI_SERVICE.lock().clone();
    let path = std::path::Path::new(&file_path);
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();

    // 可本地提取文本的格式，统一走本地策略（不管是什么 provider）
    // 云端 API（如豆包）不支持 txt/md/docx 上传
    let local_text_formats = ["txt", "md", "docx"];
    let is_local_model = service.provider_type == crate::ai::service::AiProviderType::LmStudio;

    if is_local_model || local_text_formats.contains(&ext.as_str()) {
        info!("使用本地文本提取策略 (ext={}, local_model={}): {}", ext, is_local_model, file_path);
        if let Err(e) = crate::ai::document_parser::extract_text_from_file(path) {
            return Err(format!("本地文件解析失败: {}", e));
        }
        return Ok(format!("LOCAL_FILE:{}", file_path));
    }

    // PDF/图片/视频等走云端上传
    service
        .upload_file(&file_path)
        .await
        .map_err(|e| format!("文件上传失败: {}", e))
}

/// AI 对话（RAG 模式 + 流式响应）
/// 1. 搜索知识库获取上下文
/// 2. 注入系统提示词
/// 3. 调用 AI 服务（豆包走流式 SSE，通过 Tauri Event 推送）
/// 4. AI 未配置时返回搜索结果摘要
#[tauri::command]
pub async fn chat_with_ai(
    app: AppHandle,
    messages: Vec<ChatMessage>,
    session_id: Option<String>,
    enable_web_search: Option<bool>,
    thinking_depth: Option<String>,
    attachments: Option<Vec<AttachmentInfo>>,
) -> Result<String, String> {
    let raw_attachments = attachments.unwrap_or_default();
    info!("AI 对话请求，消息数: {}，附件数: {}", messages.len(), raw_attachments.len());

    let sid = session_id.clone().unwrap_or_else(|| "default".to_string());
    let mut valid_attachments = Vec::new();
    let mut local_attachments_text = String::new();

    for att in raw_attachments {
        info!("处理附件: file_id={}, file_type={}, file_name={}", att.file_id, att.file_type, att.file_name);
        if att.file_id.starts_with("LOCAL_FILE:") {
            let file_path_str = att.file_id.trim_start_matches("LOCAL_FILE:");
            info!("本地附件路径: {}", file_path_str);
            let path = std::path::Path::new(file_path_str);
            match crate::ai::document_parser::extract_text_from_file(path) {
                Ok(text) => {
                    info!("✅ 成功解析本地附件 {}，文本长度: {} 字符", att.file_name, text.len());
                    local_attachments_text.push_str(&format!("【附件内容 - 文件名: {}】:\n{}\n\n", att.file_name, text));
                }
                Err(e) => {
                    warn!("❌ 无法解析本地附件 {}: {}", file_path_str, e);
                }
            }
        } else {
            valid_attachments.push(att);
        }
    }
    info!("附件处理完毕: local_text_len={}, valid_cloud_attachments={}", local_attachments_text.len(), valid_attachments.len());

    // 1. 获取最后一条用户消息用于 RAG 搜索
    let last_user_msg = messages.last().filter(|m| m.role == "user").map(|m| m.content.clone());

    // 2. RAG 搜索（知识库已初始化时）
    let kb_opt = KNOWLEDGE_BASE.lock().clone();

    let mut rag_context = if let Some(query) = &last_user_msg {
        if let Some(kb) = &kb_opt {
            let doc_count = kb.list_documents().await.map(|d| d.len()).unwrap_or(0);
            info!("执行 RAG 搜索 (当前知识库共 {} 个文档)，查询: {}", doc_count, query);
            
            let (results, search_type) = if kb.has_semantic_embedder() {
                (kb.search(query, 3).await.ok(), "向量语义搜索")
            } else {
                (kb.search_fulltext(query, 3).await.ok(), "全文搜索")
            };

            info!("使用 {} 模式进行检索", search_type);

            if let Some(results) = results {
                // 相似度阈值过滤：低于 0.6 的结果直接丢弃，避免"矮子里拔将军"
                let threshold = 0.50_f32;
                let filtered: Vec<_> = results
                    .into_iter()
                    .filter(|r| {
                        if r.relevance < threshold {
                            info!("过滤低相关度文档: {} (相关度: {} < {})", r.document.name, r.relevance, threshold);
                            false
                        } else {
                            true
                        }
                    })
                    .collect();

                info!("RAG 搜索完成，阈值过滤后有效结果: {} 条", filtered.len());
                if !filtered.is_empty() {
                    let ctx = filtered
                        .iter()
                        .map(|r| {
                            info!("有效匹配: {} (相关度: {})", r.document.name, r.relevance);
                            format!("【{}】{}", r.document.name, r.document.content)
                        })
                        .collect::<Vec<_>>()
                        .join("\n\n");
                    Some((ctx, filtered))
                } else {
                    info!("所有结果相关度均低于阈值 {}，退化为普通对话模式", threshold);
                    None
                }
            } else {
                warn!("RAG 搜索返回 None 或出错");
                None
            }
        } else {
            warn!("知识库尚未初始化，跳过 RAG");
            None
        }
    } else {
        None
    };

    // --- 长时记忆（RAG 搜索历史聊天记录） ---
    let mut history_context = None;
    if let Some(query) = &last_user_msg {
        if let Some(kb) = &kb_opt {
            if kb.has_semantic_embedder() {
                info!("执行长时记忆 RAG 搜索，查询: {}，会话隔离: {:?}", query, session_id);
                if let Ok(sim_msgs) = kb.search_chat_history(query, session_id.as_deref(), 3).await {
                    if !sim_msgs.is_empty() {
                        let ctx = sim_msgs
                            .iter()
                            .map(|(_msg_id, _session_id, content)| {
                                format!("【历史参考对话片段】：{}", content)
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        history_context = Some(ctx);
                    }
                }
            }
        }
    }

    // 3. 检查 AI 是否已配置
    let service = AI_SERVICE.lock().clone();

    let mut do_local_web_search = enable_web_search.unwrap_or(false);
    if do_local_web_search && service.provider_type != AiProviderType::LmStudio {
        info!("当前使用的是云端模型（{:?}），跳过本地的 DuckDuckGo 搜索，使用原生能力", service.provider_type);
        do_local_web_search = false;
    }

    if do_local_web_search {
        if let Some(query) = &last_user_msg {
            info!("执行联网搜索: {}", query);
            match search_web_ddg(query).await {
                Ok(web_results) => {
                    if !web_results.is_empty() {
                        let web_ctx = format!("【联网实时搜索资料】\n{}", web_results);
                        match &mut rag_context {
                            Some((ctx, _)) => {
                                ctx.push_str("\n\n====\n");
                                ctx.push_str(&web_ctx);
                            }
                            None => {
                                rag_context = Some((web_ctx, vec![]));
                            }
                        }
                        info!("完成联网搜索，已注入上下文");
                    } else {
                        info!("联网搜索未返回有效结果");
                    }
                }
                Err(e) => {
                    warn!("联网搜索失败: {}", e);
                }
            }
        }
    }
    
    if !service.is_configured() {
        info!("AI 未配置，返回知识库搜索结果");
        return if let Some((ctx, _results)) = &rag_context {
            Ok(format!("📚 知识库搜索结果：\n\n{}\n\n💡 提示：请在设置中配置 AI 服务以获得智能问答体验。", ctx))
        } else {
            Ok("未找到相关知识库内容。请先添加文档到知识库，或在设置中配置 AI 服务。".to_string())
        };
    }

    // 4. 构建系统提示词与上下文
    let mut final_messages = messages.clone();

    // 有本地附件时，使用极简注入策略（复刻 LM Studio inject-full-content）
    // 不加冗余系统提示词，直接把文件原文拼在用户消息前面
    if !local_attachments_text.is_empty() {
        info!("使用本地附件注入策略 (inject-full-content)，附件文本长度: {} 字符", local_attachments_text.len());
        if let Some(last_msg) = final_messages.last_mut().filter(|m| m.role == "user") {
            let original_content = last_msg.content.clone();
            last_msg.content = format!(
                "{}\n\n{}", 
                local_attachments_text.trim(), original_content
            );
            info!("已将附件内容注入用户消息，最终长度: {} 字符", last_msg.content.len());
        }
    } else {
        // 无本地附件：使用标准 RAG 系统提示词
        let mut context_prompt = String::from(
            "你是 OneLeaf 智能知识库助手。\n\
             请仔细阅读以下【参考知识】，并以此来回答用户的问题。\n\
             如果【参考知识】中没有与用户问题相关的内容，请忽略这些参考知识，\n\
             直接使用你自己的基础知识来回答，并向用户说明你是基于通用知识回答的。\n\
             切勿牵强附会地将不相关的参考知识硬塞进回答中。\n\n",
        );

        if let Some((ref ctx, _)) = rag_context {
            context_prompt.push_str("【参考知识】：\n");
            context_prompt.push_str(ctx);
            context_prompt.push_str("\n\n");
        } 

        if let Some(ref hist_ctx) = history_context {
            context_prompt.push_str("【历史跨会话聊天记忆】：\n");
            context_prompt.push_str(hist_ctx);
            context_prompt.push_str("\n\n");
        }

        if rag_context.is_some() || history_context.is_some() {
            context_prompt.push_str("(请根据以上内容回答。若参考内容与问题无关或不充分，请结合自身知识回答。)\n\n");
        } else {
            if let Some(kb) = &kb_opt {
                 if let Ok(docs) = kb.list_documents().await {
                     if !docs.is_empty() {
                         context_prompt.push_str("\n【提示】：系统中存在以下文档，但由于未加载深度语义模型，全文搜索未能直接定位到匹配的文字片段。请基于常识回答，并告知用户参考这些文档名：\n");
                         for doc in docs.iter().take(5) {
                             context_prompt.push_str(&format!("- {}\n", doc.name));
                         }
                     }
                 }
            }
        }

        info!("最终 context_prompt 长度: {} 字符", context_prompt.len());
        if let Some(last_msg) = final_messages.last_mut().filter(|m| m.role == "user") {
            let original_content = last_msg.content.clone();
            last_msg.content = format!(
                "{}\n\n=== 用户提问 ===\n{}", 
                context_prompt, original_content
            );
            info!("已将 context_prompt 注入最后一条用户消息，最终长度: {} 字符", last_msg.content.len());
        } else {
            final_messages.insert(
                0,
                ChatMessage {
                    role: "system".to_string(),
                    content: context_prompt,
                },
            );
        }
    }

    // 5. 调用 AI 服务（豆包走流式，其他走非流式）
    info!("调用 AI 服务...");
    let req_enable_web_search = enable_web_search.unwrap_or(false);

    if service.provider_type == AiProviderType::Doubao {
        // 豆包：流式 SSE + Tauri Event 推送
        let app_clone = app.clone();
        match service
            .chat_stream(
                final_messages,
                valid_attachments,
                req_enable_web_search,
                thinking_depth,
                sid.clone(),
                move |chunk: StreamChunk| {
                    let _ = app_clone.emit("ai-stream-chunk", &chunk);
                },
            )
            .await
        {
            Ok(full_response) => {
                info!("AI 流式对话完成");
                Ok(full_response)
            }
            Err(e) => {
                // 出错时也通知前端结束
                let _ = app.emit(
                    "ai-stream-chunk",
                    &StreamChunk {
                        session_id: sid,
                        chunk_type: "text".to_string(),
                        delta: String::new(),
                        done: true,
                    },
                );
                error!("AI 流式对话失败: {}", e);
                Err(format!("AI 对话失败: {}", e))
            }
        }
    } else {
        // 其他 Provider：非流式
        match service
            .chat(final_messages, req_enable_web_search, thinking_depth)
            .await
        {
            Ok(response) => {
                info!("AI 对话成功");
                Ok(response)
            }
            Err(e) => {
                error!("AI 对话失败: {}", e);
                Err(format!("AI 对话失败: {}", e))
            }
        }
    }
}

// ==========================================
// 聊天记录管理
// ==========================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMeta {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessageRecord {
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub created_at: String,
}

#[tauri::command]
pub async fn save_conversation(id: String, title: String) -> Result<(), String> {
    let kb = KNOWLEDGE_BASE.lock().as_ref().cloned().ok_or("知识库未初始化")?;
    kb.save_conversation(&id, &title).await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn load_conversations() -> Result<Vec<ConversationMeta>, String> {
    let kb = KNOWLEDGE_BASE.lock().as_ref().cloned().ok_or("知识库未初始化")?;
    let records = kb.load_conversations().await.map_err(|e| e.to_string())?;
    let convs = records.into_iter().map(|(id, title, created_at, updated_at)| ConversationMeta {
        id, title, created_at, updated_at
    }).collect();
    Ok(convs)
}

#[tauri::command]
pub async fn delete_conversation_record(session_id: String) -> Result<(), String> {
    let kb = KNOWLEDGE_BASE.lock().as_ref().cloned().ok_or("知识库未初始化")?;
    kb.delete_conversation(&session_id).await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn save_message(id: String, session_id: String, role: String, content: String) -> Result<(), String> {
    let kb = KNOWLEDGE_BASE.lock().as_ref().cloned().ok_or("知识库未初始化")?;
    kb.save_message(&id, &session_id, &role, &content).await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn load_messages(session_id: String) -> Result<Vec<ChatMessageRecord>, String> {
    let kb = KNOWLEDGE_BASE.lock().as_ref().cloned().ok_or("知识库未初始化")?;
    let records = kb.load_messages(&session_id).await.map_err(|e| e.to_string())?;
    let msgs = records.into_iter().map(|(id, session_id, role, content, created_at)| ChatMessageRecord {
        id, session_id, role, content, created_at
    }).collect();
    Ok(msgs)
}

/// 更新 AI 设置
#[tauri::command]
pub async fn update_ai_settings(app: AppHandle, settings: AiSettings) -> Result<(), String> {
    let mut service = AI_SERVICE.lock();

    let provider_type = match settings.provider.as_str() {
        "doubao" => AiProviderType::Doubao,
        "openai" => AiProviderType::OpenAi,
        "deepseek" => AiProviderType::DeepSeek,
        _ => AiProviderType::LmStudio,
    };
    service.set_provider(provider_type);

    if let Some(key) = &settings.doubao_api_key {
        if !key.is_empty() {
            service.set_doubao_key(key.clone());
        }
    }
    if let Some(model_id) = &settings.doubao_model_id {
        if !model_id.is_empty() {
            service.set_doubao_model(model_id.clone());
        }
    }
    if let Some(key) = &settings.openai_api_key {
        if !key.is_empty() {
            service.set_openai_key(key.clone());
        }
    }
    if let Some(key) = &settings.deepseek_api_key {
        if !key.is_empty() {
            service.set_deepseek_key(key.clone());
        }
    }

    if !settings.lm_studio_url.is_empty() {
        service.set_lm_studio_url(settings.lm_studio_url.clone());
    }

    save_ai_settings_to_disk(&app, &settings);

    // 通知前端 provider 已改变
    let _ = app.emit("ai-provider-changed", &settings.provider);

    info!("AI 设置已更新并持久化，提供者: {:?}", provider_type);
    Ok(())
}

/// 将设置持久化到本地文件
fn save_ai_settings_to_disk(app: &tauri::AppHandle, settings: &AiSettings) {
    if let Ok(app_dir) = app.path().app_data_dir() {
        let settings_path = app_dir.join("ai_settings.json");
        if let Ok(json) = serde_json::to_string_pretty(settings) {
            let _ = std::fs::write(settings_path, json);
        }
    }
}

/// 启动时从配置文件中读取 AI 设置并初始化
pub fn init_ai_settings(app: &tauri::AppHandle) {
    if let Ok(app_dir) = app.path().app_data_dir() {
        let settings_path = app_dir.join("ai_settings.json");
        if settings_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&settings_path) {
                if let Ok(settings) = serde_json::from_str::<AiSettings>(&content) {
                    let mut service = AI_SERVICE.lock();
                    let provider_type = match settings.provider.as_str() {
                        "doubao" => AiProviderType::Doubao,
                        "openai" => AiProviderType::OpenAi,
                        "deepseek" => AiProviderType::DeepSeek,
                        _ => AiProviderType::LmStudio,
                    };
                    service.set_provider(provider_type);

                    if let Some(key) = &settings.doubao_api_key {
                        if !key.is_empty() { service.set_doubao_key(key.clone()); }
                    }
                    if let Some(model_id) = &settings.doubao_model_id {
                        if !model_id.is_empty() { service.set_doubao_model(model_id.clone()); }
                    }
                    if let Some(key) = &settings.openai_api_key {
                        if !key.is_empty() { service.set_openai_key(key.clone()); }
                    }
                    if let Some(key) = &settings.deepseek_api_key {
                        if !key.is_empty() { service.set_deepseek_key(key.clone()); }
                    }
                    if !settings.lm_studio_url.is_empty() {
                        service.set_lm_studio_url(settings.lm_studio_url);
                    }
                    info!("已从本地配置文件加载 AI 设置");
                }
            }
        }
    }
}

/// 获取 AI 设置
#[tauri::command]
pub async fn get_ai_settings() -> Result<AiSettings, String> {
    let service = AI_SERVICE.lock();

    let provider = match service.provider_type {
        AiProviderType::Doubao => "doubao",
        AiProviderType::OpenAi => "openai",
        AiProviderType::DeepSeek => "deepseek",
        AiProviderType::LmStudio => "lmstudio",
    };

    Ok(AiSettings {
        provider: provider.to_string(),
        doubao_api_key: service.doubao_api_key.clone(),
        doubao_model_id: service.doubao_model.clone(),
        openai_api_key: service.openai_api_key.clone(),
        deepseek_api_key: service.deepseek_api_key.clone(),
        lm_studio_url: service.lm_studio_url.clone(),
    })
}

/// 检查 LM Studio 是否运行
#[tauri::command]
pub async fn check_lm_studio() -> Result<bool, String> {
    let service = AI_SERVICE.lock().clone();
    Ok(service.check_lm_studio().await)
}

/// 清除所有缓存（知识库 + 聊天记录 + 备份文件）
#[tauri::command]
pub async fn clear_all_cache(app: AppHandle) -> Result<(), String> {
    info!("开始清除所有缓存...");

    // 1. 清除数据库数据（知识库 + 聊天记录）
    let kb = KNOWLEDGE_BASE.lock().as_ref().cloned();
    if let Some(kb) = kb {
        kb.clear_all_with_history().await.map_err(|e| format!("清除数据库失败: {}", e))?;
        info!("数据库清除完成");
    }

    // 2. 清除 kb-files 备份目录
    let kb_files_dir = get_kb_files_dir(&app);
    if kb_files_dir.exists() {
        if let Err(e) = std::fs::remove_dir_all(&kb_files_dir) {
            warn!("清除备份文件目录失败: {}", e);
        } else {
            info!("备份文件目录清除完成: {:?}", kb_files_dir);
        }
    }

    info!("所有缓存清除完成");
    let _ = app.emit("cache-cleared", ());
    Ok(())
}

#[tauri::command]
pub async fn list_documents() -> Result<Vec<Document>, String> {
    let kb = KNOWLEDGE_BASE.lock().as_ref().cloned().ok_or("知识库未初始化")?;
    kb.list_documents().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_document(_app: AppHandle, id: String) -> Result<(), String> {
    let kb = KNOWLEDGE_BASE.lock().as_ref().cloned().ok_or("知识库未初始化")?;
    
    // 获取文档信息（用于删除备份文件）
    let docs = kb.list_documents().await.map_err(|e| e.to_string())?;
    if let Some(doc) = docs.iter().find(|d| d.id == id) {
        if let Some(bp) = &doc.backup_path {
            let _ = std::fs::remove_file(bp);
        }
    }
    
    kb.delete_document(&id).await.map_err(|e| e.to_string())
}

/// 获取文档内容（用于预览）
#[tauri::command]
pub async fn get_document_content(id: String) -> Result<Document, String> {
    let kb = KNOWLEDGE_BASE.lock().as_ref().cloned().ok_or("知识库未初始化")?;
    let docs = kb.list_documents().await.map_err(|e| e.to_string())?;
    docs.into_iter().find(|d| d.id == id).ok_or("文档不存在".to_string())
}

/// 用系统默认程序打开文档原件
#[tauri::command]
pub async fn open_document_file(id: String) -> Result<(), String> {
    let kb = KNOWLEDGE_BASE.lock().as_ref().cloned().ok_or("知识库未初始化")?;
    let docs = kb.list_documents().await.map_err(|e| e.to_string())?;
    let doc = docs.into_iter().find(|d| d.id == id).ok_or("文档不存在".to_string())?;
    
    let file_path = doc.backup_path.or(doc.source_path).ok_or("未找到文档文件".to_string())?;
    
    let path = PathBuf::from(&file_path);
    if !path.exists() {
        return Err(format!("文件不存在: {}", file_path));
    }
    
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &file_path])
            .spawn()
            .map_err(|e| format!("打开文件失败: {}", e))?;
    }
    
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&file_path)
            .spawn()
            .map_err(|e| format!("打开文件失败: {}", e))?;
    }
    
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&file_path)
            .spawn()
            .map_err(|e| format!("打开文件失败: {}", e))?;
    }
    
    Ok(())
}

// ========== 嵌入模型管理 ==========

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingModelInfo {
    pub name: String,
    pub description: String,
    pub size_mb: u64,
    pub is_installed: bool,
    pub model_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingModelProgress {
    pub file_name: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub progress: f32,
    pub status: String,
}

fn get_embedding_models_dir(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("models")
}

#[tauri::command]
pub async fn get_embedding_model_status(app: AppHandle) -> Result<EmbeddingModelInfo, String> {
    let models_dir = get_embedding_models_dir(&app);
    let model_dir = models_dir.join("bge-small-zh");

    // rust_tokenizers 依赖 vocab.txt
    let is_installed = model_dir.join("model.onnx").exists() && model_dir.join("vocab.txt").exists();

    Ok(EmbeddingModelInfo {
        name: "BGE-small-zh-v1.5".to_string(),
        description: "智源中文语义嵌入模型 (用于知识库 RAG 向量搜索)".to_string(),
        size_mb: 95,
        is_installed,
        model_dir: model_dir.to_string_lossy().to_string(),
    })
}

#[tauri::command]
pub async fn download_embedding_model(app: AppHandle) -> Result<(), String> {
    use tauri::Emitter; // 引入 Emitter 用于触发事件
    
    let models_dir = get_embedding_models_dir(&app);
    let model_dir = models_dir.join("bge-small-zh");
    std::fs::create_dir_all(&model_dir).map_err(|e| format!("创建目录失败: {}", e))?;

    let base_url = "https://hf-mirror.com/Xenova/bge-small-zh-v1.5/resolve/main/onnx";
    let root_url = "https://hf-mirror.com/Xenova/bge-small-zh-v1.5/resolve/main";

    let files = vec![
        ("model.onnx", format!("{}/model.onnx", base_url)),
        ("vocab.txt", format!("{}/vocab.txt", root_url)),
    ];

    let client = reqwest::Client::new();

    for (file_name, url) in files {
        let dest_path = model_dir.join(file_name);
        
        if dest_path.exists() {
            let _ = app.emit("embedding-model-progress", EmbeddingModelProgress {
                file_name: file_name.to_string(),
                downloaded_bytes: 0,
                total_bytes: 0,
                progress: 1.0,
                status: "completed".to_string(),
            });
            continue;
        }

        let _ = app.emit("embedding-model-progress", EmbeddingModelProgress {
            file_name: file_name.to_string(),
            downloaded_bytes: 0,
            total_bytes: 0,
            progress: 0.0,
            status: "downloading".to_string(),
        });

        let response = client.get(&url).send().await.map_err(|e| format!("请求失败: {}", e))?;

        if !response.status().is_success() {
            let _ = app.emit("embedding-model-progress", EmbeddingModelProgress {
                file_name: file_name.to_string(),
                downloaded_bytes: 0,
                total_bytes: 0,
                progress: 0.0,
                status: "failed".to_string(),
            });
            return Err(format!("下载 {} 失败: HTTP {}", file_name, response.status()));
        }

        let total_size = response.content_length().unwrap_or(0);
        let mut downloaded: u64 = 0;

        let mut file = tokio::fs::File::create(&dest_path).await.map_err(|e| format!("创建文件失败: {}", e))?;

        use futures_util::StreamExt;
        use tokio::io::AsyncWriteExt;

        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("下载错误: {}", e))?;
            file.write_all(&chunk).await.map_err(|e| format!("写入失败: {}", e))?;

            downloaded += chunk.len() as u64;
            let progress = if total_size > 0 { downloaded as f32 / total_size as f32 } else { 0.0 };

            if downloaded % (100 * 1024) < chunk.len() as u64 || downloaded == total_size {
                let _ = app.emit("embedding-model-progress", EmbeddingModelProgress {
                    file_name: file_name.to_string(),
                    downloaded_bytes: downloaded,
                    total_bytes: total_size,
                    progress,
                    status: "downloading".to_string(),
                });
            }
        }

        let _ = app.emit("embedding-model-progress", EmbeddingModelProgress {
            file_name: file_name.to_string(),
            downloaded_bytes: total_size,
            total_bytes: total_size,
            progress: 1.0,
            status: "completed".to_string(),
        });
    }

    // 下载完成后重新初始化知识库使模型生效
    let _ = init_knowledge_base(app, String::new()).await;

    Ok(())
}

async fn search_web_ddg(query: &str) -> Result<String, String> {
    // 优先用 Bing 国内版（cn.bing.com），DuckDuckGo 在国内被墙
    match search_web_bing(query).await {
        Ok(r) if !r.is_empty() => Ok(r),
        _ => {
            info!("Bing 搜索失败或无结果，尝试 DuckDuckGo fallback");
            search_web_ddg_fallback(query).await
        }
    }
}

async fn search_web_bing(query: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let url = format!("https://cn.bing.com/search?q={}&count=5", urlencoding::encode(query));
    let resp = client.get(&url)
        .header("Accept-Language", "zh-CN,zh;q=0.9")
        .send().await.map_err(|e| format!("Bing 请求失败: {}", e))?;

    let html = resp.text().await.map_err(|e| e.to_string())?;

    // Bing 搜索摘要在 <div class="b_caption"><p>...</p></div> 结构中
    // 或者在 <p class="...b_algoSlug...">...</p> 中
    let re_caption = regex::Regex::new(r#"(?s)<div\s+class="b_caption"[^>]*>\s*<p[^>]*>(.*?)</p>"#).unwrap();
    let re_algo = regex::Regex::new(r#"<p[^>]*class="[^"]*b_algoSlug[^"]*"[^>]*>([\s\S]*?)</p>"#).unwrap();
    let re_tags = regex::Regex::new(r#"<[^>]+>"#).unwrap();

    let mut results = Vec::new();

    // 优先用 b_caption 匹配
    for cap in re_caption.captures_iter(&html) {
        let text = cap[1].to_string();
        let clean = re_tags.replace_all(&text, " ");
        let final_text = clean_html_entities(&clean);
        let trimmed = final_text.trim().split_whitespace().collect::<Vec<_>>().join(" ");
        if !trimmed.is_empty() && trimmed.len() > 15 {
            results.push(format!("- {}", trimmed));
        }
        if results.len() >= 5 { break; }
    }

    // 如果 b_caption 没匹配到，用 b_algoSlug
    if results.is_empty() {
        for cap in re_algo.captures_iter(&html) {
            let text = cap[1].to_string();
            let clean = re_tags.replace_all(&text, " ");
            let final_text = clean_html_entities(&clean);
            let trimmed = final_text.trim().split_whitespace().collect::<Vec<_>>().join(" ");
            if !trimmed.is_empty() && trimmed.len() > 15 {
                results.push(format!("- {}", trimmed));
            }
            if results.len() >= 5 { break; }
        }
    }

    let joined = results.join("\n");
    info!("Bing 搜索完成，提取到 {} 条摘要:\n{}", results.len(), joined);
    Ok(joined)
}

async fn search_web_ddg_fallback(query: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client.post("https://lite.duckduckgo.com/lite/")
        .form(&[("q", query), ("kl", "wt-wt")])
        .send().await.map_err(|e| e.to_string())?;

    let html = resp.text().await.map_err(|e| e.to_string())?;
    
    let re = regex::Regex::new(r#"<td class='result-snippet'>([\s\S]*?)</td>"#).unwrap();
    let re_tags = regex::Regex::new(r#"<[^>]+>"#).unwrap();

    let mut results = Vec::new();
    for cap in re.captures_iter(&html).take(5) { 
        let text = cap[1].to_string();
        let clean_text = re_tags.replace_all(&text, " ");
        let final_text = clean_text
            .replace("&nbsp;", " ")
            .replace("&amp;", "&")
            .replace("&quot;", "\"")
            .replace("&#x27;", "'")
            .replace("&#39;", "'")
            .replace("&lt;", "<")
            .replace("&gt;", ">");
        
        let trimmed = final_text.trim().split_whitespace().collect::<Vec<_>>().join(" ");
        if !trimmed.is_empty() {
            results.push(format!("- {}", trimmed));
        }
    }

    Ok(results.join("\n"))
}

fn clean_html_entities(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&ensp;", " ")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&#0183;", "·")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}
