use crate::ai::knowledge_base::KnowledgeBase;
use crate::ai::service::{AiProviderType, AiService, ChatMessage};
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};
use tracing::{info, warn};

// 全局内存总结通道
pub struct MemoryManager {
    kb: Arc<Mutex<Option<KnowledgeBase>>>,
    ai_service: Arc<Mutex<AiService>>,
    sender: mpsc::Sender<MemoryTask>,
}

pub enum MemoryTask {
    /// 执行一次会话压缩检查
    TrySummarize { session_id: String },
    /// 用户发送了新消息，请求打断/重设空闲计时器
    UserActive { session_id: String },
    /// 触发全局用户画像提取分析
    TryExtractProfile { session_id: String },
}

impl MemoryManager {
    /// 初始化并启动后台监听死循环
    pub fn new(kb: Arc<Mutex<Option<KnowledgeBase>>>, ai_service: Arc<Mutex<AiService>>) -> Self {
        let (tx, mut rx) = mpsc::channel::<MemoryTask>(32);
        
        let kb_clone = kb.clone();
        let ai_service_clone = ai_service.clone();

        tokio::spawn(async move {
            let mut summary_pending: Option<String> = None;
            let mut idle_timer = Box::pin(sleep(Duration::from_secs(86400))); // 极长初始等待

            loop {
                tokio::select! {
                    msg = rx.recv() => {
                        match msg {
                            Some(MemoryTask::TrySummarize { session_id }) => {
                                // 收到总结请求，记录之
                                summary_pending = Some(session_id.clone());
                                // 检查当前 Provider 类型
                                let provider_type = ai_service_clone.lock().provider_type.clone();
                                
                                if provider_type == AiProviderType::LmStudio {
                                    // 本地模型：防抖，倒计时 15 秒（模拟闲置）
                                    info!("本地模型收到总结请求，进入 15 秒闲时防抖期...");
                                    idle_timer = Box::pin(sleep(Duration::from_secs(15)));
                                } else {
                                    // 云端 API：并发激进模式，无视卡顿直接抛出执行
                                    info!("云端大模型收到总结请求，立即并发执行！");
                                    Self::execute_summarize(kb_clone.clone(), ai_service_clone.clone(), session_id).await;
                                    summary_pending = None; // 清除等待状态
                                    idle_timer = Box::pin(sleep(Duration::from_secs(86400))); // 重设长等待
                                }
                            }
                            Some(MemoryTask::UserActive { session_id }) => {
                                // 用户活动，打断并重置本地防抖
                                if let Some(ref pending_sid) = summary_pending {
                                    if pending_sid == &session_id {
                                        let provider_type = ai_service_clone.lock().provider_type.clone();
                                        if provider_type == AiProviderType::LmStudio {
                                            info!("检测到用户正在输入活跃，打断重置本地模型总结倒计时...");
                                            idle_timer = Box::pin(sleep(Duration::from_secs(15)));
                                        }
                                    }
                                }
                            }
                            Some(MemoryTask::TryExtractProfile { session_id }) => {
                                info!("收到画像提取分析请求: {}", session_id);
                                Self::execute_extract_profile(kb_clone.clone(), ai_service_clone.clone(), session_id).await;
                            }
                            None => {
                                // 通道关闭
                                break;
                            }
                        }
                    }
                    _ = &mut idle_timer => {
                        // 倒计时结束，执行积压的任务
                        if let Some(session_id) = summary_pending.take() {
                            info!("闲时倒计时结束，开始静默执行后台总结任务...");
                            Self::execute_summarize(kb_clone.clone(), ai_service_clone.clone(), session_id).await;
                        }
                        // 设回无限长
                        idle_timer = Box::pin(sleep(Duration::from_secs(86400)));
                    }
                }
            }
        });

        Self {
            kb,
            ai_service,
            sender: tx,
        }
    }

    pub async fn trigger_check(&self, session_id: String) {
        let _ = self.sender.send(MemoryTask::UserActive { session_id: session_id.clone() }).await;
        // 如果想要每次用户说话都触发“尝试总结”，可紧接着发一条
        let _ = self.sender.send(MemoryTask::TrySummarize { session_id }).await;
    }

    pub async fn trigger_profile_extraction(&self, session_id: String) {
        let _ = self.sender.send(MemoryTask::TryExtractProfile { session_id }).await;
    }

