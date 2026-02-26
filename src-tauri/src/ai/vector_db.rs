use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::{Arc, Mutex};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum VectorDbError {
    #[error("数据库连接失败: {0}")]
    ConnectionFailed(String),
    #[error("向量插入失败: {0}")]
    InsertFailed(String),
    #[error("向量搜索失败: {0}")]
    SearchFailed(String),
    #[error("数据库错误: {0}")]
    DatabaseError(#[from] rusqlite::Error),
}

pub struct VectorDb {
    conn: Arc<Mutex<Connection>>,
}

impl VectorDb {
    /// 创建新的向量数据库实例
    pub fn new(db_path: &Path) -> Result<Self, VectorDbError> {
        let conn = Connection::open(db_path)
            .map_err(|e| VectorDbError::ConnectionFailed(e.to_string()))?;

        // 创建向量表
        conn.execute(
            "CREATE TABLE IF NOT EXISTS document_vectors (
                document_id TEXT PRIMARY KEY,
                embedding BLOB NOT NULL,
                dimension INTEGER NOT NULL,
                created_at TEXT NOT NULL
            )",
            [],
        )?;

        // 创建文档元数据表
        conn.execute(
            "CREATE TABLE IF NOT EXISTS documents (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                category TEXT NOT NULL,
                content TEXT NOT NULL,
                source_path TEXT,
                backup_path TEXT,
                file_type TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL
            )",
            [],
        )?;

        // 数据库迁移：为旧表添加新列（如果不存在）
        let _ = conn.execute("ALTER TABLE documents ADD COLUMN backup_path TEXT", []);
        let _ = conn.execute("ALTER TABLE documents ADD COLUMN file_type TEXT NOT NULL DEFAULT ''", []);

        // ===== 聊天记录表 =====
        // 会话表
        conn.execute(
            "CREATE TABLE IF NOT EXISTS conversations (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
            [],
        )?;

        // 消息表
        conn.execute(
            "CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL
            )",
            [],
        )?;

        // 消息向量表 (后续做检索用)
        conn.execute(
            "CREATE TABLE IF NOT EXISTS message_vectors (
                message_id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                embedding BLOB NOT NULL,
                dimension INTEGER NOT NULL,
                created_at TEXT NOT NULL
            )",
            [],
        )?;

        // 创建索引以加速搜索
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_document_id ON document_vectors(document_id)",
            [],
        )?;
        
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id)",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_messages_created ON messages(created_at)",
            [],
        )?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// 保存文档元数据
    pub fn save_document(
        &self, id: &str, name: &str, category: &str, content: &str,
        source_path: Option<&str>, backup_path: Option<&str>, file_type: &str, created_at: &str,
    ) -> Result<(), VectorDbError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO documents (id, name, category, content, source_path, backup_path, file_type, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![id, name, category, content, source_path, backup_path, file_type, created_at],
        ).map_err(|e| VectorDbError::InsertFailed(e.to_string()))?;
        Ok(())
    }

    /// 加载所有文档元数据
    pub fn load_documents(&self) -> Result<Vec<(String, String, String, String, Option<String>, Option<String>, String, String)>, VectorDbError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id, name, category, content, source_path, backup_path, file_type, created_at FROM documents")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, String>(6).unwrap_or_default(),
                row.get::<_, String>(7)?,
            ))
        })?;
        
        let mut docs = Vec::new();
        for row in rows {
            docs.push(row?);
        }
        Ok(docs)
    }

    /// 删除文档元数据
    pub fn delete_document(&self, id: &str) -> Result<(), VectorDbError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM documents WHERE id = ?1", params![id])?;
        Ok(())
    }

    // ==========================================
    // 聊天记录管理
    // ==========================================

    pub fn save_conversation(&self, id: &str, title: &str, created_at: &str, updated_at: &str) -> Result<(), VectorDbError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO conversations (id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
            params![id, title, created_at, updated_at],
        ).map_err(|e| VectorDbError::InsertFailed(e.to_string()))?;
        Ok(())
    }

    pub fn load_conversations(&self) -> Result<Vec<(String, String, String, String)>, VectorDbError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id, title, created_at, updated_at FROM conversations ORDER BY updated_at DESC")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        
        let mut list = Vec::new();
        for row in rows {
            list.push(row?);
        }
        Ok(list)
    }

    pub fn delete_conversation(&self, session_id: &str) -> Result<(), VectorDbError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM conversations WHERE id = ?1", params![session_id])?;
        conn.execute("DELETE FROM messages WHERE session_id = ?1", params![session_id])?;
        conn.execute("DELETE FROM message_vectors WHERE session_id = ?1", params![session_id])?;
        Ok(())
    }

    pub fn save_message(&self, id: &str, session_id: &str, role: &str, content: &str, created_at: &str) -> Result<(), VectorDbError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO messages (id, session_id, role, content, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, session_id, role, content, created_at],
        ).map_err(|e| VectorDbError::InsertFailed(e.to_string()))?;
        
        // 同时更新对话的 updated_at
        conn.execute(
            "UPDATE conversations SET updated_at = ?2 WHERE id = ?1",
            params![session_id, created_at],
        )?;

        Ok(())
    }

    pub fn load_messages(&self, session_id: &str) -> Result<Vec<(String, String, String, String, String)>, VectorDbError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id, session_id, role, content, created_at FROM messages WHERE session_id = ?1 ORDER BY created_at ASC")?;
        let rows = stmt.query_map([session_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        
        let mut list = Vec::new();
        for row in rows {
            list.push(row?);
        }
        Ok(list)
    }

    pub fn get_message_content(&self, id: &str) -> Result<Option<String>, VectorDbError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT content FROM messages WHERE id = ?1")?;
        let mut rows = stmt.query([id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    /// 获取当前数据库中存储的向量维度
    pub fn get_dimension(&self) -> Result<usize, VectorDbError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT dimension FROM document_vectors LIMIT 1")?;
        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            let dim: i32 = row.get(0)?;
            Ok(dim as usize)
        } else {
            Ok(0)
        }
    }

    /// 插入向量
    pub fn insert(&self, document_id: &str, embedding: &[f32]) -> Result<(), VectorDbError> {
        let embedding_bytes = self.f32_slice_to_bytes(embedding);
        let dimension = embedding.len() as i32;
        let created_at = chrono::Utc::now().to_rfc3339();

        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO document_vectors (document_id, embedding, dimension, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![document_id, embedding_bytes, dimension, created_at],
        ).map_err(|e| VectorDbError::InsertFailed(e.to_string()))?;

        Ok(())
    }

    /// 插入消息向量
    pub fn insert_message_vector(&self, message_id: &str, session_id: &str, embedding: &[f32]) -> Result<(), VectorDbError> {
        let embedding_bytes = self.f32_slice_to_bytes(embedding);
        let dimension = embedding.len() as i32;
        let created_at = chrono::Utc::now().to_rfc3339();

        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO message_vectors (message_id, session_id, embedding, dimension, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![message_id, session_id, embedding_bytes, dimension, created_at],
        ).map_err(|e| VectorDbError::InsertFailed(e.to_string()))?;

        Ok(())
    }

    /// 搜索相似向量（使用余弦相似度）
    pub fn search(
        &self,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<(String, f32)>, VectorDbError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT document_id, embedding, dimension FROM document_vectors")
            .map_err(|e| VectorDbError::SearchFailed(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                let document_id: String = row.get(0)?;
                let embedding_bytes: Vec<u8> = row.get(1)?;
                let dimension: i32 = row.get(2)?;
                Ok((document_id, embedding_bytes, dimension))
            })
            .map_err(|e| VectorDbError::SearchFailed(e.to_string()))?;

        let mut results = Vec::new();

        for row in rows {
            let (document_id, embedding_bytes, dimension) =
                row.map_err(|e| VectorDbError::SearchFailed(e.to_string()))?;

            let embedding = self.bytes_to_f32_slice(&embedding_bytes, dimension as usize);
            let similarity = self.cosine_similarity(query_embedding, &embedding);
            
            tracing::info!("vector db doc dimension: {}, query dimension: {}", embedding.len(), query_embedding.len());

            results.push((document_id, similarity));
        }

        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(results.into_iter().take(limit).collect())
    }

    /// 搜索相似消息向量
    pub fn search_messages(
        &self,
        query_embedding: &[f32],
        session_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<(String, String, f32)>, VectorDbError> {
        let conn = self.conn.lock().unwrap();
        let mut results = Vec::new();

        if let Some(sid) = session_id {
            let mut stmt = conn.prepare("SELECT message_id, session_id, embedding, dimension FROM message_vectors WHERE session_id = ?1")?;
            let rows = stmt.query_map([sid], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, i32>(3)?,
                ))
            })?;

            for row in rows {
                let (message_id, session_id, embedding_bytes, dimension) =
                    row.map_err(|e| VectorDbError::SearchFailed(e.to_string()))?;

                let embedding = self.bytes_to_f32_slice(&embedding_bytes, dimension as usize);
                let similarity = self.cosine_similarity(query_embedding, &embedding);
                results.push((message_id, session_id, similarity));
            }
        } else {
            let mut stmt = conn.prepare("SELECT message_id, session_id, embedding, dimension FROM message_vectors")?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, i32>(3)?,
                ))
            })?;

            for row in rows {
                let (message_id, session_id, embedding_bytes, dimension) =
                    row.map_err(|e| VectorDbError::SearchFailed(e.to_string()))?;

                let embedding = self.bytes_to_f32_slice(&embedding_bytes, dimension as usize);
                let similarity = self.cosine_similarity(query_embedding, &embedding);
                results.push((message_id, session_id, similarity));
            }
        }

        results.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        Ok(results.into_iter().take(limit).collect())
    }

    /// 删除向量
    pub fn delete(&self, document_id: &str) -> Result<(), VectorDbError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM document_vectors WHERE document_id = ?1",
            params![document_id],
        )?;
        Ok(())
    }

    /// 清除所有向量数据
    pub fn clear_all(&self) -> Result<(), VectorDbError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM document_vectors", [])?;
        conn.execute("DELETE FROM documents", [])?;
        Ok(())
    }

    fn f32_slice_to_bytes(&self, slice: &[f32]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(slice.len() * 4);
        for &value in slice {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    fn bytes_to_f32_slice(&self, bytes: &[u8], dimension: usize) -> Vec<f32> {
        let mut result = Vec::with_capacity(dimension);
        for i in 0..dimension {
            let start = i * 4;
            let end = start + 4;
            if end <= bytes.len() {
                let bytes_array: [u8; 4] = bytes[start..end].try_into().unwrap();
                result.push(f32::from_le_bytes(bytes_array));
            }
        }
        result
    }

    fn cosine_similarity(&self, a: &[f32], b: &[f32]) -> f32 {
        if a.len() != b.len() {
            return 0.0;
        }
        let dot_product: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm_a == 0.0 || norm_b == 0.0 {
            return 0.0;
        }
        dot_product / (norm_a * norm_b)
    }
}
