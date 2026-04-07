use serde_json::{json, Value};

use super::registry::ToolDef;

pub fn def() -> ToolDef {
    ToolDef {
        name: "get_weather".into(),
        description: "查询指定城市的天气信息".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "city": {
                    "type": "string",
                    "description": "城市名称，如'北京'、'上海'"
                }
            },
            "required": ["city"]
        }),
        handler: |args: Value| {
            let city = args["city"].as_str().unwrap_or("北京");
            // 模拟天气数据（后续可接真实 API）
            format!(
                "{}: 晴，温度 22°C，湿度 45%，空气质量良",
                city
            )
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_weather_tool() {
        let tool = def();
        let result = (tool.handler)(json!({"city": "上海"}));
        assert!(result.contains("上海"), "Result should contain city name: {}", result);
    }
}
