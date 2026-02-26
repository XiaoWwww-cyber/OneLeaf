// 知识库管理

use crate::ai::onnx_embedder::{EmbedderError, OnnxEmbedder};
use crate::ai::tantivy_search::{SearchError, TantivyIndex};
use crate::ai::vector_db::{VectorDb, VectorDbError};
use chrono::Utc;
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;
use uuid::Uuid;
use crate::ai::document_parser::extract_text_from_file;

#[derive(Error, Debug)]
pub enum KbError {
    #[error("文档不存在: {0}")]
    DocumentNotFound(String),
    #[error("文档解析失败: {0}")]
    ParseFailed(String),
    #[error("数据库错误: {0}")]
    DatabaseError(String),
    #[error("向量化失败: {0}")]
    EmbeddingFailed(String),
    #[error("嵌入模型未安装")]
    EmbedderNotInstalled,
    #[error("IO 错误: {0}")]
    IoError(#[from] std::io::Error),
    #[error("向量数据库错误: {0}")]
    VectorDbError(#[from] VectorDbError),
    #[error("ONNX 嵌入错误: {0}")]
    OnnxEmbedderError(#[from] EmbedderError),
    #[error("全文搜索错误: {0}")]
    TantivyError(#[from] SearchError),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Document {
    pub id: String,
    pub name: String,
    pub category: String,
    pub content: String,
    pub source_path: Option<String>,
    /// 知识库目录中的备份文件路径
    pub backup_path: Option<String>,
    /// 文件类型 (txt, md, docx, pdf, mp4, etc.)
    pub file_type: String,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SearchResult {
    pub document: Document,
    pub relevance: f32,
    pub snippet: String,
}

/// 嵌入器类型（支持回退）
pub enum Embedder {
    /// ONNX 语义嵌入（推荐）- 使用 Mutex 支持内部可变性
    Onnx(Mutex<OnnxEmbedder>),
    /// 简单嵌入（回退）
    Simple(SimpleEmbedder),
}

impl Embedder {
    /// 生成文本嵌入向量
    pub fn embed(&self, text: &str) -> Result<Vec<f32>, KbError> {
        match self {
            Embedder::Onnx(e) => e.lock().embed(text).map_err(KbError::from),
            Embedder::Simple(e) => Ok(e.embed(text)),
        }
    }

    /// 获取嵌入向量维度
    pub fn dimension(&self) -> usize {
        match self {
            Embedder::Onnx(e) => e.lock().dimension(),
            Embedder::Simple(e) => e.dimension,
        }
    }

    /// 是否使用语义嵌入
    pub fn is_semantic(&self) -> bool {
        matches!(self, Embedder::Onnx(_))
    }
}

pub struct KnowledgeBase {
    vector_db: Arc<VectorDb>,
    embedder: Arc<Embedder>,
    documents: Arc<parking_lot::RwLock<Vec<Document>>>,
    tantivy_index: Arc<TantivyIndex>,
}

impl Clone for KnowledgeBase {
    fn clone(&self) -> Self {
        Self {
            vector_db: Arc::clone(&self.vector_db),
            embedder: Arc::clone(&self.embedder),
            documents: Arc::clone(&self.documents),
            tantivy_index: Arc::clone(&self.tantivy_index),
        }
    }
}

impl KnowledgeBase {
    /// 创建新的知识库实例
    pub fn new(db_path: &Path) -> Result<Self, KbError> {
        Self::with_model_dir(db_path, None)
    }

    /// 创建知识库实例并指定模型目录
    pub fn with_model_dir(db_path: &Path, model_dir: Option<&Path>) -> Result<Self, KbError> {
        let vector_db = Arc::new(VectorDb::new(db_path)?);
        let documents = Arc::new(parking_lot::RwLock::new(Vec::new()));

        // 初始化 Tantivy 全文索引（存储在 db_path 同级目录下的 tantivy_index/）
        let tantivy_dir = db_path
            .parent()
            .unwrap_or(Path::new("."))
            .join("tantivy_index");
        let tantivy_index = Arc::new(TantivyIndex::new(&tantivy_dir)?);
        tracing::info!("Tantivy 全文索引已初始化: {:?}", tantivy_dir);

        // 尝试加载 ONNX 嵌入器，失败则回退到 SimpleEmbedder
        let embedder = if let Some(dir) = model_dir {
            match OnnxEmbedder::new(dir) {
                Ok(onnx) => {
                    tracing::info!("✅ 成功加载 ONNX 语义嵌入模型: {:?}", dir);
                    Arc::new(Embedder::Onnx(Mutex::new(onnx)))
                }
                Err(e) => {
                    tracing::warn!("❌ ONNX 模型加载失败 (可能是 DLL 缺失或架构不匹配)，回退到 SimpleEmbedder: {}", e);
                    Arc::new(Embedder::Simple(SimpleEmbedder::new(384)))
                }
            }
        } else {
            tracing::info!("ℹ️ 未找到模型目录，使用 SimpleEmbedder (搜索效果会受限)");
            Arc::new(Embedder::Simple(SimpleEmbedder::new(384)))
        };

        // 检查是否有维度不匹配需要重建向量
        let current_dim = embedder.dimension();
        let mut needs_reembed = false;
        if let Ok(db_dim) = vector_db.get_dimension() {
            if db_dim > 0 && db_dim != current_dim {
                tracing::warn!("检测到向量维度不匹配 (DB: {}, 当前: {})，将重新生成所有向量", db_dim, current_dim);
                needs_reembed = true;
            }
        }

        let mut docs_to_reembed = Vec::new();

        match vector_db.load_documents() {
            Ok(saved_docs) => {
                let mut docs = documents.write();
                let count = saved_docs.len();
                for (id, name, category, content, source_path, backup_path, file_type, created_at) in saved_docs {
                    let doc = Document {
                        id: id.clone(),
                        name,
                        category,
                        content: content.clone(),
                        source_path,
                        backup_path,
                        file_type,
                        created_at,
                    };
                    
                    if needs_reembed {
                        docs_to_reembed.push((id, content));
                    }
                    
                    // 确保都在全文索引中（启动时轻量同步）
                    let _ = tantivy_index.add_document(&doc.id, &doc.name, &doc.content);
                    
                    docs.push(doc);
                }
                tracing::info!("📚 成功从数据库加载并同步 {} 个文档", count);
            }
            Err(e) => {
                tracing::error!("❌ 无法加载数据库文档: {}", e);
            }
        }

        // 如果需要重建向量
        if needs_reembed {
            for (id, content) in docs_to_reembed {
                if let Ok(embedding) = embedder.embed(&content) {
                    let _ = vector_db.insert(&id, &embedding);
                    tracing::info!("♻️ 已为文档 {} 重新生成维度为 {} 的向量", id, current_dim);
                }
            }
        }

        Ok(Self {
            vector_db,
            embedder,
            documents,
            tantivy_index,
        })
    }

    /// 添加文档到知识库
    /// 
    /// - `path`: 源文件路径（可选）
    /// - `content`: 直接提供的文本内容（可选，如视频转写文本）
    /// - `category`: 分类（documents, video-transcript 等）
    /// - `backup_dir`: 知识库备份目录（可选），用于备份原始文件
    pub async fn add_document(
        &self, path: Option<&PathBuf>, content: Option<String>,
        category: &str, backup_dir: Option<&PathBuf>,
    ) -> Result<Document, KbError> {
        let doc_id = Uuid::new_v4().to_string();

        // 确定文档内容、名称、源路径、文件类型
        let (final_content, name, source_path, file_type) = if let Some(p) = path {
            if !p.exists() && content.is_none() {
                return Err(KbError::DocumentNotFound(p.display().to_string()));
            }
            let parsed_content = if let Some(c) = content { 
                c 
            } else { 
                extract_text_from_file(p).map_err(|e| KbError::ParseFailed(e.to_string()))? 
            };
            let file_name = p.file_name().and_then(|n| n.to_str()).unwrap_or("unknown").to_string();
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
            (parsed_content, file_name, Some(p.to_string_lossy().to_string()), ext)
        } else if let Some(c) = content {
            let ft = if category == "video-transcript" { "mp4".to_string() } else { "txt".to_string() };
            (c, "视频转写文本".to_string(), None, ft)
        } else {
            return Err(KbError::ParseFailed("必须提供文件路径或内容".to_string()));
        };

        // 备份原始文件到知识库目录
        let backup_path = if let Some(bdir) = backup_dir {
            std::fs::create_dir_all(bdir).ok();
            if let Some(p) = path {
                if p.exists() {
                    // 备份原始文件  
                    let _ext = p.extension().and_then(|e| e.to_str()).unwrap_or("bin");
                    let backup_name = format!("{}_{}", &doc_id[..8], 
                        p.file_name().and_then(|n| n.to_str()).unwrap_or("file"));
                    let backup_file = bdir.join(&backup_name);
                    std::fs::copy(p, &backup_file).ok();
                    
                    // 如果是视频类型，额外保存转写文本
                    if category == "video-transcript" {
                        let txt_name = format!("{}_{}.txt", &doc_id[..8], 
                            p.file_stem().and_then(|n| n.to_str()).unwrap_or("video"));
                        let txt_file = bdir.join(&txt_name);
                        std::fs::write(&txt_file, &final_content).ok();
                    }
                    
                    Some(backup_file.to_string_lossy().to_string())
                } else {
                    // 没有源文件，只保存文本
                    let txt_name = format!("{}_transcript.txt", &doc_id[..8]);
                    let txt_file = bdir.join(&txt_name);
                    std::fs::write(&txt_file, &final_content).ok();
                    Some(txt_file.to_string_lossy().to_string())
                }
            } else {
                // 纯文本（如视频转写），保存为 txt 文件
                let txt_name = format!("{}_transcript.txt", &doc_id[..8]);
                let txt_file = bdir.join(&txt_name);
                std::fs::write(&txt_file, &final_content).ok();
                Some(txt_file.to_string_lossy().to_string())
            }
        } else {
            None
        };

        // 生成向量嵌入
        let embedding = self.embedder.embed(&final_content)?;
        self.vector_db.insert(&doc_id, &embedding)?;

        let doc = Document {
            id: doc_id,
            name,
            category: category.to_string(),
            content: final_content,
            source_path,
            backup_path,
            file_type,
            created_at: Utc::now().to_rfc3339(),
        };

        // 同步写入 Tantivy 全文索引
        if let Err(e) = self.tantivy_index.add_document(&doc.id, &doc.name, &doc.content) {
            tracing::warn!("Tantivy 索引写入失败（非致命）: {}", e);
        }

        // 保存文档元数据到数据库
        self.vector_db.save_document(
            &doc.id, &doc.name, &doc.category, &doc.content,
            doc.source_path.as_deref(), doc.backup_path.as_deref(),
            &doc.file_type, &doc.created_at,
        )?;

        // 保存到内存中
        self.documents.write().push(doc.clone());
        Ok(doc)
    }



    /// 搜索相关知识
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, KbError> {
        let query_embedding = self.embedder.embed(query)?;
        tracing::info!("Query embedding length: {}", query_embedding.len());
        let similar_docs = self.vector_db.search(&query_embedding, limit)?;

        let documents = self.documents.read();
        let mut results = Vec::new();

        for (doc_id, relevance) in similar_docs {
            tracing::info!("Doc ID: {}, Relevance: {}", doc_id, relevance);
            if let Some(doc) = documents.iter().find(|d| d.id == doc_id) {
                let snippet = if doc.content.len() > 300 {
                    let char_boundary = doc.content.char_indices().nth(300).map(|(i, _)| i).unwrap_or(doc.content.len());
                    format!("{}...", &doc.content[..char_boundary])
                } else {
                    doc.content.clone()
                };

                results.push(SearchResult {
                    document: doc.clone(),
                    relevance,
                    snippet,
                });
            }
        }

        Ok(results)
    }

    /// 删除文档
    pub async fn delete_document(&self, id: &str) -> Result<(), KbError> {
        self.vector_db.delete(id)?;
        self.vector_db.delete_document(id)?;
        // 同步删除 Tantivy 索引
        if let Err(e) = self.tantivy_index.delete_document(id) {
            tracing::warn!("Tantivy 索引删除失败（非致命）: {}", e);
        }
        self.documents.write().retain(|d| d.id != id);
        Ok(())
    }

    /// 清除所有数据
    pub async fn clear_all(&self) -> Result<(), KbError> {
        self.vector_db.clear_all()?;
        if let Err(e) = self.tantivy_index.clear_all() {
            tracing::warn!("Tantivy 索引清除失败（非致命）: {}", e);
        }
        self.documents.write().clear();
        Ok(())
    }

    /// 获取所有文档
    pub async fn list_documents(&self) -> Result<Vec<Document>, KbError> {
        Ok(self.documents.read().clone())
    }

    /// 全文搜索（不依赖 AI，使用 Tantivy）
    pub async fn search_fulltext(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, KbError> {
        let ft_results = self.tantivy_index.search(query, limit)?;
        let documents = self.documents.read();
        let mut results = Vec::new();

        for ft in ft_results {
            if let Some(doc) = documents.iter().find(|d| d.id == ft.doc_id) {
                results.push(SearchResult {
                    document: doc.clone(),
                    relevance: ft.score,
                    snippet: ft.snippet,
                });
            }
        }

        Ok(results)
    }

    /// 是否使用语义嵌入（即是否有 AI 模型）
    pub fn has_semantic_embedder(&self) -> bool {
        self.embedder.is_semantic()
    }

    // ==========================================
    // 聊天记录 & 消息向量管理
    // ==========================================

    pub async fn save_conversation(&self, id: &str, title: &str) -> Result<(), KbError> {
        let now = Utc::now().to_rfc3339();
        self.vector_db.save_conversation(id, title, &now, &now)?;
        Ok(())
    }

    pub async fn load_conversations(&self) -> Result<Vec<(String, String, String, String)>, KbError> {
        self.vector_db.load_conversations().map_err(|e| KbError::VectorDbError(e))
    }

    pub async fn delete_conversation(&self, session_id: &str) -> Result<(), KbError> {
        self.vector_db.delete_conversation(session_id)?;
        Ok(())
    }

    pub async fn save_message(&self, id: &str, session_id: &str, role: &str, content: &str) -> Result<(), KbError> {
        let now = Utc::now().to_rfc3339();
        
        // 1. 保存消息本体
        self.vector_db.save_message(id, session_id, role, content, &now)?;
        
        // 2. 只有用户或 AI 的实际长消息才做向量化 (简单避免大量系统提示词冗余)
        if (role == "user" || role == "assistant") && content.len() > 10 {
            // 后台静默向量化，不阻塞主流程错误
            match self.embedder.embed(content) {
                Ok(embedding) => {
                    if let Err(e) = self.vector_db.insert_message_vector(id, session_id, &embedding) {
                        tracing::warn!("消息 {} 向量化插入失败: {}", id, e);
                    }
                }
                Err(e) => {
                    tracing::warn!("消息 {} 无法生成嵌入: {}", id, e);
                }
            }
        }
        
        Ok(())
    }

    pub async fn load_messages(&self, session_id: &str) -> Result<Vec<(String, String, String, String, String)>, KbError> {
        self.vector_db.load_messages(session_id).map_err(|e| KbError::VectorDbError(e))
    }

    pub async fn search_chat_history(&self, query: &str, session_id: Option<&str>, limit: usize) -> Result<Vec<(String, String, String)>, KbError> {
        let query_embedding = self.embedder.embed(query)?;
        let sim_msgs = self.vector_db.search_messages(&query_embedding, session_id, limit)?;
        
        let mut results = Vec::new();
        for (msg_id, session_id, _sim) in sim_msgs {
            if let Ok(Some(content)) = self.vector_db.get_message_content(&msg_id) {
                results.push((msg_id, session_id, content));
            }
        }
        
        Ok(results)
    }
}

pub struct SimpleEmbedder {
    dimension: usize,
}

impl SimpleEmbedder {
    pub fn new(dimension: usize) -> Self {
        Self { dimension }
    }

    pub fn embed(&self, text: &str) -> Vec<f32> {
        // 简单的伪随机嵌入，仅用于回退测试
        let mut embedding = vec![0.0; self.dimension];
        let chars: Vec<char> = text.chars().collect();
        for (i, ch) in chars.iter().enumerate() {
            let idx = (*ch as usize + i) % self.dimension;
            embedding[idx] += 1.0;
        }
        // 归一化
        let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut embedding {
                *x /= norm;
            }
        }
        embedding
    }
}
