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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_fields() {
        let c = AppConfig::default();
        assert_eq!(c.provider, "deepseek");
        assert_eq!(c.model, DEEPSEEK_MODEL);
        assert_eq!(c.base_url, DEEPSEEK_BASE);
        assert_eq!(c.temperature, 0.8);
        assert!(c.api_key.is_empty());
    }

    #[test]
    fn save_then_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("mindpal-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        let mut c = AppConfig::default();
        c.api_key = "sk-test".to_string();
        c.model = "custom-model".to_string();
        save_config(&path, &c).unwrap();
        let loaded = load_config(&path);
        assert_eq!(loaded.api_key, "sk-test");
        assert_eq!(loaded.model, "custom-model");
        assert_eq!(loaded.provider, "deepseek");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn corrupt_file_falls_back_to_default() {
        let dir = std::env::temp_dir().join(format!("mindpal-cfg-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(load_config(&path).provider, "deepseek");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_file_falls_back_to_default() {
        let path = std::env::temp_dir().join("mindpal-cfg-nonexistent.json");
        assert_eq!(load_config(&path).model, DEEPSEEK_MODEL);
    }
}
