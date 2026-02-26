// AI 服务适配器
// 支持豆包、OpenAI、DeepSeek、LM Studio
// 豆包使用 Responses API（流式 SSE），其他使用 Chat API

use futures_util::StreamExt;
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
    #[error("IO 错误: {0}")]
    IoError(#[from] std::io::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// 聊天附件信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentInfo {
    /// 火山引擎返回的 file_id
    pub file_id: String,
    /// 附件类型: "file" | "image" | "video"
    pub file_type: String,
    /// 原始文件名
    pub file_name: String,
}

/// 流式 chunk 类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamChunk {
    pub session_id: String,
    /// "text" | "thinking"
    pub chunk_type: String,
    pub delta: String,
    pub done: bool,
}

// OpenAI 兼容的请求/响应结构（用于非豆包 Provider）
#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<serde_json::Value>>,
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

/// 豆包 API 提供者 — 使用 Responses API + 流式 SSE
pub struct DoubaoProvider {
    client: Client,
    api_key: String,
    model: String,
    base_url: String,
}

impl DoubaoProvider {
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .unwrap_or_default(),
            api_key,
            model,
            base_url: "https://ark.cn-beijing.volces.com/api/v3".to_string(),
        }
    }

    /// 上传文件到火山引擎，返回 file_id
    pub async fn upload_file(&self, file_path: &str) -> Result<String, AiError> {
        let path = std::path::Path::new(file_path);
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string());

        let file_bytes = tokio::fs::read(path).await?;

        let part = reqwest::multipart::Part::bytes(file_bytes)
            .file_name(file_name)
            .mime_str("application/octet-stream")
            .map_err(|e| AiError::ApiCallFailed(e.to_string()))?;

        let form = reqwest::multipart::Form::new()
            .text("purpose", "user_data")
            .part("file", part);

        let response = self
            .client
            .post(format!("{}/files", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .multipart(form)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(AiError::ApiCallFailed(format!(
                "文件上传失败 - 状态码: {}, 响应: {}",
                status, text
            )));
        }

        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AiError::ParseError(e.to_string()))?;

        json["id"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| AiError::ParseError("文件上传返回中缺少 id 字段".to_string()))
    }

    /// 构建多模态 input 数组
    fn build_input(
        messages: &[ChatMessage],
        attachments: &[AttachmentInfo],
    ) -> serde_json::Value {
        let mut input_items: Vec<serde_json::Value> = Vec::new();

        // 将附件转为对应的 input_xxx 类型
        for att in attachments {
            let item = match att.file_type.as_str() {
                "image" => serde_json::json!({
                    "type": "input_image",
                    "file_id": att.file_id,
                }),
                "video" => serde_json::json!({
                    "type": "input_video",
                    "file_id": att.file_id,
                }),
                _ => serde_json::json!({
                    "type": "input_file",
                    "file_id": att.file_id,
                }),
            };
            input_items.push(item);
        }

        // 将所有消息以 role+content 格式添加
        for msg in messages {
            input_items.push(serde_json::json!({
                "role": msg.role,
                "content": [{
                    "type": "input_text",
                    "text": msg.content,
                }],
            }));
        }

        serde_json::Value::Array(input_items)
    }

    /// 流式对话（豆包 Responses API + SSE）
    /// callback 用于实时回传 chunk，由调用方决定如何推送给前端
    pub async fn chat_stream<F>(
        &self,
        messages: Vec<ChatMessage>,
        attachments: Vec<AttachmentInfo>,
        enable_web_search: bool,
        thinking_depth: Option<String>,
        session_id: String,
        mut callback: F,
    ) -> Result<String, AiError>
    where
        F: FnMut(StreamChunk) + Send,
    {
        let input = if attachments.is_empty() {
            // 无附件时，简化为消息数组（兼容原有逻辑）
            serde_json::to_value(&messages).unwrap_or_default()
        } else {
            Self::build_input(&messages, &attachments)
        };

        let mut payload = serde_json::json!({
            "model": self.model,
            "input": input,
            "temperature": 0.7,
            "stream": true,
        });

        if let Some(re) = &thinking_depth {
            if re != "none" {
                payload["reasoning"] = serde_json::json!({ "effort": re });
            }
        }

        if enable_web_search {
            payload["tools"] = serde_json::json!([{"type": "web_search"}]);
        }

        tracing::info!("豆包流式请求开始，model: {}", self.model);

        let response = self
            .client
            .post(format!("{}/responses", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&payload)
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

        // 以 SSE 方式逐行读取
        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut full_text = String::new();
        let mut full_thinking = String::new();

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.map_err(|e| AiError::NetworkError(e))?;
            let text = String::from_utf8_lossy(&chunk);
            buffer.push_str(&text);

            // 按行处理 SSE
            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim().to_string();
                buffer = buffer[pos + 1..].to_string();

                if line.is_empty() {
                    continue;
                }

                if line == "data: [DONE]" {
                    // 流式结束
                    callback(StreamChunk {
                        session_id: session_id.clone(),
                        chunk_type: "text".to_string(),
                        delta: String::new(),
                        done: true,
                    });
                    break;
                }

                if let Some(data) = line.strip_prefix("data: ") {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                        if let Some(event_type) = json["type"].as_str() {
                            match event_type {
                                "response.output_text.delta" => {
                                    if let Some(delta) = json["delta"].as_str() {
                                        full_text.push_str(delta);
                                        callback(StreamChunk {
                                            session_id: session_id.clone(),
                                            chunk_type: "text".to_string(),
                                            delta: delta.to_string(),
                                            done: false,
                                        });
                                    }
                                }
                                "response.reasoning_summary_text.delta" => {
                                    if let Some(delta) = json["delta"].as_str() {
                                        full_thinking.push_str(delta);
                                        callback(StreamChunk {
                                            session_id: session_id.clone(),
                                            chunk_type: "thinking".to_string(),
                                            delta: delta.to_string(),
                                            done: false,
                                        });
                                    }
                                }
                                "response.completed" => {
                                    callback(StreamChunk {
                                        session_id: session_id.clone(),
                                        chunk_type: "text".to_string(),
                                        delta: String::new(),
                                        done: true,
                                    });
                                }
                                _ => {
                                    // 其他事件类型忽略
                                }
                            }
                        }
                    }
                }
            }
        }

        // 组合最终完整文本（包含 thinking 标签）
        let mut result = String::new();
        let rt = full_thinking.trim();
        if !rt.is_empty() {
            result.push_str("<think>\n");
            result.push_str(rt);
            result.push_str("\n</think>\n\n");
        }
        result.push_str(full_text.trim());

        Ok(result)
    }

    /// 非流式对话（保留兼容，当不需要流式时使用）
    pub async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        enable_web_search: bool,
        thinking_depth: Option<String>,
    ) -> Result<String, AiError> {
        let request = ChatRequest {
            model: self.model.clone(),
            messages,
            temperature: 0.7,
            reasoning_effort: thinking_depth,
            tools: if enable_web_search {
                Some(vec![serde_json::json!({"type": "web_search"})])
            } else {
                None
            },
        };
        self.send_request(&request).await
    }

    async fn send_request(&self, request: &ChatRequest) -> Result<String, AiError> {
        let (endpoint, payload) = if request.tools.is_some() {
            let mut payload = serde_json::json!({
                "model": request.model,
                "input": request.messages,
                "temperature": request.temperature,
                "tools": request.tools
            });
            if let Some(re) = &request.reasoning_effort {
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert(
                        "reasoning".to_string(),
                        serde_json::json!({ "effort": re }),
                    );
                }
            }
            ("responses", payload)
        } else {
            let payload = serde_json::to_value(request).unwrap_or_default();
            ("chat/completions", payload)
        };

        let response = self
            .client
            .post(format!("{}/{}", self.base_url, endpoint))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .timeout(std::time::Duration::from_secs(60))
            .json(&payload)
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

        let response_json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AiError::ParseError(e.to_string()))?;

        if endpoint == "responses" {
            if let Some(output_arr) = response_json["output"].as_array() {
                let mut reasoning_text = String::new();
                let mut content_text = String::new();

                for item in output_arr {
                    if let Some(item_type) = item["type"].as_str() {
                        if item_type == "reasoning" {
                            if let Some(summary_arr) = item["summary"].as_array() {
                                for sum in summary_arr {
                                    if sum["type"].as_str() == Some("summary_text") {
                                        if let Some(text) = sum["text"].as_str() {
                                            reasoning_text.push_str(text);
                                        }
                                    }
                                }
                            }
                        } else if item_type == "message" {
                            if let Some(content_arr) = item["content"].as_array() {
                                for cont in content_arr {
                                    if cont["type"].as_str() == Some("output_text") {
                                        if let Some(text) = cont["text"].as_str() {
                                            content_text.push_str(text);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                let mut final_text = String::new();
                let rt = reasoning_text.trim();
                if !rt.is_empty() {
                    final_text.push_str("<think>\n");
                    final_text.push_str(rt);
                    final_text.push_str("\n</think>\n\n");
                }
                final_text.push_str(content_text.trim());

                if !final_text.is_empty() {
                    return Ok(final_text);
                }
            }
            Err(AiError::ParseError(format!(
                "无法从 Responses API 解析内容: {}",
                response_json
            )))
        } else {
            if let Some(choices) =
                response_json["choices"]
                    .as_array()
                    .and_then(|arr| arr.first())
            {
                if let Some(content) = choices["message"]["content"].as_str() {
                    return Ok(content.to_string());
                }
            }
            Err(AiError::ParseError(format!(
                "无法从 Chat API 解析内容: {}",
                response_json
            )))
        }
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

    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        _enable_web_search: bool,
        _thinking_depth: Option<String>,
    ) -> Result<String, AiError> {
        let request = ChatRequest {
            model: self.model.clone(),
            messages,
            temperature: 0.7,
            reasoning_effort: None,
            tools: None,
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

    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        _enable_web_search: bool,
        _thinking_depth: Option<String>,
    ) -> Result<String, AiError> {
        let request = ChatRequest {
            model: self.model.clone(),
            messages,
            temperature: 0.7,
            reasoning_effort: None,
            tools: None,
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

    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        _enable_web_search: bool,
        _thinking_depth: Option<String>,
    ) -> Result<String, AiError> {
        if !self.is_running().await {
            return Err(AiError::ServiceUnavailable("LM Studio 未运行".to_string()));
        }

        let request = ChatRequest {
            model: "default".to_string(),
            messages,
            temperature: 0.7,
            reasoning_effort: None,
            tools: None,
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
    pub doubao_model: Option<String>,
    pub openai_api_key: Option<String>,
    pub deepseek_api_key: Option<String>,
    pub lm_studio_url: String,
}

impl Default for AiService {
    fn default() -> Self {
        Self {
            provider_type: AiProviderType::LmStudio,
            doubao_api_key: None,
            doubao_model: Some("doubao-seed-2-0-pro-260215".to_string()),
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

    pub fn set_doubao_model(&mut self, model: String) {
        self.doubao_model = Some(model);
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

    /// 判断 AI 是否已配置
    pub fn is_configured(&self) -> bool {
        match self.provider_type {
            AiProviderType::Doubao => self
                .doubao_api_key
                .as_ref()
                .map_or(false, |k| !k.is_empty()),
            AiProviderType::OpenAi => self
                .openai_api_key
                .as_ref()
                .map_or(false, |k| !k.is_empty()),
            AiProviderType::DeepSeek => self
                .deepseek_api_key
                .as_ref()
                .map_or(false, |k| !k.is_empty()),
            AiProviderType::LmStudio => true,
        }
    }

    /// 检查 LM Studio 是否运行
    pub async fn check_lm_studio(&self) -> bool {
        let provider = LmStudioProvider::new(self.lm_studio_url.clone());
        provider.is_running().await
    }

    /// 上传文件到 AI（豆包专用）
    pub async fn upload_file(&self, file_path: &str) -> Result<String, AiError> {
        match self.provider_type {
            AiProviderType::Doubao => {
                let api_key = self.doubao_api_key.clone().ok_or(AiError::InvalidApiKey)?;
                let model = self
                    .doubao_model
                    .clone()
                    .unwrap_or_else(|| "doubao-seed-2-0-pro-260215".to_string());
                let provider = DoubaoProvider::new(api_key, model);
                provider.upload_file(file_path).await
            }
            _ => Err(AiError::ApiCallFailed(
                "当前 AI 提供商不支持文件上传".to_string(),
            )),
        }
    }

    /// 流式对话（豆包使用 SSE，其他 Provider 降级为非流式）
    pub async fn chat_stream<F>(
        &self,
        messages: Vec<ChatMessage>,
        attachments: Vec<AttachmentInfo>,
        enable_web_search: bool,
        thinking_depth: Option<String>,
        session_id: String,
        callback: F,
    ) -> Result<String, AiError>
    where
        F: FnMut(StreamChunk) + Send,
    {
        tracing::info!(
            "--- AI 流式调用，提供者: {:?} ---",
            self.provider_type
        );
        match self.provider_type {
            AiProviderType::Doubao => {
                let api_key = self.doubao_api_key.clone().ok_or(AiError::InvalidApiKey)?;
                let model = self
                    .doubao_model
                    .clone()
                    .unwrap_or_else(|| "doubao-seed-2-0-pro-260215".to_string());
                tracing::info!("豆包流式模型: {}", model);
                let provider = DoubaoProvider::new(api_key, model);
                provider
                    .chat_stream(
                        messages,
                        attachments,
                        enable_web_search,
                        thinking_depth,
                        session_id,
                        callback,
                    )
                    .await
            }
            _ => {
                // 其他 Provider 降级为非流式
                let result = self
                    .chat(messages, enable_web_search, thinking_depth)
                    .await?;
                Ok(result)
            }
        }
    }

    /// 非流式对话（兼容所有 Provider）
    pub async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        enable_web_search: bool,
        thinking_depth: Option<String>,
    ) -> Result<String, AiError> {
        tracing::info!(
            "--- 正在调用 AI 服务，提供者: {:?} ---",
            self.provider_type
        );
        match self.provider_type {
            AiProviderType::Doubao => {
                let api_key = self.doubao_api_key.clone().ok_or(AiError::InvalidApiKey)?;
                let model = self
                    .doubao_model
                    .clone()
                    .unwrap_or_else(|| "doubao-seed-2-0-pro-260215".to_string());
                tracing::info!(
                    "豆包模型: {}, API Base: https://ark.cn-beijing.volces.com/api/v3",
                    model
                );
                let provider = DoubaoProvider::new(api_key, model);
                provider
                    .chat(messages, enable_web_search, thinking_depth)
                    .await
            }
            AiProviderType::OpenAi => {
                let api_key = self.openai_api_key.clone().ok_or(AiError::InvalidApiKey)?;
                let provider = OpenAiProvider::new(api_key);
                provider
                    .chat(messages, enable_web_search, thinking_depth)
                    .await
            }
            AiProviderType::DeepSeek => {
                let api_key = self.deepseek_api_key.clone().ok_or(AiError::InvalidApiKey)?;
                let provider = DeepSeekProvider::new(api_key);
                provider
                    .chat(messages, enable_web_search, thinking_depth)
                    .await
            }
            AiProviderType::LmStudio => {
                let provider = LmStudioProvider::new(self.lm_studio_url.clone());
                provider
                    .chat(messages, enable_web_search, thinking_depth)
                    .await
            }
        }
    }
}
