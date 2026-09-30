mod config;
mod db;
mod llm;
mod memory;
mod types;

use std::sync::Mutex;
use tauri::{Manager, State};

/// 全局状态：SQLite 连接 + 应用配置
pub struct AppState {
    pub db: Mutex<rusqlite::Connection>,
    pub config: Mutex<config::AppConfig>,
}

// ---------- 配置 ----------

#[tauri::command]
fn get_config(state: State<'_, AppState>) -> config::AppConfig {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
fn save_config(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    config: config::AppConfig,
) -> Result<(), String> {
    let path = app.path().app_data_dir().map_err(|e| e.to_string())?.join("config.json");
    config::save_config(&path, &config)?;
    *state.config.lock().unwrap() = config;
    Ok(())
}

// ---------- 会话 ----------

#[tauri::command]
fn new_conversation(state: State<'_, AppState>, title: Option<String>) -> Result<i64, String> {
    let conn = state.db.lock().unwrap();
    db::new_conversation(&conn, &title.unwrap_or_else(|| "新对话".to_string()))
}

#[tauri::command]
fn list_conversations(state: State<'_, AppState>) -> Result<Vec<types::ConversationSummary>, String> {
    let conn = state.db.lock().unwrap();
    db::list_conversations(&conn)
}

#[tauri::command]
fn load_conversation(state: State<'_, AppState>, id: i64) -> Result<Vec<types::ChatMessage>, String> {
    let conn = state.db.lock().unwrap();
    db::load_conversation(&conn, id)
}

#[tauri::command]
fn delete_conversation(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let conn = state.db.lock().unwrap();
    db::delete_conversation(&conn, id)
}

#[tauri::command]
fn rename_conversation(state: State<'_, AppState>, id: i64, title: String) -> Result<(), String> {
    let conn = state.db.lock().unwrap();
    db::rename_conversation(&conn, id, &title)
}

#[tauri::command]
fn save_message(
    state: State<'_, AppState>,
    conversation_id: i64,
    role: String,
    content: String,
) -> Result<i64, String> {
    let conn = state.db.lock().unwrap();
    db::save_message(&conn, conversation_id, &role, &content)
}

// ---------- 记忆 ----------

#[tauri::command]
fn list_memories(state: State<'_, AppState>) -> Result<Vec<memory::MemoryItem>, String> {
    let conn = state.db.lock().unwrap();
    memory::list_memories(&conn)
}

#[tauri::command]
fn delete_memory(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let conn = state.db.lock().unwrap();
    memory::delete_memory(&conn, id)
}

#[tauri::command]
fn clear_memories(state: State<'_, AppState>) -> Result<(), String> {
    let conn = state.db.lock().unwrap();
    memory::clear_memories(&conn)
}

// ---------- LLM 聊天 ----------

#[tauri::command]
async fn chat_stream(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    system_prompt: String,
    messages: Vec<types::ChatMessage>,
    memory_enabled: bool,
) -> Result<String, String> {
    let cfg = state.config.lock().unwrap().clone();
    if cfg.api_key.trim().is_empty() {
        return Err("未配置 API Key，请先在设置中填写".to_string());
    }

    // 记忆：归档本条用户消息 + 提取画像事实 + 检索注入上下文
    let mut sys = system_prompt.clone();
    if memory_enabled {
        let conn = state.db.lock().unwrap();
        if let Some(last_user) = messages.iter().rev().find(|m| m.role == "user") {
            let _ = memory::add_memory(&conn, &last_user.content, "chat");
            for fact in memory::extract_facts(&last_user.content) {
                if !memory::fact_exists(&conn, &fact).unwrap_or(false) {
                    let _ = memory::add_memory(&conn, &fact, "fact");
                }
            }
            if let Ok(ctx) = memory::build_memory_context(&conn, &last_user.content, 6) {
                if !ctx.is_empty() {
                    sys = format!("{}\n\n{}", system_prompt, ctx);
                }
            }
        }
    }

    let client = reqwest::Client::new();
    llm::stream_chat(&app, &client, &cfg, &sys, &messages).await
}

// ---------- 入口 ----------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| -> Result<(), Box<dyn std::error::Error>> {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let conn = db::init(&data_dir.join("mindpal.db"))
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
            let cfg = config::load_config(&data_dir.join("config.json"));
            app.manage(AppState {
                db: Mutex::new(conn),
                config: Mutex::new(cfg),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            new_conversation,
            list_conversations,
            load_conversation,
            delete_conversation,
            rename_conversation,
            save_message,
            list_memories,
            delete_memory,
            clear_memories,
            chat_stream
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
