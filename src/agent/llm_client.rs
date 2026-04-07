use reqwest::Client;
use serde_json::{json, Value};
use std::env;

/// 共享的 LLM 客户端，AgentLoop 和 SubAgent 都使用
#[derive(Clone)]
pub struct LLMClient {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
}

/// 重试配置
const MAX_RETRIES: u32 = 3;
const RETRY_DELAY_429_MS: u64 = 2000;
const RETRY_DELAY_5XX_MS: u64 = 1000;

impl LLMClient {
    /// 从环境变量读取配置创建客户端
    pub fn new() -> Self {
        let api_key = env::var("DASHSCOPE_API_KEY").expect("DASHSCOPE_API_KEY not set");
        let base_url = env::var("DASHSCOPE_BASE_URL")
            .unwrap_or_else(|_| "https://dashscope.aliyuncs.com/compatible-mode/v1".into());
        let model = env::var("DASHSCOPE_MODEL").unwrap_or_else(|_| "qwen-plus".into());

        Self {
            client: Client::new(),
            api_key,
            base_url,
            model,
        }
    }

    /// 使用指定模型创建客户端
    pub fn with_model(model: &str) -> Self {
        let mut client = Self::new();
        client.model = model.to_string();
        client
    }

    /// 完整的 chat 调用（带 tools 支持），含 429/5xx 重试
    pub async fn chat(&self, messages: &[Value], tools: &[Value]) -> Result<Value, String> {
        let url = format!("{}/chat/completions", self.base_url);

        let mut body = json!({
            "model": self.model,
            "messages": messages,
        });

        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }

        let mut last_error = String::new();

        for attempt in 0..=MAX_RETRIES {
            if attempt > 0 {
                println!("  [LLM] 第 {} 次重试...", attempt);
            }

            let resp = self
                .client
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("HTTP 请求失败: {}", e))?;

            let status = resp.status();
            let status_code = status.as_u16();
            let text = resp.text().await.map_err(|e| format!("读取响应失败: {}", e))?;

            if status.is_success() {
                return serde_json::from_str(&text)
                    .map_err(|e| format!("JSON 解析失败: {}", e));
            }

            // 判断是否可重试
            let (retryable, delay_ms) = match status_code {
                429 => {
                    println!("  [LLM] 429 Too Many Requests, 等待 {}ms", RETRY_DELAY_429_MS);
                    (true, RETRY_DELAY_429_MS)
                }
                500 | 502 | 503 => {
                    println!("  [LLM] {} Server Error, 等待 {}ms", status_code, RETRY_DELAY_5XX_MS);
                    (true, RETRY_DELAY_5XX_MS)
                }
                _ => (false, 0),
            };

            last_error = format!("API 错误 ({}): {}", status, &text[..text.len().min(200)]);

            if !retryable || attempt == MAX_RETRIES {
                return Err(last_error);
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
        }

        Err(last_error)
    }

    /// 流式 chat 调用 — 逐 delta 通过回调输出
    /// callback 接收每个 delta 的文本片段
    /// 返回完整的最终响应 JSON（模拟非流式格式）
    pub async fn chat_stream<F>(
        &self,
        messages: &[Value],
        tools: &[Value],
        mut on_delta: F,
    ) -> Result<Value, String>
    where
        F: FnMut(&str),
    {
        let url = format!("{}/chat/completions", self.base_url);

        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "stream": true,
        });

        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("HTTP 请求失败: {}", e))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.map_err(|e| format!("读取响应失败: {}", e))?;
            return Err(format!("API 错误 ({}): {}", status, &text[..text.len().min(200)]));
        }

        // 逐行读取 SSE 响应
        let text = resp.text().await.map_err(|e| format!("读取流失败: {}", e))?;

        let mut full_content = String::new();
        let mut finish_reason = String::new();
        let mut tool_calls_acc: Vec<Value> = Vec::new();

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line == "data: [DONE]" {
                continue;
            }

            if let Some(data) = line.strip_prefix("data: ") {
                if let Ok(chunk) = serde_json::from_str::<Value>(data) {
                    if let Some(choices) = chunk["choices"].as_array() {
                        for choice in choices {
                            let delta = &choice["delta"];

                            // 文本内容
                            if let Some(content) = delta["content"].as_str() {
                                on_delta(content);
                                full_content.push_str(content);
                            }

                            // 工具调用累积
                            if let Some(tcs) = delta["tool_calls"].as_array() {
                                for tc in tcs {
                                    let idx = tc["index"].as_u64().unwrap_or(0) as usize;
                                    while tool_calls_acc.len() <= idx {
                                        tool_calls_acc.push(json!({
                                            "id": "",
                                            "type": "function",
                                            "function": {"name": "", "arguments": ""}
                                        }));
                                    }
                                    if let Some(id) = tc["id"].as_str() {
                                        tool_calls_acc[idx]["id"] = json!(id);
                                    }
                                    if let Some(name) = tc["function"]["name"].as_str() {
                                        let old = tool_calls_acc[idx]["function"]["name"]
                                            .as_str()
                                            .unwrap_or("");
                                        tool_calls_acc[idx]["function"]["name"] =
                                            json!(format!("{}{}", old, name));
                                    }
                                    if let Some(args) = tc["function"]["arguments"].as_str() {
                                        let old = tool_calls_acc[idx]["function"]["arguments"]
                                            .as_str()
                                            .unwrap_or("");
                                        tool_calls_acc[idx]["function"]["arguments"] =
                                            json!(format!("{}{}", old, args));
                                    }
                                }
                            }

                            if let Some(fr) = choice["finish_reason"].as_str() {
                                finish_reason = fr.to_string();
                            }
                        }
                    }
                }
            }
        }

        // 组装成非流式格式的响应
        let mut message = json!({
            "role": "assistant",
            "content": if full_content.is_empty() { Value::Null } else { json!(full_content) },
        });

        if !tool_calls_acc.is_empty() {
            message["tool_calls"] = json!(tool_calls_acc);
        }

        let result = json!({
            "choices": [{
                "message": message,
                "finish_reason": finish_reason,
            }],
        });

        Ok(result)
    }

    /// 简单调用（给 Subagent/压缩用），不带 tools
    pub async fn simple_chat(&self, system: &str, user_msg: &str) -> Result<String, String> {
        let messages = vec![
            json!({"role": "system", "content": system}),
            json!({"role": "user", "content": user_msg}),
        ];

        let response = self.chat(&messages, &[]).await?;

        let content = response["choices"]
            .get(0)
            .and_then(|c| c["message"]["content"].as_str())
            .unwrap_or("")
            .to_string();

        Ok(content)
    }

    pub fn model_name(&self) -> &str {
        &self.model
    }
}
