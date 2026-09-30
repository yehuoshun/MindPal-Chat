use crate::types::ChatMessage;
use rusqlite::{params, Connection};
use std::path::Path;

/// 初始化数据库（表 + 索引），幂等
pub fn init(db_path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(db_path).map_err(|e| format!("打开数据库失败: {e}"))?;
    init_schema(&conn)?;
    Ok(conn)
}

/// 建表（幂等），独立出来便于内存库单测
pub fn init_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
        CREATE TABLE IF NOT EXISTS conversations (
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
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn new_conversation_increments_id() {
        let conn = test_conn();
        let a = new_conversation(&conn, "新对话").unwrap();
        let b = new_conversation(&conn, "新对话").unwrap();
        assert!(b > a);
    }

    #[test]
    fn save_and_load_messages_roundtrip() {
        let conn = test_conn();
        let id = new_conversation(&conn, "测试会话").unwrap();
        save_message(&conn, id, "user", "你好").unwrap();
        save_message(&conn, id, "assistant", "嗨").unwrap();
        let msgs = load_conversation(&conn, id).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content, "你好");
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[1].content, "嗨");
    }

    #[test]
    fn first_user_message_becomes_title_truncated() {
        let conn = test_conn();
        let id = new_conversation(&conn, "新对话").unwrap();
        let long = "一二三四五六七八九十一二三四五六七八九十超出部分啊";
        save_message(&conn, id, "user", long).unwrap();
        let convs = list_conversations(&conn).unwrap();
        assert_eq!(convs.len(), 1);
        assert_eq!(convs[0].title, "一二三四五六七八九十一二三四五六七八九十超出部分…");
    }

    #[test]
    fn assistant_first_message_keeps_default_title() {
        let conn = test_conn();
        let id = new_conversation(&conn, "新对话").unwrap();
        save_message(&conn, id, "assistant", "你好呀").unwrap();
        let convs = list_conversations(&conn).unwrap();
        assert_eq!(convs[0].title, "新对话");
    }

    #[test]
    fn list_sorted_by_updated_at_desc() {
        let conn = test_conn();
        let a = new_conversation(&conn, "A").unwrap();
        let b = new_conversation(&conn, "B").unwrap();
        // 睡 1.1s 确保时间戳严格递增，避免同一秒排序平局
        std::thread::sleep(std::time::Duration::from_millis(1100));
        save_message(&conn, a, "user", "后发的消息").unwrap();
        let convs = list_conversations(&conn).unwrap();
        assert_eq!(convs[0].id, a);
        assert_eq!(convs[1].id, b);
    }

    #[test]
    fn delete_conversation_cascades_messages() {
        let conn = test_conn();
        let id = new_conversation(&conn, "要删的").unwrap();
        save_message(&conn, id, "user", "内容").unwrap();
        delete_conversation(&conn, id).unwrap();
        assert!(load_conversation(&conn, id).unwrap().is_empty());
        assert!(list_conversations(&conn).unwrap().is_empty());
    }

    #[test]
    fn rename_updates_title() {
        let conn = test_conn();
        let id = new_conversation(&conn, "旧标题").unwrap();
        rename_conversation(&conn, id, "新标题").unwrap();
        assert_eq!(list_conversations(&conn).unwrap()[0].title, "新标题");
    }
}