    /// 执行真正的提取与压缩逻辑
    async fn execute_summarize(kb_mtx: Arc<Mutex<Option<KnowledgeBase>>>, ai_service_mtx: Arc<Mutex<AiService>>, session_id: String) {
        let kb_opt = kb_mtx.lock().clone();
        if let Some(kb) = kb_opt {
            // 1. 读取该 session 所有的聊天记录（为简化逻辑，先拉回来判断）
            if let Ok(msgs) = kb.load_messages(&session_id).await {
                // 如果聊天轮数太少，没必要总结（例如总对话不超过 6 条(3轮)）
                if msgs.len() < 8 {
                    info!("会话 {} 消息数 {} < 8，跳过中期总结", session_id, msgs.len());
                    return;
                }

                // 取前多少条或所有历史进行判断，理想状况下应该取 "除去最近 4 条之外的所有历史" 作为基础
                // 或者直接拉取之前的概要 + 最近新增的 N 条做融合。
                // 此处简化：将除了最后 2 轮（4条）之外的历史拼接起来，要求大模型进行总结。
                let summarize_target_msgs: Vec<_> = msgs.iter().take(msgs.len() - 4).collect();
                
                // 为了防止重复压缩，这里本应比对 last_summarized_msg_id。
                // 获取上一个总结点，如果最后一条没变，就不重做。
                let last_id = &summarize_target_msgs.last().unwrap().0;
                // 这部分判断可以通过 SQLite 扩展读取，这里简化演示。

                let content_to_summarize = summarize_target_msgs
                    .iter()
                    .map(|(_, _, role, content, _)| format!("[{}]: {}", role, content))
                    .collect::<Vec<_>>()
                    .join("\n");

                let prompt = format!(
                    "【请执行对话压缩任务】\n请将以下对话浓缩为一段极简的摘要（字数控制在150字以内）。\n\
                     请使用第三人称（如“用户询问了...，AI回答了...”），提取出此段对话中最核心的技术事实、讨论决策和关键参数。\n\
                     过滤掉寒暄和无用的中间过程。\n\
                     如果对话没有太多实质内容，请直接回复【无】。\n\n\
                     以下是需要总结的对话：\n{}", 
                    content_to_summarize
                );

                let messages = vec![
                    ChatMessage {
                        role: "system".to_string(),
                        content: "你是一个专业的 AI 记忆管理助手。你负责从对话历史中提取核心事实并进行极简总结。请严格遵守字数要求，禁止直接回答对话中的问题。".to_string(),
                    },
                    ChatMessage {
                        role: "user".to_string(),
                        content: prompt,
                    }
                ];

                let ai = ai_service_mtx.lock().clone();
                // 强制不调用联网等复杂操作
                info!(">> 开始请求大模型生成内部摘要...");
                match ai.chat(messages, false, None).await {
                    Ok(summary_res) => {
                        let result = summary_res.trim();
                        if !result.is_empty() && result != "【无】" && result != "无" {
                            // 更新到 SQLite
                            if let Err(e) = kb.update_conversation_summary(&session_id, result, last_id).await {
                                warn!("更新会话 {} 摘要失败: {}", session_id, e);
                            } else {
                                info!("✅ 中期总结生成并入库成功: {}", result);
                            }
                        }
                    }
                    Err(e) => {
                        warn!("后台总结任务失败: {}", e);
                    }
                }
            }
        }
    }

    /// 执行全局用户画像提取
    async fn execute_extract_profile(kb_mtx: Arc<Mutex<Option<KnowledgeBase>>>, ai_service_mtx: Arc<Mutex<AiService>>, session_id: String) {
        let kb_opt = kb_mtx.lock().clone();
        if let Some(kb) = kb_opt {
            if let Ok(msgs) = kb.load_messages(&session_id).await {
                // 取最近的部分对话来进行用户设定挖掘
                let recent_msgs: Vec<_> = msgs.into_iter().rev().take(10).rev().collect();
                if recent_msgs.is_empty() { return; }

                let content_to_extract = recent_msgs
                    .iter()
                    .map(|(_, _, role, content, _)| format!("[{}]: {}", role, content))
                    .collect::<Vec<_>>()
                    .join("\n");

                let prompt = format!(
                    "【请执行用户画像提取任务】\n分析以下用户的近期对话，提取出明确的“用户身份特征”、“持久化偏好”或“强制性格式要求”。\n\
                     例如：用户是一名Rust程序员、用户要求回复一律用英文、用户偏好极简代码风格等。\n\
                     如果没有发现任何明显的长期特征或偏好，请直接回复【无】。\n\
                     如果有，请用一句话高度概括提取（字数控制在50字以内）。\n\n\
                     以下是近期对话：\n{}", 
                    content_to_extract
                );

                let messages = vec![
                    ChatMessage {
                        role: "system".to_string(),
                        content: "你是一个专业的 AI 观察员。你负责分析对话并提取用户的长期偏好、职业身份或特定回复格式。请以客观第三人称进行极简概括。".to_string(),
                    },
                    ChatMessage {
                        role: "user".to_string(),
                        content: prompt,
                    }
                ];

                let ai = ai_service_mtx.lock().clone();
                match ai.chat(messages, false, None).await {
                    Ok(extracted) => {
                        let result = extracted.trim();
                        if !result.is_empty() && result != "【无】" && result != "无" {
                            // 简单起见，以时间戳为Key写入 profile
                            let timestamp_key = chrono::Utc::now().timestamp().to_string();
                            if let Err(e) = kb.save_user_profile(&format!("profile_{}", timestamp_key), result).await {
                                warn!("写入全局用户画像失败: {}", e);
                            } else {
                                info!("⭐ 成功提取并注入新用户画像: {}", result);
                            }
                        }
                    }
                    Err(e) => {
                        warn!("后台画像提取失败: {}", e);
                    }
                }
            }
        }
    }
}
