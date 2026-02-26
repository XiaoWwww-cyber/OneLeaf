use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ParserError {
    #[error("Document not found: {0}")]
    NotFound(String),
    #[error("Failed to parse document: {0}")]
    ParseFailed(String),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

/// 解析文档并返回纯文本内容
pub fn extract_text_from_file(path: &Path) -> Result<String, ParserError> {
    if !path.exists() {
        return Err(ParserError::NotFound(path.display().to_string()));
    }

    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    match extension.as_str() {
        "txt" | "md" => parse_txt(path),
        "docx" => parse_docx(path),
        "doc" => Err(ParserError::ParseFailed(
            "不支持旧版 .doc 格式，请转换为 .docx".to_string(),
        )),
        "pdf" => parse_pdf(path),
        _ => Err(ParserError::ParseFailed(format!(
            "不支持的文档格式: {}",
            extension
        ))),
    }
}

fn parse_txt(path: &Path) -> Result<String, ParserError> {
    std::fs::read_to_string(path)
        .map_err(|e| ParserError::ParseFailed(format!("读取文本文件失败: {}", e)))
}

fn parse_docx(path: &Path) -> Result<String, ParserError> {
    use std::fs::File;
    use std::io::Read;
    use zip::ZipArchive;

    let file = File::open(path)
        .map_err(|e| ParserError::ParseFailed(format!("打开 DOCX 文件失败: {}", e)))?;

    let mut archive = ZipArchive::new(file)
        .map_err(|e| ParserError::ParseFailed(format!("解析 DOCX 压缩包失败: {}", e)))?;

    let mut document_xml = archive
        .by_name("word/document.xml")
        .map_err(|e| ParserError::ParseFailed(format!("找不到 document.xml: {}", e)))?;

    let mut xml_content = String::new();
    document_xml
        .read_to_string(&mut xml_content)
        .map_err(|e| ParserError::ParseFailed(format!("读取 document.xml 失败: {}", e)))?;

    // 简单的 XML 文本提取 (可以使用 regex 优化，这里复用简单的逻辑)
    let text = extract_text_from_xml(&xml_content);

    if text.trim().is_empty() {
        return Err(ParserError::ParseFailed("文档内容为空".to_string()));
    }

    Ok(text)
}

fn extract_text_from_xml(xml: &str) -> String {
    // 简化版 XML 提取，仅提取 <w:t> 内容
    let regex = regex::Regex::new(r"<w:t[^>]*>([^<]*)</w:t>").unwrap();
    let mut text = String::new();
    for cap in regex.captures_iter(xml) {
        text.push_str(&cap[1]);
    }
    text
}

fn parse_pdf(path: &Path) -> Result<String, ParserError> {
    let text = pdf_extract::extract_text(path)
        .map_err(|e| ParserError::ParseFailed(format!("解析 PDF 失败: {}", e)))?;

    if text.trim().is_empty() {
        return Err(ParserError::ParseFailed("PDF 文档内容为空".to_string()));
    }

    Ok(text)
}
