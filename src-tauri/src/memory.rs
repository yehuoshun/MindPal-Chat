use rusqlite::{params, params_from_iter, Connection};
use serde::Serialize;

/// 记忆条目（列表展示 / 检索命中共用）
#[derive(Debug, Clone, Serialize)]
pub struct MemoryItem {
    pub id: i64,
    pub content: String,
    pub kind: String, // "chat" = 对话记忆 | "fact" = 用户画像事实
    pub created_at: i64,
}

/// 建表（幂等），由 db::init_schema 调用
pub fn init_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS memories (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            content    TEXT NOT NULL,
            kind       TEXT NOT NULL DEFAULT 'chat',
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_memories_kind ON memories(kind);",
    )
    .map_err(|e| format!("初始化记忆表失败: {e}"))?;
    Ok(())
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 写入一条记忆
pub fn add_memory(conn: &Connection, content: &str, kind: &str) -> Result<i64, String> {
    conn.execute(
        "INSERT INTO memories (content, kind, created_at) VALUES (?1, ?2, ?3)",
        params![content, kind, now()],
    )
    .map_err(|e| format!("写入记忆失败: {e}"))?;
    Ok(conn.last_insert_rowid())
}

/// 全部记忆（新→旧）
pub fn list_memories(conn: &Connection) -> Result<Vec<MemoryItem>, String> {
    let mut stmt = conn
        .prepare("SELECT id, content, kind, created_at FROM memories ORDER BY id DESC")
        .map_err(|e| format!("查询记忆失败: {e}"))?;
    let rows = stmt
        .query_map([], map_item)
        .map_err(|e| format!("查询记忆失败: {e}"))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("读取记忆失败: {e}"))?);
    }
    Ok(out)
}

/// 关键词检索（中文拆词 + LIKE 多词 OR；数据量小，全扫毫秒级）
pub fn search_memories(conn: &Connection, query: &str, limit: usize) -> Result<Vec<MemoryItem>, String> {
    let terms = tokenize(query);
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let limit = limit.clamp(1, 50);
    let conds = vec!["content LIKE '%' || ? || '%'"; terms.len()].join(" OR ");
    let sql = format!("SELECT id, content, kind, created_at FROM memories WHERE {conds} ORDER BY id DESC LIMIT {limit}");
    let mut stmt = conn.prepare(&sql).map_err(|e| format!("检索记忆失败: {e}"))?;
    let rows = stmt
        .query_map(params_from_iter(terms.iter()), map_item)
        .map_err(|e| format!("检索记忆失败: {e}"))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("读取记忆失败: {e}"))?);
    }
    Ok(out)
}

fn map_item(row: &rusqlite::Row) -> rusqlite::Result<MemoryItem> {
    Ok(MemoryItem {
        id: row.get(0)?,
        content: row.get(1)?,
        kind: row.get(2)?,
        created_at: row.get(3)?,
    })
}

/// 删除单条记忆
pub fn delete_memory(conn: &Connection, id: i64) -> Result<(), String> {
    conn.execute("DELETE FROM memories WHERE id = ?1", params![id])
        .map_err(|e| format!("删除记忆失败: {e}"))?;
    Ok(())
}

/// 清空全部记忆
pub fn clear_memories(conn: &Connection) -> Result<(), String> {
    conn.execute("DELETE FROM memories", [])
        .map_err(|e| format!("清空记忆失败: {e}"))?;
    Ok(())
}

/// 同内容事实是否已存在（防重复堆积）
pub fn fact_exists(conn: &Connection, content: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM memories WHERE kind = 'fact' AND content = ?1)",
        params![content],
        |row| row.get(0),
    )
    .map_err(|e| format!("查询事实失败: {e}"))
}

/// 规则提取用户画像事实（我叫/我喜欢/我在…工作/我今年X岁 等）
pub fn extract_facts(content: &str) -> Vec<String> {
    let mut facts: Vec<String> = Vec::new();
    let patterns: &[(&str, &str)] = &[
        (r"我(?:名字)?叫([\p{Han}A-Za-z0-9]{1,12})", "用户名字叫{1}"),
        (r"我喜欢([\p{Han}A-Za-z0-9]{1,20}?)(?:[，。；、！？!?\s]|$)", "用户喜欢{1}"),
        (r"我爱([\p{Han}A-Za-z0-9]{1,20}?)(?:[，。；、！？!?\s]|$)", "用户喜欢{1}"),
        (r"我不喜欢([\p{Han}A-Za-z0-9]{1,20}?)(?:[，。；、！？!?\s]|$)", "用户不喜欢{1}"),
        (r"我住在([\p{Han}A-Za-z0-9]{1,20}?)(?:[，。；、！？!?\s]|$)", "用户住在{1}"),
        (r"我在([\p{Han}A-Za-z0-9]{1,15}?)工作", "用户在{1}工作"),
        (r"我是做([\p{Han}A-Za-z0-9]{1,20}?)(?:的|的[，。；、！？!?\s]|$)", "用户职业是{1}"),
        (r"我今年(\d{1,3})岁", "用户今年{1}岁"),
    ];
    for (pat, tmpl) in patterns {
        if let Ok(re) = regex::Regex::new(pat) {
            for cap in re.captures_iter(content) {
                let val = cap.get(1).map(|m| m.as_str().trim()).unwrap_or("");
                if val.is_empty() {
                    continue;
                }
                let fact = tmpl.replace("{1}", val);
                // 前缀去重：已存在同主题（如已记过"用户喜欢X"）则跳过
                let prefix = fact.chars().take(4).collect::<String>();
                if facts.iter().any(|f| f.starts_with(&prefix)) {
                    continue;
                }
                facts.push(fact);
            }
        }
    }
    facts
}

