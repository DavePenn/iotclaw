use serde_json::{json, Value};

use crate::agent::llm_client::LLMClient;
use crate::agent::cancellation::CancellationToken;
use crate::tools::registry::ToolRegistry;
use crate::tools::experience::ExperienceManager;
use crate::skills::loader::SkillDef;
use crate::memory::core_memory::CoreMemory;
use crate::context::manager::ContextManager;
use crate::logging::Logger;
use crate::security::{SecurityScanner, RiskLevel};

/// Agent 对话消息
#[derive(Clone)]
pub struct Message {
    pub role: String,
    pub content: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub tool_call_id: Option<String>,
}

#[derive(Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// Agent Loop 引擎
pub struct AgentLoop {
    llm_client: LLMClient,
    tools: ToolRegistry,
    messages: Vec<Message>,
    system_prompt: String,
    allowed_tools: Option<Vec<String>>,
    core_memory: CoreMemory,
    max_iterations: usize,
    feishu_enabled: bool,
    logger: Logger,
    experience: ExperienceManager,
    cancellation_token: CancellationToken,
    start_time: std::time::Instant,
    message_count: usize,
    current_skill: String,
    streaming_enabled: bool,
}

impl AgentLoop {
    pub fn new(tools: ToolRegistry, core_memory: CoreMemory) -> Self {
        let feishu_enabled = std::env::var("FEISHU_WEBHOOK").is_ok();
        let logger = Logger::new();
        let experience = ExperienceManager::load();

        Self {
            llm_client: LLMClient::new(),
            tools,
            messages: Vec::new(),
            system_prompt: "你是 IoTClaw, 一个智能助手。".into(),
            allowed_tools: None,
            core_memory,
            max_iterations: 10,
            feishu_enabled,
            logger,
            experience,
            cancellation_token: CancellationToken::new(),
            start_time: std::time::Instant::now(),
            message_count: 0,
            current_skill: "default".into(),
            streaming_enabled: false,
        }
    }

    /// 获取 LLMClient 引用（给外部使用）
    pub fn llm_client(&self) -> &LLMClient {
        &self.llm_client
    }

    /// 获取 CancellationToken
    pub fn cancellation_token(&self) -> &CancellationToken {
        &self.cancellation_token
    }

    /// 设置是否启用流式输出
    pub fn set_streaming(&mut self, enabled: bool) {
        self.streaming_enabled = enabled;
    }

    /// 获取状态信息
    pub fn status_info(&self) -> String {
        let uptime = self.start_time.elapsed();
        let hours = uptime.as_secs() / 3600;
        let mins = (uptime.as_secs() % 3600) / 60;
        let secs = uptime.as_secs() % 60;
        let tool_count = self.tools.to_openai_tools().len();
        let feishu_status = if self.feishu_enabled { "已连接" } else { "未配置" };
        let wechat_status = if std::env::var("WECHAT_WEBHOOK").is_ok() { "已连接" } else { "未配置" };

        format!(
            "模型: {}\n运行时间: {}h {}m {}s\n消息数: {}\n当前 Skill: {}\n已注册工具: {} 个\n飞书: {}\n微信: {}",
            self.llm_client.model_name(),
            hours, mins, secs,
            self.message_count,
            self.current_skill,
            tool_count,
            feishu_status,
            wechat_status,
        )
    }

    /// 加载 Skill
    pub fn load_skill(&mut self, skill: &SkillDef) {
        let mut skill = skill.clone();
        skill.load_content(); // 按需加载完整内容
        self.system_prompt = skill.system_prompt.clone();
        self.allowed_tools = if skill.tools.is_empty() {
            Some(vec![])
        } else {
            Some(skill.tools.clone())
        };
        self.messages.clear();
        self.current_skill = skill.name.clone();
        println!("  已切换到 Skill: {} -- {}", skill.name, skill.description);
    }

