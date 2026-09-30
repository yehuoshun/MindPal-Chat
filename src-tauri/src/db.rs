use crate::types::ChatMessage;
use rusqlite::{params, Connection};
use std::path::Path;

/// 初始化数据库（表 + 索引），幂等
pub fn init(db_path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(db_path).map_err(|e| format!("打开数据库失败: {e}"))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS conversations (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            title      TEXT NOT NULL DEFAULT '新对话',
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS messages (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
            role            TEXT NOT NULL,
            content         TEXT NOT NULL,
            created_at      INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_messages_conv ON messages(conversation_id, id);",
    )
    .map_err(|e| format!("初始化表结构失败: {e}"))?;
    Ok(conn)
}

pub fn new_conversation(conn: &Connection, title: &str) -> Result<i64, String> {
    let now = chrono_now();
    conn.execute(
        "INSERT INTO conversations (title, created_at, updated_at) VALUES (?1, ?2, ?2)",
        params![title, now],
    )
    .map_err(|e| format!("创建会话失败: {e}"))?;
    Ok(conn.last_insert_rowid())
}

pub fn list_conversations(conn: &Connection) -> Result<Vec<super::types::ConversationSummary>, String> {
    let mut stmt = conn
        .prepare("SELECT id, title, updated_at FROM conversations ORDER BY updated_at DESC")
        .map_err(|e| format!("查询会话失败: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(super::types::ConversationSummary {
                id: row.get(0)?,
                title: row.get(1)?,
                updated_at: row.get(2)?,
            })
        })
        .map_err(|e| format!("查询会话失败: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("读取会话失败: {e}"))?);
    }
    Ok(out)
}

pub fn load_conversation(conn: &Connection, id: i64) -> Result<Vec<ChatMessage>, String> {
    let mut stmt = conn
        .prepare("SELECT role, content FROM messages WHERE conversation_id = ?1 ORDER BY id")
        .map_err(|e| format!("查询消息失败: {e}"))?;
    let rows = stmt
        .query_map(params![id], |row| {
            Ok(ChatMessage {
                role: row.get(0)?,
                content: row.get(1)?,
            })
        })
        .map_err(|e| format!("查询消息失败: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("读取消息失败: {e}"))?);
    }
    Ok(out)
}

pub fn delete_conversation(conn: &Connection, id: i64) -> Result<(), String> {
    conn.execute("DELETE FROM conversations WHERE id = ?1", params![id])
        .map_err(|e| format!("删除会话失败: {e}"))?;
    Ok(())
}

pub fn rename_conversation(conn: &Connection, id: i64, title: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE conversations SET title = ?1, updated_at = ?2 WHERE id = ?3",
        params![title, chrono_now(), id],
    )
    .map_err(|e| format!("重命名会话失败: {e}"))?;
    Ok(())
}

/// 保存一条消息；若会话标题还是默认「新对话」且是用户消息，则用消息前 24 字当标题
pub fn save_message(conn: &Connection, conversation_id: i64, role: &str, content: &str) -> Result<i64, String> {
    let now = chrono_now();
    conn.execute(
        "INSERT INTO messages (conversation_id, role, content, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![conversation_id, role, content, now],
    )
    .map_err(|e| format!("保存消息失败: {e}"))?;
    let id = conn.last_insert_rowid();

    if role == "user" {
        let is_default_title: bool = conn
            .query_row(
                "SELECT title = '新对话' FROM conversations WHERE id = ?1",
                params![conversation_id],
                |row| row.get(0),
            )
            .unwrap_or(false);
        if is_default_title {
            let mut title: String = content.chars().take(24).collect();
            if content.chars().count() > 24 {
                title.push('…');
            }
            let _ = conn.execute(
                "UPDATE conversations SET title = ?1 WHERE id = ?2",
                params![title, conversation_id],
            );
        }
    }

    conn.execute(
        "UPDATE conversations SET updated_at = ?1 WHERE id = ?2",
        params![now, conversation_id],
    )
    .map_err(|e| format!("更新会话时间失败: {e}"))?;
    Ok(id)
}

/// 无第三方依赖的秒级时间戳（UTC）
fn chrono_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
