mod config;
mod db;
mod llm;
mod memory;
mod stt;
mod tts;
mod types;

use std::sync::Mutex;
use sha2::Digest;
use tauri::{Manager, State};

/// 全局状态：SQLite 连接 + 应用配置 + 语音识别上下文缓存
pub struct AppState {
    pub db: Mutex<rusqlite::Connection>,
    pub config: Mutex<config::AppConfig>,
    pub whisper: Mutex<Option<stt::SharedContext>>,
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

// ---------- 语音 ----------

#[tauri::command]
async fn tts_speak(app: tauri::AppHandle, text: String, voice: String) -> Result<String, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?.join("tts");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建语音目录失败: {e}"))?;
    // 同文本+音色复用缓存，不重复合成
    let mut h = sha2::Sha256::new();
    h.update(format!("{voice}|{text}").as_bytes());
    let name: String = h.finalize().iter().take(16).map(|b| format!("{b:02x}")).collect();
    let path = dir.join(format!("{name}.mp3"));
    if !path.exists() {
        tts::synthesize(&text, &voice, &path).await?;
    }
    Ok(path.to_string_lossy().to_string())
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
    let sys = if memory_enabled {
        let conn = state.db.lock().unwrap();
        memory::apply_memory(&conn, &system_prompt, &messages)
    } else {
        system_prompt.clone()
    };

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
                whisper: Mutex::new(None),
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
            tts_speak,
            stt::stt_status,
            stt::stt_download_model,
            stt::stt_transcribe,
            chat_stream
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
