use serde::Deserialize;
use std::env;
use std::path::Path;
use std::sync::OnceLock;

/// Global config singleton
static GLOBAL_CONFIG: OnceLock<Config> = OnceLock::new();

/// Get or initialize the global config
pub fn get_config() -> &'static Config {
    GLOBAL_CONFIG.get_or_init(|| Config::load())
}

/// IoTClaw complete configuration (TOML-based)
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub agent: AgentConfig,

    #[serde(default)]
    pub memory: MemoryConfig,

    #[serde(default)]
    pub server: ServerConfig,

    #[serde(default)]
    pub security: SecurityConfig,

    #[serde(default)]
    pub feishu: FeishuConfig,

    #[serde(default)]
    pub wechat: WechatConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    #[serde(default = "default_model")]
    pub model: String,

    #[serde(default = "default_provider")]
    pub provider: String,

    #[serde(default = "default_max_iterations")]
    pub max_iterations: usize,

    #[serde(default = "default_token_limit")]
    pub token_limit: usize,

    #[serde(default = "default_skills_dir")]
    pub skills_dir: String,

    #[serde(default = "default_log_level")]
    pub log_level: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MemoryConfig {
    #[serde(default = "default_core_memory_path")]
    pub core_memory_path: String,

    #[serde(default = "default_vector_memory_path")]
    pub vector_memory_path: String,

    #[serde(default = "default_data_dir")]
    pub data_dir: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_port")]
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SecurityConfig {
    #[serde(default = "default_exec_whitelist")]
    pub exec_whitelist: Vec<String>,

    #[serde(default = "default_exec_timeout")]
    pub exec_timeout_secs: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FeishuConfig {
    #[serde(default)]
    pub webhook: String,

    #[serde(default)]
    pub app_id: String,

    #[serde(default)]
    pub app_secret: String,

    #[serde(default)]
    pub encrypt_key: String,

    #[serde(default)]
    pub verification_token: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WechatConfig {
    #[serde(default)]
    pub webhook: String,

    #[serde(default)]
    pub corp_id: String,

    #[serde(default)]
    pub corp_secret: String,

    #[serde(default)]
    pub agent_id: String,

    #[serde(default)]
    pub token: String,

    #[serde(default)]
    pub encoding_aes_key: String,
}

// --- Defaults ---

fn default_model() -> String { "qwen-plus".into() }
fn default_provider() -> String { "dashscope".into() }
fn default_max_iterations() -> usize { 10 }
fn default_token_limit() -> usize { 6000 }
fn default_skills_dir() -> String { "skills".into() }
fn default_log_level() -> String { "info".into() }
fn default_core_memory_path() -> String { "data/core_memory.json".into() }
fn default_vector_memory_path() -> String { "data/vector_memory.json".into() }
fn default_data_dir() -> String { "data".into() }
fn default_port() -> u16 { 3000 }
fn default_exec_whitelist() -> Vec<String> {
    vec!["ls", "cat", "echo", "date", "curl", "python3", "pwd", "head", "tail", "wc", "grep", "find", "whoami", "hostname"]
        .into_iter().map(String::from).collect()
}
fn default_exec_timeout() -> u64 { 10 }

// --- Default impls ---

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            model: default_model(),
            provider: default_provider(),
            max_iterations: default_max_iterations(),
            token_limit: default_token_limit(),
            skills_dir: default_skills_dir(),
            log_level: default_log_level(),
        }
    }
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            core_memory_path: default_core_memory_path(),
            vector_memory_path: default_vector_memory_path(),
            data_dir: default_data_dir(),
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self { port: default_port() }
    }
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            exec_whitelist: default_exec_whitelist(),
            exec_timeout_secs: default_exec_timeout(),
        }
    }
}

impl Default for FeishuConfig {
    fn default() -> Self {
        Self {
            webhook: String::new(),
            app_id: String::new(),
            app_secret: String::new(),
            encrypt_key: String::new(),
            verification_token: String::new(),
        }
    }
}

