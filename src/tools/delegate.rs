use serde_json::{json, Value};

use super::registry::ToolDef;

/// delegate_task 工具定义
/// 注意：实际执行是异步的，这里返回占位结果
/// 真正的异步执行在 AgentLoop 中特殊处理
pub fn def() -> ToolDef {
    ToolDef {
        name: "delegate_task".into(),
        description: "将一个子任务委派给子Agent独立完成。子Agent有自己的对话上下文，会使用工具完成任务后返回结果。适用于可以独立完成的子任务。".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "要委派的任务描述，需要清晰明确"
                },
                "context": {
                    "type": "string",
                    "description": "提供给子Agent的背景信息（可选）"
                }
            },
            "required": ["task"]
        }),
        // 同步 handler 作为 fallback（实际走异步路径）
        handler: |args: Value| {
            let task = args["task"].as_str().unwrap_or("");
            format!("delegate_task 需要异步执行，任务: {}", task)
        },
    }
}

/// 异步执行 delegate_task（在 AgentLoop 中调用）
pub async fn execute_delegate(args: &Value) -> String {
    use crate::agent::subagent::SubAgent;
    use crate::tools::registry::ToolRegistry;

    let task = args["task"].as_str().unwrap_or("");
    let context = args["context"].as_str().unwrap_or("");

    if task.is_empty() {
        return "错误: task 不能为空".into();
    }

    let system_prompt = format!(
        "你是一个专注执行子任务的助手。你会使用可用的工具来完成任务，然后给出简洁的结果。\n{}",
        if context.is_empty() { String::new() } else { format!("\n背景信息: {}", context) }
    );

    // 子 Agent 使用基础工具（不包含 delegate_task，避免递归）
    let mut sub_tools = ToolRegistry::new();
    sub_tools.register(crate::tools::time_tool::def());
    sub_tools.register(crate::tools::weather::def());
    sub_tools.register(crate::tools::iot_device::list_devices_tool());
    sub_tools.register(crate::tools::iot_device::control_device_tool());
    sub_tools.register(crate::tools::iot_device::query_device_status_tool());

    let mut sub_agent = SubAgent::new(&system_prompt, sub_tools);

    println!("  🧵 [子Agent] 开始执行任务: {}", task);

    match sub_agent.run(task).await {
        Ok(result) => {
            println!("  🧵 [子Agent] 任务完成");
            result
        }
        Err(e) => format!("子Agent执行失败: {}", e),
    }
}
