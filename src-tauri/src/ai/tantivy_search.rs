//! Tantivy 全文搜索引擎
//! 使用 jieba 中文分词，提供不依赖 AI 的本地全文搜索能力

use std::path::Path;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::*;
use tantivy::{doc, Index, IndexReader, IndexWriter, ReloadPolicy};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SearchError {
    #[error("索引创建失败: {0}")]
    IndexCreation(String),
    #[error("索引写入失败: {0}")]
    IndexWrite(String),
    #[error("搜索失败: {0}")]
    SearchFailed(String),
    #[error("Tantivy 错误: {0}")]
    TantivyError(#[from] tantivy::TantivyError),
}

/// 全文搜索结果
#[derive(Debug, Clone)]
pub struct FulltextResult {
    pub doc_id: String,
    pub score: f32,
    pub snippet: String,
    pub content: String,
}

pub struct TantivyIndex {
    index: Index,
    reader: IndexReader,
    /// 文档 ID 字段
    field_doc_id: Field,
    /// 标题字段（可搜索）
    field_title: Field,
    /// 正文字段（可搜索）
    field_content: Field,
}

impl TantivyIndex {
    /// 创建或打开 Tantivy 索引
    ///
    /// `index_dir` 为索引存储目录，不存在时自动创建
    pub fn new(index_dir: &Path) -> Result<Self, SearchError> {
        std::fs::create_dir_all(index_dir)
            .map_err(|e| SearchError::IndexCreation(format!("创建索引目录失败: {}", e)))?;

        // 注册 jieba 中文分词器
        let tokenizer = tantivy_jieba::JiebaTokenizer {};

        // 构建 Schema
        let mut schema_builder = Schema::builder();

        let doc_id = schema_builder.add_text_field("doc_id", STRING | STORED);
        let title = schema_builder.add_text_field(
            "title",
            TextOptions::default()
                .set_indexing_options(
                    TextFieldIndexing::default()
                        .set_tokenizer("jieba")
                        .set_index_option(IndexRecordOption::WithFreqsAndPositions),
                )
                .set_stored(),
        );
        let content = schema_builder.add_text_field(
            "content",
            TextOptions::default()
                .set_indexing_options(
                    TextFieldIndexing::default()
                        .set_tokenizer("jieba")
                        .set_index_option(IndexRecordOption::WithFreqsAndPositions),
                )
                .set_stored(),
        );

        let schema = schema_builder.build();

        // 打开或创建索引
        let index = if index_dir.join("meta.json").exists() {
            Index::open_in_dir(index_dir)?
        } else {
            Index::create_in_dir(index_dir, schema)?
        };

        // 注册分词器
        index.tokenizers().register("jieba", tokenizer);

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;

        Ok(Self {
            index,
            reader,
            field_doc_id: doc_id,
            field_title: title,
            field_content: content,
        })
    }

    /// 添加文档到索引
    pub fn add_document(
        &self,
        doc_id: &str,
        title: &str,
        content: &str,
    ) -> Result<(), SearchError> {
        let mut writer = self.create_writer()?;

        // 先删除旧文档（如果存在）
        let term = tantivy::Term::from_field_text(self.field_doc_id, doc_id);
        writer.delete_term(term);

        writer.add_document(doc!(
            self.field_doc_id => doc_id,
            self.field_title => title,
            self.field_content => content,
        ))?;

        writer
            .commit()
            .map_err(|e| SearchError::IndexWrite(format!("提交索引失败: {}", e)))?;

        self.reader.reload()?;
        Ok(())
    }

    /// 从索引中删除文档
    pub fn delete_document(&self, doc_id: &str) -> Result<(), SearchError> {
        let mut writer = self.create_writer()?;
        let term = tantivy::Term::from_field_text(self.field_doc_id, doc_id);
        writer.delete_term(term);
        writer
            .commit()
            .map_err(|e| SearchError::IndexWrite(format!("提交索引失败: {}", e)))?;
        self.reader.reload()?;
        Ok(())
    }

    /// 全文搜索
    ///
    /// 返回 `(doc_id, score, snippet)` 列表，按相关度降序
    pub fn search(
        &self,
        query_str: &str,
        limit: usize,
    ) -> Result<Vec<FulltextResult>, SearchError> {
        if query_str.trim().is_empty() {
            return Ok(vec![]);
        }

        let searcher = self.reader.searcher();
        let query_parser =
            QueryParser::for_index(&self.index, vec![self.field_title, self.field_content]);

        let query = query_parser
            .parse_query(query_str)
            .map_err(|e| SearchError::SearchFailed(format!("解析查询失败: {}", e)))?;

        let top_docs = searcher
            .search(&query, &TopDocs::with_limit(limit))
            .map_err(|e| SearchError::SearchFailed(format!("搜索执行失败: {}", e)))?;

        let mut results = Vec::new();

        for (score, doc_address) in top_docs {
            let retrieved_doc: TantivyDocument = searcher.doc(doc_address)?;

            let doc_id = retrieved_doc
                .get_first(self.field_doc_id)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let content = retrieved_doc
                .get_first(self.field_content)
                .and_then(|v| v.as_str())
                .unwrap_or("");

            // 生成摘要片段：找到查询词附近的文本
            let snippet = self.generate_snippet(content, query_str);

            results.push(FulltextResult {
                doc_id,
                score,
                snippet,
                content: content.to_string(),
            });
        }

        Ok(results)
    }

    /// 清除所有索引数据
    pub fn clear_all(&self) -> Result<(), SearchError> {
        let mut writer = self.create_writer()?;
        writer.delete_all_documents()?;
        writer
            .commit()
            .map_err(|e| SearchError::IndexWrite(format!("提交索引失败: {}", e)))?;
        self.reader.reload()?;
        Ok(())
    }

    /// 创建 IndexWriter（固定 50MB 堆内存）
    fn create_writer(&self) -> Result<IndexWriter, SearchError> {
        self.index
            .writer(50_000_000)
            .map_err(|e| SearchError::IndexWrite(format!("创建 writer 失败: {}", e)))
    }

    /// 生成搜索结果摘要片段
    ///
    /// 尝试找到查询词在正文中的位置，截取前后各 100 字符
    fn generate_snippet(&self, content: &str, query: &str) -> String {
        let query_lower = query.to_lowercase();
        let content_lower = content.to_lowercase();

        // 查找首个匹配位置
        if let Some(pos) = content_lower.find(&query_lower) {
            let chars: Vec<char> = content.chars().collect();
            // 将字节位置转换为字符位置
            let char_pos = content[..pos].chars().count();
            let start = char_pos.saturating_sub(50);
            let end = (char_pos + query.chars().count() + 100).min(chars.len());

            let mut snippet: String = chars[start..end].iter().collect();
            if start > 0 {
                snippet = format!("...{}", snippet);
            }
            if end < chars.len() {
                snippet = format!("{}...", snippet);
            }
            snippet
        } else {
            // 没有精确匹配，返回前 200 字符
            let chars: Vec<char> = content.chars().collect();
            let end = 200.min(chars.len());
            let mut snippet: String = chars[..end].iter().collect();
            if end < chars.len() {
                snippet = format!("{}...", snippet);
            }
            snippet
        }
    }
}