/// 检索记忆并格式化为注入 LLM 的上下文；无命中返回空串
pub fn build_memory_context(conn: &Connection, query: &str, limit: usize) -> Result<String, String> {
    let hits = search_memories(conn, query, limit)?;
    if hits.is_empty() {
        return Ok(String::new());
    }
    let mut lines = vec!["[来自记忆的上下文：以下信息来自过往对话，请自然地使用它们]".to_string()];
    for h in hits {
        lines.push(format!("- {}", h.content));
    }
    Ok(lines.join("\n"))
}

/// 中文拆词：按非字母数字字符切分，去重保序
pub fn tokenize(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for c in s.chars() {
        if c.is_alphanumeric() {
            cur.push(c);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out.sort();
    out.dedup();
    out
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
    fn tokenize_splits_chinese_and_ascii() {
        let t = tokenize("我喜欢喝咖啡 and coding 123");
        assert_eq!(t, vec!["123", "and", "coding", "我喜欢喝咖啡"]);
        assert!(tokenize("!!!@@@   ").is_empty());
    }

    #[test]
    fn search_hits_chinese_substring() {
        let conn = test_conn();
        add_memory(&conn, "用户喜欢喝咖啡，每天两杯", "chat").unwrap();
        add_memory(&conn, "用户在做 MindPal 项目", "chat").unwrap();
        let hits = search_memories(&conn, "咖啡", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].content.contains("咖啡"));
    }

    #[test]
    fn search_multi_term_or() {
        let conn = test_conn();
        add_memory(&conn, "用户喜欢听摇滚乐", "chat").unwrap();
        add_memory(&conn, "用户养了一只橘猫", "chat").unwrap();
        let hits = search_memories(&conn, "摇滚 橘猫", 10).unwrap();
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn search_no_match_or_special_chars() {
        let conn = test_conn();
        add_memory(&conn, "用户住在上海", "chat").unwrap();
        assert!(search_memories(&conn, "北京", 10).unwrap().is_empty());
        // 特殊字符不 panic，返回空
        assert!(search_memories(&conn, "!!!@@@", 10).unwrap().is_empty());
    }

    #[test]
    fn extract_facts_multiple() {
        let facts = extract_facts("我叫小明，我喜欢喝咖啡，我在北京工作，我今年28岁");
        assert!(facts.contains(&"用户名字叫小明".to_string()));
        assert!(facts.contains(&"用户喜欢喝咖啡".to_string()));
        assert!(facts.contains(&"用户在北京工作".to_string()));
        assert!(facts.contains(&"用户今年28岁".to_string()));
    }

    #[test]
    fn extract_facts_dedup_prefix() {
        let facts = extract_facts("我喜欢咖啡，也喜欢音乐");
        // 两个"喜欢"主题合并，只保留第一条
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0], "用户喜欢咖啡");
    }

    #[test]
    fn fact_exists_dedup() {
        let conn = test_conn();
        let f = "用户喜欢喝咖啡".to_string();
        assert!(!fact_exists(&conn, &f).unwrap());
        add_memory(&conn, &f, "fact").unwrap();
        assert!(fact_exists(&conn, &f).unwrap());
    }

    #[test]
    fn build_context_formats() {
        let conn = test_conn();
        add_memory(&conn, "用户喜欢喝咖啡", "fact").unwrap();
        let ctx = build_memory_context(&conn, "咖啡", 5).unwrap();
        assert!(ctx.starts_with("[来自记忆的上下文"));
        assert!(ctx.contains("用户喜欢喝咖啡"));
        // 无命中返回空
        assert!(build_memory_context(&conn, "滑雪", 5).unwrap().is_empty());
    }

    #[test]
    fn delete_and_clear() {
        let conn = test_conn();
        let id = add_memory(&conn, "临时记忆", "chat").unwrap();
        delete_memory(&conn, id).unwrap();
        assert!(list_memories(&conn).unwrap().is_empty());
        add_memory(&conn, "a", "chat").unwrap();
        add_memory(&conn, "b", "chat").unwrap();
        clear_memories(&conn).unwrap();
        assert!(list_memories(&conn).unwrap().is_empty());
    }
}
