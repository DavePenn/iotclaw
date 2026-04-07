use serde_json::{json, Value};

use crate::agent::llm_client::LLMClient;
use crate::tools::registry::ToolRegistry;

/// 子 Agent — 独立的消息上下文和 system prompt
pub struct SubAgent {
    llm_client: LLMClient,
    tools: ToolRegistry,
    messages: Vec<SubMessage>,
    system_prompt: String,
    max_iterations: usize,
}

#[derive(Clone)]
struct SubMessage {
    role: String,
    content: Option<String>,
    tool_calls: Option<Vec<SubToolCall>>,
    tool_call_id: Option<String>,
}

#[derive(Clone)]
struct SubToolCall {
    id: String,
    name: String,
    arguments: String,
}

impl SubAgent {
    /// 创建子 Agent（使用 qwen-turbo 轻量模型）
    pub fn new(system_prompt: &str, tools: ToolRegistry) -> Self {
        Self {
            llm_client: LLMClient::with_model("qwen-turbo"),
            tools,
            messages: Vec::new(),
            system_prompt: system_prompt.to_string(),
            max_iterations: 5,
        }
    }

    /// 执行任务并返回最终结果
    pub async fn run(&mut self, task: &str) -> Result<String, String> {
        self.messages.push(SubMessage {
            role: "user".into(),
            content: Some(task.into()),
            tool_calls: None,
            tool_call_id: None,
        });

        for _iteration in 0..self.max_iterations {
            let response = self.call_model().await?;

            let choice = response["choices"]
                .get(0)
                .ok_or("子 Agent: 模型返回空响应")?;

            let message = &choice["message"];

            if let Some(tool_calls) = message["tool_calls"].as_array() {
                if !tool_calls.is_empty() {
                    let mut parsed_calls = Vec::new();
                    for tc in tool_calls {
                        parsed_calls.push(SubToolCall {
                            id: tc["id"].as_str().unwrap_or("").into(),
                            name: tc["function"]["name"].as_str().unwrap_or("").into(),
                            arguments: tc["function"]["arguments"].as_str().unwrap_or("{}").into(),
                        });
                    }

                    self.messages.push(SubMessage {
                        role: "assistant".into(),
                        content: message["content"].as_str().map(|s| s.into()),
                        tool_calls: Some(parsed_calls.clone()),
                        tool_call_id: None,
                    });

                    for call in &parsed_calls {
                        let args: Value = serde_json::from_str(&call.arguments)
                            .unwrap_or(json!({}));

                        println!("    🔧 [子Agent] 调用工具: {}({})", call.name, call.arguments);

                        let result = match self.tools.execute(&call.name, args) {
                            Ok(r) => r,
                            Err(e) => format!("工具调用失败: {}", e),
                        };

                        println!("    📋 [子Agent] 结果: {}", result);

                        self.messages.push(SubMessage {
                            role: "tool".into(),
                            content: Some(result),
                            tool_calls: None,
                            tool_call_id: Some(call.id.clone()),
                        });
                    }

                    continue;
                }
            }

            let reply = message["content"]
                .as_str()
                .unwrap_or("（子Agent无回复）")
                .to_string();

            return Ok(reply);
        }

        Err("子 Agent 超过最大迭代次数".into())
    }

    async fn call_model(&self) -> Result<Value, String> {
        let mut api_messages = vec![json!({
            "role": "system",
            "content": self.system_prompt,
        })];

        for msg in &self.messages {
            let mut m = json!({ "role": msg.role });

            if let Some(content) = &msg.content {
                m["content"] = json!(content);
            }

            if let Some(tool_calls) = &msg.tool_calls {
                m["tool_calls"] = json!(tool_calls.iter().map(|tc| {
                    json!({
                        "id": tc.id,
                        "type": "function",
                        "function": {
                            "name": tc.name,
                            "arguments": tc.arguments,
                        }
                    })
                }).collect::<Vec<_>>());
            }

            if let Some(tool_call_id) = &msg.tool_call_id {
                m["tool_call_id"] = json!(tool_call_id);
            }

            api_messages.push(m);
        }

        let tools_json = self.tools.to_openai_tools();
        self.llm_client.chat(&api_messages, &tools_json).await
    }
}
