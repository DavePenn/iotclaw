use chrono::Local;
use serde_json::{json, Value};

use super::registry::ToolDef;

pub fn def() -> ToolDef {
    ToolDef {
        name: "get_current_time".into(),
        description: "获取当前时间和日期".into(),
        parameters: json!({
            "type": "object",
            "properties": {},
        }),
        handler: |_args: Value| {
            let now = Local::now();
            format!(
                "当前时间: {} ({})",
                now.format("%Y-%m-%d %H:%M:%S"),
                now.format("%A")
            )
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_time_tool() {
        let tool = def();
        let result = (tool.handler)(json!({}));
        let year = chrono::Local::now().format("%Y").to_string();
        assert!(result.contains(&year), "Result should contain current year: {}", result);
    }
}
