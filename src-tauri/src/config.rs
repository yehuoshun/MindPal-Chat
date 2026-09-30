use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// 各 provider 的默认端点与模型
pub const DEEPSEEK_BASE: &str = "https://api.deepseek.com/v1";
pub const DEEPSEEK_MODEL: &str = "deepseek-chat";
pub const CLAUDE_BASE: &str = "https://api.anthropic.com/v1";
pub const CLAUDE_MODEL: &str = "claude-sonnet-4-20250514";

/// 应用配置，保存到 app_data_dir/config.json（本地，不上云）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub provider: String, // "deepseek" | "claude"
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    pub temperature: f64,
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            provider: "deepseek".to_string(),
            api_key: String::new(),
            model: DEEPSEEK_MODEL.to_string(),
            base_url: DEEPSEEK_BASE.to_string(),
            temperature: 0.8,
        }
    }
}

/// 读取配置；文件不存在或损坏时回退默认值
pub fn load_config(path: &Path) -> AppConfig {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => AppConfig::default(),
    }
}

/// 保存配置（覆盖写）
pub fn save_config(path: &Path, config: &AppConfig) -> Result<(), String> {
    let json = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| format!("写入配置失败: {e}"))
}
