use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::tools::registry::ToolDef;

const DATA_DIR: &str = "data";
const MEMORY_FILE: &str = "data/core_memory.json";

/// Core Memory — 长期记忆，持久化到 JSON 文件
#[derive(Clone)]
pub struct CoreMemory {
    data: Arc<Mutex<HashMap<String, String>>>,
}

impl CoreMemory {
    pub fn load() -> Self {
        // 确保 data 目录存在
        let _ = fs::create_dir_all(DATA_DIR);

        let data = if Path::new(MEMORY_FILE).exists() {
            let content = fs::read_to_string(MEMORY_FILE).unwrap_or_default();
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            HashMap::new()
        };

        if !data.is_empty() {
            println!("  🧠 加载 {} 条记忆", data.len());
        }

        Self {
            data: Arc::new(Mutex::new(data)),
        }
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.data.lock().unwrap().get(key).cloned()
    }

    pub fn set(&self, key: &str, value: &str) {
        self.data.lock().unwrap().insert(key.into(), value.into());
        self.save();
    }

    pub fn list(&self) -> HashMap<String, String> {
        self.data.lock().unwrap().clone()
    }

    fn save(&self) {
        let data = self.data.lock().unwrap();
        let json = serde_json::to_string_pretty(&*data).unwrap_or_default();
        let _ = fs::write(MEMORY_FILE, json);
    }

    /// 生成记忆摘要，注入 system prompt
    pub fn to_prompt_section(&self) -> String {
        let data = self.data.lock().unwrap();
        if data.is_empty() {
            return String::new();
        }
        let mut lines = vec!["\n\n## 已知用户信息（Core Memory）".to_string()];
        for (k, v) in data.iter() {
            lines.push(format!("- {}: {}", k, v));
        }
        lines.join("\n")
    }
}

/// 创建 save_memory 工具
pub fn save_memory_tool(memory: CoreMemory) -> ToolDef {
    ToolDef {
        name: "save_memory".into(),
        description: "保存一条用户信息到长期记忆（如姓名、偏好、习惯）".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "记忆键名，如'姓名'、'喜欢的温度'" },
                "value": { "type": "string", "description": "要记住的值" }
            },
            "required": ["key", "value"]
        }),
        handler: {
            // 用闭包不行（fn pointer），用全局 static 也麻烦
            // 简单方案：把 memory 序列化到环境变量，handler 里读
            // 更好的方案：改 handler 为 Box<dyn Fn>
            // 先用简单方案，handler 写入文件
            |args: Value| {
                let key = args["key"].as_str().unwrap_or("");
                let value = args["value"].as_str().unwrap_or("");
                if key.is_empty() {
                    return "错误: key 不能为空".into();
                }
                // 直接读写文件（因为 fn pointer 不能捕获环境）
                let _ = fs::create_dir_all(DATA_DIR);
                let mut data: HashMap<String, String> = if Path::new(MEMORY_FILE).exists() {
                    let content = fs::read_to_string(MEMORY_FILE).unwrap_or_default();
                    serde_json::from_str(&content).unwrap_or_default()
                } else {
                    HashMap::new()
                };
                data.insert(key.into(), value.into());
                let json = serde_json::to_string_pretty(&data).unwrap_or_default();
                let _ = fs::write(MEMORY_FILE, json);
                format!("已记住: {} = {}", key, value)
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_memory() -> CoreMemory {
        // Use a test-specific file
        let _ = fs::create_dir_all(DATA_DIR);
        let mem = CoreMemory {
            data: Arc::new(Mutex::new(HashMap::new())),
        };
        mem
    }

    #[test]
    fn test_save_and_recall() {
        let mem = test_memory();
        mem.data.lock().unwrap().insert("name".into(), "Alice".into());
        assert_eq!(mem.get("name"), Some("Alice".to_string()));
        assert_eq!(mem.get("nonexistent"), None);
    }

    #[test]
    fn test_prompt_section() {
        let mem = test_memory();
        // Empty memory should return empty string
        assert!(mem.to_prompt_section().is_empty());

        // With data, should contain header and key-value pairs
        mem.data.lock().unwrap().insert("name".into(), "Bob".into());
        let section = mem.to_prompt_section();
        assert!(section.contains("Core Memory"), "Section: {}", section);
        assert!(section.contains("name"), "Section: {}", section);
        assert!(section.contains("Bob"), "Section: {}", section);
    }
}

/// 创建 recall_memory 工具
pub fn recall_memory_tool() -> ToolDef {
    ToolDef {
        name: "recall_memory".into(),
        description: "回忆之前保存的用户信息".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "要回忆的键名，如'姓名'。留空则返回所有记忆" }
            }
        }),
        handler: |args: Value| {
            let data: HashMap<String, String> = if Path::new(MEMORY_FILE).exists() {
                let content = fs::read_to_string(MEMORY_FILE).unwrap_or_default();
                serde_json::from_str(&content).unwrap_or_default()
            } else {
                HashMap::new()
            };

            let key = args["key"].as_str().unwrap_or("");
            if key.is_empty() {
                if data.is_empty() {
                    "没有保存任何记忆".into()
                } else {
                    let items: Vec<String> = data.iter()
                        .map(|(k, v)| format!("{}: {}", k, v))
                        .collect();
                    format!("所有记忆:\n{}", items.join("\n"))
                }
            } else {
                match data.get(key) {
                    Some(v) => format!("{} = {}", key, v),
                    None => format!("没有关于 '{}' 的记忆", key),
                }
            }
        },
    }
}
