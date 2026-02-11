// 知识库管理

use crate::ai::onnx_embedder::{EmbedderError, OnnxEmbedder};
use crate::ai::vector_db::{VectorDb, VectorDbError};
use chrono::Utc;
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;
use uuid::Uuid;

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
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Document {
    pub id: String,
    pub name: String,
    pub category: String,
    pub content: String,
    pub source_path: Option<String>,
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

    /// 是否使用语义嵌入
    pub fn is_semantic(&self) -> bool {
        matches!(self, Embedder::Onnx(_))
    }
}

pub struct KnowledgeBase {
    vector_db: Arc<VectorDb>,
    embedder: Arc<Embedder>,
    documents: Arc<parking_lot::RwLock<Vec<Document>>>,
}

impl Clone for KnowledgeBase {
    fn clone(&self) -> Self {
        Self {
            vector_db: Arc::clone(&self.vector_db),
            embedder: Arc::clone(&self.embedder),
            documents: Arc::clone(&self.documents),
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

        // 尝试加载 ONNX 嵌入器，失败则回退到 SimpleEmbedder
        let embedder = if let Some(dir) = model_dir {
            match OnnxEmbedder::new(dir) {
                Ok(onnx) => {
                    tracing::info!("使用 ONNX 语义嵌入模型: {:?}", dir);
                    Arc::new(Embedder::Onnx(Mutex::new(onnx)))
                }
                Err(e) => {
                    tracing::warn!("ONNX 模型加载失败，回退到 SimpleEmbedder: {}", e);
                    Arc::new(Embedder::Simple(SimpleEmbedder::new(384)))
                }
            }
        } else {
            tracing::info!("未指定模型目录，使用 SimpleEmbedder");
            Arc::new(Embedder::Simple(SimpleEmbedder::new(384)))
        };

        // 从数据库加载已保存的文档
        if let Ok(saved_docs) = vector_db.load_documents() {
            let mut docs = documents.write();
            for (id, name, category, content, source_path, created_at) in saved_docs {
                docs.push(Document {
                    id,
                    name,
                    category,
                    content,
                    source_path,
                    created_at,
                });
            }
            tracing::info!("从数据库加载了 {} 个文档", docs.len());
        }

        Ok(Self {
            vector_db,
            embedder,
            documents,
        })
    }

    /// 添加文档到知识库
    /// 
    /// 如果 content 不为空，直接使用 content 作为文档内容 (例如从视频转写的文本)
    /// 如果 content 为空且 path 存在，则尝试解析文件内容
    pub async fn add_document(&self, path: Option<&PathBuf>, content: Option<String>, category: &str) -> Result<Document, KbError> {
        // 确定文档内容和名称
        let (final_content, name, source_path) = if let Some(p) = path {
            if !p.exists() && content.is_none() {
                return Err(KbError::DocumentNotFound(p.display().to_string()));
            }
            
            let parsed_content = if let Some(c) = content {
                c
            } else {
                self.parse_document(p)?
            };

            let file_name = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
                
            (parsed_content, file_name, Some(p.to_string_lossy().to_string()))
        } else if let Some(c) = content {
            (c, "Text Snippet".to_string(), None)
        } else {
            return Err(KbError::ParseFailed("必须提供文件路径或内容".to_string()));
        };

        // 生成文档 ID
        let doc_id = Uuid::new_v4().to_string();

        // 生成向量嵌入
        let embedding = self.embedder.embed(&final_content)?;

        // 保存向量到数据库
        self.vector_db.insert(&doc_id, &embedding)?;

        // 创建文档对象
        let doc = Document {
            id: doc_id,
            name,
            category: category.to_string(),
            content: final_content,
            source_path,
            created_at: Utc::now().to_rfc3339(),
        };

        // 保存文档元数据到数据库
        self.vector_db.save_document(
            &doc.id, 
            &doc.name, 
            &doc.category, 
            &doc.content, 
            doc.source_path.as_deref(), 
            &doc.created_at
        )?;

        // 保存到内存中
        self.documents.write().push(doc.clone());

        Ok(doc)
    }

    /// 解析文档内容
    fn parse_document(&self, path: &PathBuf) -> Result<String, KbError> {
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        match extension.as_str() {
            "txt" | "md" => self.parse_txt(path),
            "docx" => self.parse_docx(path),
            "doc" => Err(KbError::ParseFailed(
                "不支持旧版 .doc 格式，请转换为 .docx".to_string(),
            )),
            "pdf" => self.parse_pdf(path),
            _ => Err(KbError::ParseFailed(format!(
                "不支持的文档格式: {}",
                extension
            ))),
        }
    }

    fn parse_txt(&self, path: &Path) -> Result<String, KbError> {
        std::fs::read_to_string(path)
            .map_err(|e| KbError::ParseFailed(format!("读取文本文件失败: {}", e)))
    }

    fn parse_docx(&self, path: &Path) -> Result<String, KbError> {
        use std::fs::File;
        use std::io::Read;
        use zip::ZipArchive;

        let file = File::open(path)
            .map_err(|e| KbError::ParseFailed(format!("打开 DOCX 文件失败: {}", e)))?;

        let mut archive = ZipArchive::new(file)
            .map_err(|e| KbError::ParseFailed(format!("解析 DOCX 压缩包失败: {}", e)))?;

        let mut document_xml = archive
            .by_name("word/document.xml")
            .map_err(|e| KbError::ParseFailed(format!("找不到 document.xml: {}", e)))?;

        let mut xml_content = String::new();
        document_xml
            .read_to_string(&mut xml_content)
            .map_err(|e| KbError::ParseFailed(format!("读取 document.xml 失败: {}", e)))?;

        // 简单的 XML 文本提取 (可以使用 regex 优化，这里复用简单的逻辑)
        let text = self.extract_text_from_xml(&xml_content);

        if text.trim().is_empty() {
            return Err(KbError::ParseFailed("文档内容为空".to_string()));
        }

        Ok(text)
    }

    fn extract_text_from_xml(&self, xml: &str) -> String {
        // 简化版 XML 提取，仅提取 <w:t> 内容
        // 实际使用 regex 会更健壮
        let regex = regex::Regex::new(r"<w:t[^>]*>([^<]*)</w:t>").unwrap();
        let mut text = String::new();
        for cap in regex.captures_iter(xml) {
            text.push_str(&cap[1]);
        }
        text
    }

    fn parse_pdf(&self, path: &Path) -> Result<String, KbError> {
        let text = pdf_extract::extract_text(path)
            .map_err(|e| KbError::ParseFailed(format!("解析 PDF 失败: {}", e)))?;

        if text.trim().is_empty() {
            return Err(KbError::ParseFailed("PDF 文档内容为空".to_string()));
        }

        Ok(text)
    }

    /// 搜索相关知识
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, KbError> {
        let query_embedding = self.embedder.embed(query)?;
        let similar_docs = self.vector_db.search(&query_embedding, limit)?;

        let documents = self.documents.read();
        let mut results = Vec::new();

        for (doc_id, relevance) in similar_docs {
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
        self.documents.write().retain(|d| d.id != id);
        Ok(())
    }

    /// 清除所有数据
    pub async fn clear_all(&self) -> Result<(), KbError> {
        self.vector_db.clear_all()?;
        self.documents.write().clear();
        Ok(())
    }

    /// 获取所有文档
    pub async fn list_documents(&self) -> Result<Vec<Document>, KbError> {
        Ok(self.documents.read().clone())
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