impl Default for WechatConfig {
    fn default() -> Self {
        Self {
            webhook: String::new(),
            corp_id: String::new(),
            corp_secret: String::new(),
            agent_id: String::new(),
            token: String::new(),
            encoding_aes_key: String::new(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            agent: AgentConfig::default(),
            memory: MemoryConfig::default(),
            server: ServerConfig::default(),
            security: SecurityConfig::default(),
            feishu: FeishuConfig::default(),
            wechat: WechatConfig::default(),
        }
    }
}

impl Config {
    /// Load config: iotclaw.toml > ~/.iotclaw/config.toml > defaults
    /// .env variables override TOML values
    pub fn load() -> Self {
        // 1. Try iotclaw.toml, then ~/.iotclaw/config.toml
        let toml_paths = vec![
            "iotclaw.toml".to_string(),
            dirs_home().map(|h| format!("{}/.iotclaw/config.toml", h)).unwrap_or_default(),
        ];

        let mut config = Config::default();

        for path in &toml_paths {
            if path.is_empty() { continue; }
            if Path::new(path).exists() {
                match std::fs::read_to_string(path) {
                    Ok(content) => {
                        match toml::from_str::<Config>(&content) {
                            Ok(c) => {
                                println!("  Config: loaded {}", path);
                                config = c;
                                break;
                            }
                            Err(e) => {
                                eprintln!("  Config: parse {} failed: {}, using defaults", path, e);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("  Config: read {} failed: {}", path, e);
                    }
                }
            }
        }

        // 2. .env overrides (already loaded via dotenv)
        if let Ok(v) = env::var("DASHSCOPE_MODEL") {
            config.agent.model = v;
        }
        if let Ok(v) = env::var("DEFAULT_PROVIDER") {
            config.agent.provider = v;
        }
        if let Ok(v) = env::var("LOG_LEVEL") {
            config.agent.log_level = v;
        }
        if let Ok(v) = env::var("MAX_ITERATIONS") {
            if let Ok(n) = v.parse() { config.agent.max_iterations = n; }
        }
        if let Ok(v) = env::var("TOKEN_LIMIT") {
            if let Ok(n) = v.parse() { config.agent.token_limit = n; }
        }
        if let Ok(v) = env::var("FEISHU_SERVER_PORT") {
            if let Ok(n) = v.parse() { config.server.port = n; }
        }
        if let Ok(v) = env::var("FEISHU_WEBHOOK") {
            if !v.is_empty() { config.feishu.webhook = v; }
        }
        if let Ok(v) = env::var("FEISHU_APP_ID") {
            if !v.is_empty() { config.feishu.app_id = v; }
        }
        if let Ok(v) = env::var("FEISHU_APP_SECRET") {
            if !v.is_empty() { config.feishu.app_secret = v; }
        }
        if let Ok(v) = env::var("WECHAT_WEBHOOK") {
            if !v.is_empty() { config.wechat.webhook = v; }
        }
        if let Ok(v) = env::var("WECHAT_CORP_ID") {
            if !v.is_empty() { config.wechat.corp_id = v; }
        }

        config
    }
}

/// Get home directory path
fn dirs_home() -> Option<String> {
    env::var("HOME").ok().or_else(|| env::var("USERPROFILE").ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.agent.model, "qwen-plus");
        assert_eq!(config.agent.provider, "dashscope");
        assert_eq!(config.agent.max_iterations, 10);
        assert_eq!(config.agent.token_limit, 6000);
        assert_eq!(config.server.port, 3000);
        assert_eq!(config.security.exec_timeout_secs, 10);
        assert!(!config.security.exec_whitelist.is_empty());
        assert!(config.security.exec_whitelist.contains(&"echo".to_string()));
        assert!(config.security.exec_whitelist.contains(&"ls".to_string()));
    }
}
