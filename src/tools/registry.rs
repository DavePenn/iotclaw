use serde_json::{json, Value};
use std::collections::HashMap;

/// 工具定义
#[derive(Clone)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub parameters: Value, // JSON Schema
    pub handler: fn(Value) -> String,
}

/// 工具注册表
pub struct ToolRegistry {
    tools: HashMap<String, ToolDef>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: ToolDef) {
        self.tools.insert(tool.name.clone(), tool);
    }

    /// 执行工具调用
    pub fn execute(&self, name: &str, args: Value) -> Result<String, String> {
        match self.tools.get(name) {
            Some(tool) => Ok((tool.handler)(args)),
            None => Err(format!("未知工具: {}", name)),
        }
    }

    /// 生成 OpenAI 兼容的 tools 数组（给模型用）
    pub fn to_openai_tools(&self) -> Vec<Value> {
        self.tools
            .values()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    }
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_tool() -> ToolDef {
        ToolDef {
            name: "echo".into(),
            description: "Echo back input".into(),
            parameters: json!({"type": "object", "properties": {}}),
            handler: |args: Value| {
                format!("echo: {}", args["msg"].as_str().unwrap_or(""))
            },
        }
    }

    #[test]
    fn test_register_and_execute() {
        let mut reg = ToolRegistry::new();
        reg.register(dummy_tool());
        let result = reg.execute("echo", json!({"msg": "hello"}));
        assert!(result.is_ok());
        assert!(result.unwrap().contains("hello"));
    }

    #[test]
    fn test_execute_unknown_tool() {
        let reg = ToolRegistry::new();
        let result = reg.execute("nonexistent", json!({}));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("未知工具"));
    }

    #[test]
    fn test_to_openai_tools() {
        let mut reg = ToolRegistry::new();
        reg.register(dummy_tool());
        let tools = reg.to_openai_tools();
        assert_eq!(tools.len(), 1);
        let tool = &tools[0];
        assert_eq!(tool["type"], "function");
        assert_eq!(tool["function"]["name"], "echo");
        assert!(tool["function"]["description"].as_str().unwrap().len() > 0);
        assert!(tool["function"]["parameters"].is_object());
    }
}
