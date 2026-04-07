use serde_json::{json, Value};

use super::registry::ToolDef;

/// 模拟设备数据
fn get_device_db() -> Vec<Value> {
    vec![
        json!({"id": "light_living", "name": "客厅灯", "room": "客厅", "type": "light", "risk": "low", "status": "off"}),
        json!({"id": "light_bedroom", "name": "卧室灯", "room": "卧室", "type": "light", "risk": "low", "status": "on"}),
        json!({"id": "curtain_living", "name": "客厅窗帘", "room": "客厅", "type": "curtain", "risk": "low", "status": "closed"}),
        json!({"id": "ac_bedroom", "name": "卧室空调", "room": "卧室", "type": "ac", "risk": "medium", "status": "off", "temp": 26}),
        json!({"id": "ac_living", "name": "客厅空调", "room": "客厅", "type": "ac", "risk": "medium", "status": "off", "temp": 24}),
        json!({"id": "sweeper", "name": "扫地机器人", "room": "客厅", "type": "sweeper", "risk": "medium", "status": "idle"}),
        json!({"id": "lock_front", "name": "前门门锁", "room": "玄关", "type": "lock", "risk": "high", "status": "locked"}),
        json!({"id": "gas_valve", "name": "燃气阀门", "room": "厨房", "type": "gas_valve", "risk": "high", "status": "closed"}),
        json!({"id": "camera_door", "name": "门口摄像头", "room": "玄关", "type": "camera", "risk": "high", "status": "recording"}),
        json!({"id": "speaker", "name": "小爱音箱", "room": "客厅", "type": "speaker", "risk": "low", "status": "standby", "volume": 50}),
    ]
}

/// 设备列表工具
pub fn list_devices_tool() -> ToolDef {
    ToolDef {
        name: "list_devices".into(),
        description: "列出家中所有智能设备及其状态".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "room": { "type": "string", "description": "按房间筛选，如'客厅'、'卧室'。留空返回所有" }
            }
        }),
        handler: |args: Value| {
            let devices = get_device_db();
            let room = args["room"].as_str().unwrap_or("");

            let filtered: Vec<&Value> = if room.is_empty() {
                devices.iter().collect()
            } else {
                devices.iter().filter(|d| d["room"].as_str() == Some(room)).collect()
            };

            let lines: Vec<String> = filtered.iter().map(|d| {
                let risk_label = match d["risk"].as_str().unwrap_or("") {
                    "low" => "🟢低",
                    "medium" => "🟡中",
                    "high" => "🔴高",
                    _ => "?",
                };
                format!("{}({}) [{}] 状态:{} 风险:{}",
                    d["name"].as_str().unwrap_or(""),
                    d["room"].as_str().unwrap_or(""),
                    d["id"].as_str().unwrap_or(""),
                    d["status"].as_str().unwrap_or(""),
                    risk_label
                )
            }).collect();

            format!("设备列表 ({} 台):\n{}", lines.len(), lines.join("\n"))
        },
    }
}

/// 设备控制工具
pub fn control_device_tool() -> ToolDef {
    ToolDef {
        name: "control_device".into(),
        description: "控制智能设备。注意：高风险设备（门锁/燃气/摄像头）不能自动控制，中风险设备（空调/热水器）需用户确认".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "device_id": { "type": "string", "description": "设备ID" },
                "action": { "type": "string", "description": "操作，如 on/off/open/close/set_temp" },
                "value": { "type": "string", "description": "操作参数，如温度值'26'" }
            },
            "required": ["device_id", "action"]
        }),
        handler: |args: Value| {
            let device_id = args["device_id"].as_str().unwrap_or("");
            let action = args["action"].as_str().unwrap_or("");
            let value = args["value"].as_str().unwrap_or("");

            let devices = get_device_db();
            let device = devices.iter().find(|d| d["id"].as_str() == Some(device_id));

            match device {
                None => format!("❌ 未找到设备: {}", device_id),
                Some(d) => {
                    let name = d["name"].as_str().unwrap_or("");
                    let risk = d["risk"].as_str().unwrap_or("low");

                    match risk {
                        "high" => {
                            format!("🔴 安全拒绝: {} 是高风险设备，不能自动控制。请用户手动操作或在 App 中确认。", name)
                        }
                        "medium" => {
                            format!("🟡 需要用户确认: 即将对 {} 执行 {} {}。这是中风险设备，请确认是否继续？",
                                name, action, value)
                        }
                        _ => {
                            // 低风险设备，直接执行
                            format!("✅ 已执行: {} → {} {}", name, action, value)
                        }
                    }
                }
            }
        },
    }
}

/// 设备状态查询工具
pub fn query_device_status_tool() -> ToolDef {
    ToolDef {
        name: "query_device_status".into(),
        description: "查询指定设备的详细状态".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "device_id": { "type": "string", "description": "设备ID" }
            },
            "required": ["device_id"]
        }),
        handler: |args: Value| {
            let device_id = args["device_id"].as_str().unwrap_or("");
            let devices = get_device_db();

            match devices.iter().find(|d| d["id"].as_str() == Some(device_id)) {
                None => format!("未找到设备: {}", device_id),
                Some(d) => serde_json::to_string_pretty(d).unwrap_or_default(),
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_devices() {
        let tool = list_devices_tool();
        let result = (tool.handler)(json!({}));
        assert!(result.contains("10 台"), "Should list 10 devices: {}", result);
    }

    #[test]
    fn test_control_low_risk() {
        let tool = control_device_tool();
        let result = (tool.handler)(json!({"device_id": "light_living", "action": "on"}));
        assert!(result.contains("✅"), "Low risk device should execute directly: {}", result);
    }

    #[test]
    fn test_control_medium_risk() {
        let tool = control_device_tool();
        let result = (tool.handler)(json!({"device_id": "ac_bedroom", "action": "on"}));
        assert!(result.contains("🟡"), "Medium risk device should need confirmation: {}", result);
    }

    #[test]
    fn test_control_high_risk() {
        let tool = control_device_tool();
        let result = (tool.handler)(json!({"device_id": "lock_front", "action": "open"}));
        assert!(result.contains("🔴"), "High risk device should be rejected: {}", result);
    }

    #[test]
    fn test_control_unknown_device() {
        let tool = control_device_tool();
        let result = (tool.handler)(json!({"device_id": "nonexistent", "action": "on"}));
        assert!(result.contains("❌"), "Unknown device should return error: {}", result);
    }
}