    /// 处理用户输入，返回最终回复
    pub async fn chat(&mut self, user_input: &str) -> Result<String, String> {
        // 重置 cancellation token
        self.cancellation_token.reset();

        // Prompt injection 检测
        match SecurityScanner::check_injection(user_input) {
            RiskLevel::Blocked => {
                return Err("输入被拦截：检测到潜在的 prompt injection 攻击".into());
            }
            RiskLevel::Suspicious => {
                println!("  [Security] 可疑输入，继续处理但已记录");
                self.logger.log(json!({
                    "event": "security_warning",
                    "type": "suspicious_input",
                    "input_preview": &user_input[..user_input.len().min(100)],
                }));
            }
            RiskLevel::Safe => {}
        }

        // 上下文溢出检查 — 用模型压缩
        if let Some((start, end)) = ContextManager::check_overflow(&self.messages) {
            let to_compress: Vec<Message> = self.messages[start..end].to_vec();
            let summary = ContextManager::compress_with_model(&self.llm_client, &to_compress).await;
            let remaining = self.messages[end..].to_vec();
            self.messages = vec![summary];
            self.messages.extend(remaining);
            println!("  上下文压缩: {} 条消息 -> 模型摘要", to_compress.len());
        }

        // 添加用户消息
        self.messages.push(Message {
            role: "user".into(),
            content: Some(user_input.into()),
            tool_calls: None,
            tool_call_id: None,
        });

        self.message_count += 1;

        // 记录用户消息日志
        self.logger.log_message("user", user_input, None, None, None);

        // Agent Loop
        for _iteration in 0..self.max_iterations {
            // 检查取消
            if self.cancellation_token.is_cancelled() {
                return Ok("已中断".into());
            }

            let response = if self.streaming_enabled {
                self.call_model_stream().await?
            } else {
                self.call_model().await?
            };

            // 记录模型调用日志
            self.logger.log_message("model_response", "", None, None, Some(&response.to_string()));

            let choice = response["choices"]
                .get(0)
                .ok_or("模型返回空响应")?;

            let message = &choice["message"];
            let _finish_reason = choice["finish_reason"].as_str().unwrap_or("");

            if let Some(tool_calls) = message["tool_calls"].as_array() {
                if !tool_calls.is_empty() {
                    let mut parsed_calls = Vec::new();
                    for tc in tool_calls {
                        let call = ToolCall {
                            id: tc["id"].as_str().unwrap_or("").into(),
                            name: tc["function"]["name"].as_str().unwrap_or("").into(),
                            arguments: tc["function"]["arguments"].as_str().unwrap_or("{}").into(),
                        };
                        parsed_calls.push(call);
                    }

                    self.messages.push(Message {
                        role: "assistant".into(),
                        content: message["content"].as_str().map(|s| s.into()),
                        tool_calls: Some(parsed_calls.clone()),
                        tool_call_id: None,
                    });

                    for call in &parsed_calls {
                        // 检查取消
                        if self.cancellation_token.is_cancelled() {
                            return Ok("已中断".into());
                        }

                        let args: Value = serde_json::from_str(&call.arguments)
                            .unwrap_or(json!({}));

                        println!("  调用工具: {}({})", call.name, call.arguments);

                        // delegate_task 需要异步执行
                        let result = if call.name == "delegate_task" {
                            crate::tools::delegate::execute_delegate(&args).await
                        } else {
                            match self.tools.execute(&call.name, args) {
                                Ok(r) => r,
                                Err(e) => format!("工具调用失败: {}", e),
                            }
                        };

                        println!("  结果: {}", result);

                        // 记录工具执行日志
                        self.logger.log_message(
                            "tool",
                            &result,
                            Some(&call.name),
                            Some(&call.arguments),
                            Some(&result),
                        );

                        self.messages.push(Message {
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
                .unwrap_or("（无回复）")
                .to_string();

            // 出站泄露扫描
            let reply = SecurityScanner::check_outbound_leak(&reply);

            self.messages.push(Message {
                role: "assistant".into(),
                content: Some(reply.clone()),
                tool_calls: None,
                tool_call_id: None,
            });

            self.message_count += 1;

            // 记录助手回复日志
            self.logger.log_message("assistant", &reply, None, None, None);

            // 如果启用了飞书，同时发送到飞书
            if self.feishu_enabled {
                let bot = crate::im::feishu::FeishuBot::from_env();
                if let Err(e) = bot.send(&reply).await {
                    eprintln!("  飞书发送失败: {}", e);
                }
            }

            // Persist session to SQLite after each completed chat turn
            self.persist_session();

            return Ok(reply);
        }

        Err("Agent Loop 超过最大迭代次数".into())
    }

    /// Persist current messages to SQLite
    fn persist_session(&self) {
        let db = crate::storage::Database::global();
        let session_id = self.logger.session_id();
        // Serialize messages to a compact JSON array
        let messages_json: Vec<serde_json::Value> = self.messages.iter().map(|m| {
            let mut obj = json!({ "role": &m.role });
            if let Some(c) = &m.content { obj["content"] = json!(c); }
            if let Some(tc) = &m.tool_calls {
                obj["tool_calls"] = json!(tc.iter().map(|t| json!({
                    "id": &t.id, "name": &t.name, "arguments": &t.arguments
                })).collect::<Vec<_>>());
            }
            if let Some(tid) = &m.tool_call_id { obj["tool_call_id"] = json!(tid); }
            obj
        }).collect();
        let json_str = serde_json::to_string(&messages_json).unwrap_or_else(|_| "[]".into());
        if let Err(e) = db.save_session(session_id, &json_str) {
            eprintln!("  Session persist error: {}", e);
        }
    }

    /// Restore session from SQLite by session_id
    pub fn restore_session(&mut self, session_id: &str) -> Result<(), String> {
        let db = crate::storage::Database::global();
        let json_str = db.load_session(session_id)?
            .ok_or_else(|| format!("Session '{}' not found", session_id))?;
        let arr: Vec<serde_json::Value> = serde_json::from_str(&json_str)
            .map_err(|e| format!("Parse session: {}", e))?;
        self.messages.clear();
        for obj in &arr {
            let role = obj["role"].as_str().unwrap_or("user").to_string();
            let content = obj["content"].as_str().map(|s| s.to_string());
            let tool_call_id = obj["tool_call_id"].as_str().map(|s| s.to_string());
            let tool_calls = obj["tool_calls"].as_array().map(|tcs| {
                tcs.iter().map(|tc| ToolCall {
                    id: tc["id"].as_str().unwrap_or("").into(),
                    name: tc["name"].as_str().unwrap_or("").into(),
                    arguments: tc["arguments"].as_str().unwrap_or("{}").into(),
                }).collect()
            });
            self.messages.push(Message { role, content, tool_calls, tool_call_id });
        }
        println!("  Restored session '{}' with {} messages", session_id, self.messages.len());
        Ok(())
    }

    /// 调用 DashScope API（非流式）
    async fn call_model(&self) -> Result<Value, String> {
        let (api_messages, tools_json) = self.prepare_api_call();
        self.llm_client.chat(&api_messages, &tools_json).await
    }

    /// 调用 DashScope API（流式，实时打印 delta）
    async fn call_model_stream(&self) -> Result<Value, String> {
        let (api_messages, tools_json) = self.prepare_api_call();
        self.llm_client
            .chat_stream(&api_messages, &tools_json, |delta| {
                print!("{}", delta);
                use std::io::Write;
                let _ = std::io::stdout().flush();
            })
            .await
    }

    /// 准备 API 调用参数（含经验注入）
    fn prepare_api_call(&self) -> (Vec<Value>, Vec<Value>) {
        // System prompt WITHOUT core memory (to preserve KV cache across turns)
        let mut api_messages = vec![json!({
            "role": "system",
            "content": &self.system_prompt,
        })];

        // Inject core_memory as a prefix to the first user message
        let memory_section = self.core_memory.to_prompt_section();
        let mut memory_injected = false;

        for msg in &self.messages {
            let mut m = json!({ "role": msg.role });

            if let Some(content) = &msg.content {
                if msg.role == "user" && !memory_injected && !memory_section.is_empty() {
                    m["content"] = json!(format!("{}\n\n{}", memory_section, content));
                    memory_injected = true;
                } else {
                    m["content"] = json!(content);
                }
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

        let tools_json = match &self.allowed_tools {
            Some(allowed) if allowed.is_empty() => vec![],
            Some(allowed) => {
                self.tools.to_openai_tools()
                    .into_iter()
                    .filter(|t| {
                        t["function"]["name"].as_str()
                            .map_or(false, |n| allowed.iter().any(|a| a == n))
                    })
                    .collect()
            }
            None => self.tools.to_openai_tools(),
        };

        // 注入工具经验到描述中
        let enriched_tools: Vec<Value> = tools_json
            .into_iter()
            .map(|mut t| {
                if let Some(name) = t["function"]["name"].as_str().map(String::from) {
                    if let Some(desc) = t["function"]["description"].as_str().map(String::from) {
                        let enriched = self.experience.enrich_description(&name, &desc);
                        t["function"]["description"] = json!(enriched);
                    }
                }
                t
            })
            .collect();

        (api_messages, enriched_tools)
    }

    /// 重置对话
    pub fn reset(&mut self) {
        self.messages.clear();
    }

    /// 获取记忆快照
    pub fn memory_snapshot(&self) -> std::collections::HashMap<String, String> {
        self.core_memory.list()
    }

    /// 从历史消息恢复对话上下文
    pub fn restore_messages(&mut self, messages: Vec<Message>) {
        self.messages = messages;
    }
}
