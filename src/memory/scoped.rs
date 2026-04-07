use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::tools::registry::ToolDef;

const MEMORY_DIR: &str = "data/memory";

/// Scoped Memory — 按 scope_id 隔离的记忆存储
#[derive(Clone)]
pub struct ScopedMemory {
    scope_id: String,
}

impl ScopedMemory {
    pub fn new(scope_id: &str) -> Self {
        let _ = fs::create_dir_all(MEMORY_DIR);
        Self {
            scope_id: scope_id.to_string(),
        }
    }

    fn file_path(&self) -> String {
        format!("{}/{}.json", MEMORY_DIR, self.scope_id)
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.load_data().get(key).cloned()
    }

    pub fn set(&self, key: &str, value: &str) {
        let mut data = self.load_data();
        data.insert(key.into(), value.into());
        self.save_data(&data);
    }

    pub fn list(&self) -> HashMap<String, String> {
        self.load_data()
    }

    fn load_data(&self) -> HashMap<String, String> {
        let path = self.file_path();
        if Path::new(&path).exists() {
            let content = fs::read_to_string(&path).unwrap_or_default();
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            HashMap::new()
        }
    }

    fn save_data(&self, data: &HashMap<String, String>) {
        let _ = fs::create_dir_all(MEMORY_DIR);
        let json = serde_json::to_string_pretty(data).unwrap_or_default();
        let _ = fs::write(self.file_path(), json);
    }
}

/// save_memory 工具（支持 scope 参数）
pub fn save_scoped_memory_tool() -> ToolDef {
    ToolDef {
        name: "save_memory".into(),
        description: "保存一条用户信息到长期记忆。可指定 scope 隔离不同场景的记忆。".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "记忆键名，如'姓名'、'喜欢的温度'" },
                "value": { "type": "string", "description": "要记住的值" },
                "scope": { "type": "string", "description": "记忆作用域（可选），不同 scope 的记忆互相隔离。默认为 'default'" }
            },
            "required": ["key", "value"]
        }),
        handler: |args: Value| {
            let key = args["key"].as_str().unwrap_or("");
            let value = args["value"].as_str().unwrap_or("");
            let scope = args["scope"].as_str().unwrap_or("default");

            if key.is_empty() {
                return "错误: key 不能为空".into();
            }

            let mem = ScopedMemory::new(scope);
            mem.set(key, value);
            format!("已记住 [{}]: {} = {}", scope, key, value)
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scoped_isolation() {
        let scope_a = ScopedMemory::new("test_scope_a");
        let scope_b = ScopedMemory::new("test_scope_b");

        scope_a.set("key1", "value_a");
        scope_b.set("key1", "value_b");

        assert_eq!(scope_a.get("key1"), Some("value_a".to_string()));
        assert_eq!(scope_b.get("key1"), Some("value_b".to_string()));

        // Scope A should not see scope B's data
        assert_eq!(scope_a.get("key1").unwrap(), "value_a");
        assert_eq!(scope_b.get("key1").unwrap(), "value_b");

        // Cleanup
        let _ = fs::remove_file(scope_a.file_path());
        let _ = fs::remove_file(scope_b.file_path());
    }
}

/// recall_memory 工具（支持 scope 参数）
pub fn recall_scoped_memory_tool() -> ToolDef {
    ToolDef {
        name: "recall_memory".into(),
        description: "回忆之前保存的用户信息。可指定 scope 查询特定作用域的记忆。".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "要回忆的键名。留空则返回所有记忆" },
                "scope": { "type": "string", "description": "记忆作用域（可选）。默认为 'default'" }
            }
        }),
        handler: |args: Value| {
            let key = args["key"].as_str().unwrap_or("");
            let scope = args["scope"].as_str().unwrap_or("default");

            let mem = ScopedMemory::new(scope);
            let data = mem.list();

            if key.is_empty() {
                if data.is_empty() {
                    format!("没有 [{}] 作用域的记忆", scope)
                } else {
                    let items: Vec<String> = data.iter()
                        .map(|(k, v)| format!("{}: {}", k, v))
                        .collect();
                    format!("[{}] 所有记忆:\n{}", scope, items.join("\n"))
                }
            } else {
                match data.get(key) {
                    Some(v) => format!("[{}] {} = {}", scope, key, v),
                    None => format!("[{}] 没有关于 '{}' 的记忆", scope, key),
                }
            }
        },
    }
}
