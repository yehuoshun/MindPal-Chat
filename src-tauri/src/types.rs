use serde::{Deserialize, Serialize};

/// 一条聊天消息，与前端 ChatMessage 对应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// 会话列表项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSummary {
    pub id: i64,
    pub title: String,
    pub updated_at: i64,
}
