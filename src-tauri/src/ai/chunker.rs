#[derive(Debug, Clone)]
pub struct Chunk {
    pub index: usize,
    pub content: String,
}

/// 基于结构和滑动窗口混合规则的文档切分
pub fn chunk_text(text: &str, max_len: usize, overlap: usize) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let mut chunk_idx = 0;

    // 1. 粗切：基于段落双换行进行初步切分。这能大致保留 Markdown 结构的完整段落。
    let paragraphs: Vec<&str> = text.split("\n\n").collect();
    
    let mut current_block = String::new();

    for p in paragraphs {
        let p = p.trim();
        if p.is_empty() {
            continue;
        }

        let p_len = p.chars().count();
        let current_len = current_block.chars().count();

        // 如果单段已经超长，先结算之前的，再对单段进行滑动窗口硬切
        if p_len > max_len {
            if !current_block.is_empty() {
                chunks.push(Chunk {
                    index: chunk_idx,
                    content: current_block.clone(),
                });
                chunk_idx += 1;
                current_block.clear();
            }

            let window_chunks = sliding_window_chunk(p, max_len, overlap);
            for wc in window_chunks {
                chunks.push(Chunk {
                    index: chunk_idx,
                    content: wc,
                });
                chunk_idx += 1;
            }
        } 
        // 还没超限，可以拼
        else if current_len > 0 && current_len + p_len + 2 <= max_len {
            current_block.push_str("\n\n");
            current_block.push_str(p);
        } 
        else if current_len == 0 {
            current_block.push_str(p);
        }
        // 加上就超限了，先结算现在的，再把当前的 p 作为新块的开始
        else {
            chunks.push(Chunk {
                index: chunk_idx,
                content: current_block.clone(),
            });
            chunk_idx += 1;
            current_block.clear();
            current_block.push_str(p);
        }
    }

    // 处理收尾
    if !current_block.is_empty() {
        chunks.push(Chunk {
            index: chunk_idx,
            content: current_block,
        });
    }

    chunks
}

/// 滑动窗口强切
fn sliding_window_chunk(text: &str, max_len: usize, overlap: usize) -> Vec<String> {
    let mut result = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut start = 0;
    
    while start < chars.len() {
        let end = std::cmp::min(start + max_len, chars.len());
        let chunk: String = chars[start..end].iter().collect();
        result.push(chunk);
        
        if end >= chars.len() {
            break;
        }
        
        let step = if max_len > overlap { max_len - overlap } else { 1 };
        start += step;
    }
    
    result
}
