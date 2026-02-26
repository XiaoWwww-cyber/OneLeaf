// AI 服务适配器
// 支持豆包、OpenAI、DeepSeek、LM Studio
// 仿照 douyin-creator-toolkit 的 service.rs 实现

use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AiError {
    #[error("API 调用失败: {0}")]
    ApiCallFailed(String),
    #[error("API Key 无效")]
    InvalidApiKey,
    #[error("服务不可用: {0}")]
    ServiceUnavailable(String),
    #[error("响应解析失败: {0}")]
    ParseError(String),
    #[error("网络错误: {0}")]
    NetworkError(#[from] reqwest::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

// OpenAI 兼容的请求/响应结构
#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    temperature: f32,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

// ========== 提供者实现 ==========

/// 豆包 API 提供者
struct DoubaoProvider {
    client: Client,
    api_key: String,
    model: String,
    base_url: String,
}

impl DoubaoProvider {
    fn new(api_key: String) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap_or_default(),
            api_key,
            model: "doubao-seed-1-8-251228".to_string(),
            base_url: "https://ark.cn-beijing.volces.com/api/v3".to_string(),
        }
    }

    async fn chat(&self, messages: Vec<ChatMessage>) -> Result<String, AiError> {
        let request = ChatRequest {
            model: self.model.clone(),
            messages,
            temperature: 0.7,
        };
        self.send_request(&request).await
    }

    async fn send_request(&self, request: &ChatRequest) -> Result<String, AiError> {
        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .timeout(std::time::Duration::from_secs(60))
            .json(request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(AiError::ApiCallFailed(format!(
                "状态码: {}, 响应: {}",
                status, text
            )));
        }

        let chat_response: ChatResponse = response
            .json()
            .await
            .map_err(|e| AiError::ParseError(e.to_string()))?;

        chat_response
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .ok_or_else(|| AiError::ParseError("空响应".to_string()))
    }
}

/// OpenAI API 提供者
struct OpenAiProvider {
    client: Client,
    api_key: String,
    model: String,
    base_url: String,
}

impl OpenAiProvider {
    fn new(api_key: String) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap_or_default(),
            api_key,
            model: "gpt-4".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
        }
    }

    async fn chat(&self, messages: Vec<ChatMessage>) -> Result<String, AiError> {
        let request = ChatRequest {
            model: self.model.clone(),
            messages,
            temperature: 0.7,
        };

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            if status.as_u16() == 401 {
                return Err(AiError::InvalidApiKey);
            }
            return Err(AiError::ApiCallFailed(format!(
                "状态码: {}, 响应: {}",
                status, text
            )));
        }

        let chat_response: ChatResponse = response
            .json()
            .await
            .map_err(|e| AiError::ParseError(e.to_string()))?;

        chat_response
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .ok_or_else(|| AiError::ParseError("空响应".to_string()))
    }
}

/// DeepSeek API 提供者
struct DeepSeekProvider {
    client: Client,
    api_key: String,
    model: String,
    base_url: String,
}

impl DeepSeekProvider {
    fn new(api_key: String) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap_or_default(),
            api_key,
            model: "deepseek-chat".to_string(),
            base_url: "https://api.deepseek.com/v1".to_string(),
        }
    }

    async fn chat(&self, messages: Vec<ChatMessage>) -> Result<String, AiError> {
        let request = ChatRequest {
            model: self.model.clone(),
            messages,
            temperature: 0.7,
        };

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            if status.as_u16() == 401 {
                return Err(AiError::InvalidApiKey);
            }
            return Err(AiError::ApiCallFailed(format!(
                "状态码: {}, 响应: {}",
                status, text
            )));
        }

        let chat_response: ChatResponse = response
            .json()
            .await
            .map_err(|e| AiError::ParseError(e.to_string()))?;

        chat_response
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .ok_or_else(|| AiError::ParseError("空响应".to_string()))
    }
}

/// LM Studio 本地模型提供者
struct LmStudioProvider {
    client: Client,
    base_url: String,
}

