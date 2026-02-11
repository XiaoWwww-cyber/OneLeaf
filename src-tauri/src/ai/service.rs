// AI 服务层
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Clone)]
pub struct AiService {
    // 可以在此添加 LLM 客户端 (OpenAI, DeepSeek, Etc.)
    // 目前仅做简单模拟或转发
}

impl AiService {
    pub fn new() -> Self {
        Self {}
    }

    pub async fn chat(&self, messages: Vec<ChatMessage>) -> Result<String, String> {
        // TODO: 这里应接入真实的 LLM 服务 (OpenAI, DeepSeek, Local LLM)
        // 暂时返回一个简单的回复，或者集成 OpenAI
        
        // 模拟回复
        let last_msg = messages.last().map(|m| m.content.clone()).unwrap_or_default();
        Ok(format!("(AI 回复) 我收到了你的消息: {}", last_msg))
    }
}