impl LmStudioProvider {
    fn new(base_url: String) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self { client, base_url }
    }

    /// 检测 LM Studio 是否运行
    async fn is_running(&self) -> bool {
        match Client::new()
            .get(format!("{}/v1/models", self.base_url))
            .timeout(std::time::Duration::from_secs(3))
            .send()
            .await
        {
            Ok(response) => response.status().is_success(),
            Err(_) => false,
        }
    }

    async fn chat(&self, messages: Vec<ChatMessage>) -> Result<String, AiError> {
        if !self.is_running().await {
            return Err(AiError::ServiceUnavailable("LM Studio 未运行".to_string()));
        }

        let request = ChatRequest {
            model: "default".to_string(),
            messages,
            temperature: 0.7,
        };

        let response = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(AiError::ApiCallFailed(format!(
                "状态码: {}, 响应: {}",
                status, text
            )));
        }

        let chat_response: ChatResponse = response
            .json()
            .await
            .map_err(|e| AiError::ParseError(e.to_string()))?;

        chat_response
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .ok_or_else(|| AiError::ParseError("空响应".to_string()))
    }
}

// ========== AI 服务管理器 ==========

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AiProviderType {
    Doubao,
    OpenAi,
    DeepSeek,
    LmStudio,
}

#[derive(Clone)]
pub struct AiService {
    pub provider_type: AiProviderType,
    pub doubao_api_key: Option<String>,
    pub openai_api_key: Option<String>,
    pub deepseek_api_key: Option<String>,
    pub lm_studio_url: String,
}

impl Default for AiService {
    fn default() -> Self {
        Self {
            provider_type: AiProviderType::LmStudio,
            doubao_api_key: None,
            openai_api_key: None,
            deepseek_api_key: None,
            lm_studio_url: "http://localhost:1234".to_string(),
        }
    }
}

impl AiService {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_provider(&mut self, provider_type: AiProviderType) {
        self.provider_type = provider_type;
    }

    pub fn set_doubao_key(&mut self, api_key: String) {
        self.doubao_api_key = Some(api_key);
    }

    pub fn set_openai_key(&mut self, api_key: String) {
        self.openai_api_key = Some(api_key);
    }

    pub fn set_deepseek_key(&mut self, api_key: String) {
        self.deepseek_api_key = Some(api_key);
    }

    pub fn set_lm_studio_url(&mut self, url: String) {
        self.lm_studio_url = url;
    }

    /// 判断 AI 是否已配置（有可用的 API Key 或 LM Studio）
    pub fn is_configured(&self) -> bool {
        match self.provider_type {
            AiProviderType::Doubao => self.doubao_api_key.as_ref().map_or(false, |k| !k.is_empty()),
            AiProviderType::OpenAi => self.openai_api_key.as_ref().map_or(false, |k| !k.is_empty()),
            AiProviderType::DeepSeek => self.deepseek_api_key.as_ref().map_or(false, |k| !k.is_empty()),
            AiProviderType::LmStudio => true, // LM Studio 始终"已配置"，运行时检测可用性
        }
    }

    /// 检查 LM Studio 是否运行
    pub async fn check_lm_studio(&self) -> bool {
        let provider = LmStudioProvider::new(self.lm_studio_url.clone());
        provider.is_running().await
    }

    /// 调用 AI 对话
    pub async fn chat(&self, messages: Vec<ChatMessage>) -> Result<String, AiError> {
        match self.provider_type {
            AiProviderType::Doubao => {
                let api_key = self.doubao_api_key.clone().ok_or(AiError::InvalidApiKey)?;
                let provider = DoubaoProvider::new(api_key);
                provider.chat(messages).await
            }
            AiProviderType::OpenAi => {
                let api_key = self.openai_api_key.clone().ok_or(AiError::InvalidApiKey)?;
                let provider = OpenAiProvider::new(api_key);
                provider.chat(messages).await
            }
            AiProviderType::DeepSeek => {
                let api_key = self.deepseek_api_key.clone().ok_or(AiError::InvalidApiKey)?;
                let provider = DeepSeekProvider::new(api_key);
                provider.chat(messages).await
            }
            AiProviderType::LmStudio => {
                let provider = LmStudioProvider::new(self.lm_studio_url.clone());
                provider.chat(messages).await
            }
        }
    }
}
