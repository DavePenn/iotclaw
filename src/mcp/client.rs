use serde_json::{json, Value};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::tools::registry::ToolDef;

/// MCP Client -- JSON-RPC over stdio
pub struct McpClient {
    child: Child,
    reader: BufReader<std::process::ChildStdout>,
    writer: BufWriter<std::process::ChildStdin>,
    request_id: AtomicU64,
}

impl McpClient {
    /// 启动 MCP Server 子进程并连接
    pub fn connect(command: &str, args: &[&str]) -> Result<Self, String> {
        let mut child = Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("启动 MCP Server 失败: {}", e))?;

        let stdout = child.stdout.take().ok_or("无法获取子进程 stdout")?;
        let stdin = child.stdin.take().ok_or("无法获取子进程 stdin")?;

        Ok(Self {
            child,
            reader: BufReader::new(stdout),
            writer: BufWriter::new(stdin),
            request_id: AtomicU64::new(1),
        })
    }

    /// 发送 JSON-RPC 请求并读取响应
    fn send_request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.request_id.fetch_add(1, Ordering::SeqCst);

        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        let request_str = serde_json::to_string(&request)
            .map_err(|e| format!("序列化请求失败: {}", e))?;

        // 写入 stdin（每行一个 JSON）
        writeln!(self.writer, "{}", request_str)
            .map_err(|e| format!("写入 stdin 失败: {}", e))?;
        self.writer
            .flush()
            .map_err(|e| format!("flush stdin 失败: {}", e))?;

        // 从 stdout 读取一行响应
        let mut line = String::new();
        self.reader
            .read_line(&mut line)
            .map_err(|e| format!("读取 stdout 失败: {}", e))?;

        if line.trim().is_empty() {
            return Err("MCP Server 返回空响应".into());
        }

        let response: Value =
            serde_json::from_str(line.trim()).map_err(|e| format!("解析响应 JSON 失败: {} (raw: {})", e, line.trim()))?;

        // 验证 JSON-RPC 响应格式
        if response.get("jsonrpc").and_then(|v| v.as_str()) != Some("2.0") {
            return Err(format!("无效的 JSON-RPC 响应: 缺少 jsonrpc 字段"));
        }

        // 检查 id 匹配
        if let Some(resp_id) = response.get("id").and_then(|v| v.as_u64()) {
            if resp_id != id {
                return Err(format!("JSON-RPC id 不匹配: 期望 {}, 收到 {}", id, resp_id));
            }
        }

        // 检查错误
        if let Some(error) = response.get("error") {
            let msg = error["message"].as_str().unwrap_or("未知错误");
            let code = error["code"].as_i64().unwrap_or(0);
            return Err(format!("MCP 错误 ({}): {}", code, msg));
        }

        Ok(response["result"].clone())
    }

    /// 发送 initialize 请求
    pub fn initialize(&mut self) -> Result<Value, String> {
        let params = json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "iotclaw",
                "version": "0.1.0"
            }
        });

        let result = self.send_request("initialize", params)?;

        // 发送 initialized 通知（无 id，无需响应）
        let notification = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
        });
        let notif_str = serde_json::to_string(&notification)
            .map_err(|e| format!("序列化通知失败: {}", e))?;
        let _ = writeln!(self.writer, "{}", notif_str);
        let _ = self.writer.flush();

        Ok(result)
    }

    /// 获取 MCP Server 提供的工具列表
    pub fn list_tools(&mut self) -> Result<Vec<McpToolDef>, String> {
        let result = self.send_request("tools/list", json!({}))?;

        let tools_array = result["tools"]
            .as_array()
            .ok_or("tools/list 响应格式错误")?;

        let mut tools = Vec::new();
        for tool in tools_array {
            let name = tool["name"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let description = tool["description"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let input_schema = tool["inputSchema"].clone();

            tools.push(McpToolDef {
                name,
                description,
                input_schema,
            });
        }

        Ok(tools)
    }

    /// 调用 MCP Server 的工具
    pub fn call_tool(&mut self, name: &str, arguments: Value) -> Result<String, String> {
        let params = json!({
            "name": name,
            "arguments": arguments,
        });

        let result = self.send_request("tools/call", params)?;

        // MCP 工具结果在 content 数组中
        if let Some(content) = result["content"].as_array() {
            let texts: Vec<String> = content
                .iter()
                .filter_map(|c| c["text"].as_str().map(|s| s.to_string()))
                .collect();
            if !texts.is_empty() {
                return Ok(texts.join("\n"));
            }
        }

        // fallback: 整个 result 作为字符串
        Ok(serde_json::to_string_pretty(&result).unwrap_or_default())
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// MCP 工具定义
#[derive(Debug, Clone)]
pub struct McpToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

// ─── MCP SSE Client ──────────────────────────────────────────────────

/// MCP Client over SSE (Server-Sent Events) transport
/// SSE endpoint: GET /sse → receive event stream
/// POST endpoint: POST /message → send JSON-RPC requests
pub struct McpSseClient {
    base_url: String,
    client: reqwest::Client,
    request_id: AtomicU64,
    message_endpoint: String,
}

impl McpSseClient {
    /// Create a new SSE client
    pub fn new(base_url: &str) -> Self {
        let base = base_url.trim_end_matches('/').to_string();
        Self {
            message_endpoint: format!("{}/message", base),
            base_url: base,
            client: reqwest::Client::new(),
            request_id: AtomicU64::new(1),
        }
    }

    /// Send a JSON-RPC request via HTTP POST and get response
    pub async fn send_request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.request_id.fetch_add(1, Ordering::SeqCst);

        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        let resp = self.client
            .post(&self.message_endpoint)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| format!("MCP SSE POST failed: {}", e))?;

        let status = resp.status();
        let text = resp.text().await.map_err(|e| format!("MCP SSE read response failed: {}", e))?;

        if !status.is_success() {
            return Err(format!("MCP SSE error ({}): {}", status, &text[..text.len().min(200)]));
        }

        let response: Value = serde_json::from_str(&text)
            .map_err(|e| format!("MCP SSE JSON parse failed: {} (raw: {})", e, &text[..text.len().min(200)]))?;

        // Check JSON-RPC error
        if let Some(error) = response.get("error") {
            let msg = error["message"].as_str().unwrap_or("unknown");
            let code = error["code"].as_i64().unwrap_or(0);
            return Err(format!("MCP SSE error ({}): {}", code, msg));
        }

        Ok(response["result"].clone())
    }

    /// Initialize the MCP connection
    pub async fn initialize(&self) -> Result<Value, String> {
        let params = json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "iotclaw",
                "version": "0.1.0"
            }
        });

        let result = self.send_request("initialize", params).await?;

        // Send initialized notification
        let notification = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
        });

        let _ = self.client
            .post(&self.message_endpoint)
            .header("Content-Type", "application/json")
            .json(&notification)
            .send()
            .await;

        Ok(result)
    }

    /// List tools from MCP Server
    pub async fn list_tools(&self) -> Result<Vec<McpToolDef>, String> {
        let result = self.send_request("tools/list", json!({})).await?;

        let tools_array = result["tools"]
            .as_array()
            .ok_or("tools/list response format error")?;

        let mut tools = Vec::new();
        for tool in tools_array {
            tools.push(McpToolDef {
                name: tool["name"].as_str().unwrap_or("").to_string(),
                description: tool["description"].as_str().unwrap_or("").to_string(),
                input_schema: tool["inputSchema"].clone(),
            });
        }

        Ok(tools)
    }

    /// Call a tool on the MCP Server
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<String, String> {
        let params = json!({
            "name": name,
            "arguments": arguments,
        });

        let result = self.send_request("tools/call", params).await?;

        if let Some(content) = result["content"].as_array() {
            let texts: Vec<String> = content
                .iter()
                .filter_map(|c| c["text"].as_str().map(|s| s.to_string()))
                .collect();
            if !texts.is_empty() {
                return Ok(texts.join("\n"));
            }
        }

        Ok(serde_json::to_string_pretty(&result).unwrap_or_default())
    }

    /// SSE endpoint URL (for connecting to event stream)
    pub fn sse_url(&self) -> String {
        format!("{}/sse", self.base_url)
    }
}

/// Connect to MCP Server via SSE and list tools
pub async fn connect_sse_and_list_tools(base_url: &str) -> Result<(McpSseClient, Vec<ToolDef>), String> {
    let client = McpSseClient::new(base_url);

    println!("  MCP SSE: connecting to {}...", base_url);
    let init_result = client.initialize().await?;
    let server_name = init_result["serverInfo"]["name"].as_str().unwrap_or("unknown");
    println!("  MCP SSE: connected ({})", server_name);

    // Derive prefix from server URL
    let prefix = get_tool_prefix(base_url);

    println!("  MCP SSE: listing tools...");
    let mcp_tools = client.list_tools().await?;
    println!("  MCP SSE: found {} tools (prefix: {})", mcp_tools.len(), prefix);

    let tool_defs: Vec<ToolDef> = mcp_tools
        .iter()
        .map(|t| mcp_tool_to_tooldef_with_prefix(t, &prefix))
        .collect();

    for t in &mcp_tools {
        println!("    - {}_{} : {}", prefix, t.name, t.description);
    }

    Ok((client, tool_defs))
}

/// 将 MCP 工具转换为 ToolRegistry 可注册的 ToolDef
/// Uses default prefix: mcp_{tool_name}
pub fn mcp_tool_to_tooldef(mcp_tool: &McpToolDef) -> ToolDef {
    mcp_tool_to_tooldef_with_prefix(mcp_tool, "mcp")
}

/// Convert MCP tool to ToolDef with a custom server name prefix.
/// Naming: {prefix}_{tool_name} to avoid collisions with built-in tools.
/// The prefix can be configured in config.toml via [mcp] tool_prefix = "..."
pub fn mcp_tool_to_tooldef_with_prefix(mcp_tool: &McpToolDef, server_name: &str) -> ToolDef {
    let name = mcp_tool.name.clone();
    let description = mcp_tool.description.clone();
    let parameters = if mcp_tool.input_schema.is_null() {
        json!({ "type": "object", "properties": {} })
    } else {
        mcp_tool.input_schema.clone()
    };

    let prefixed_name = format!("{}_{}", server_name, name);

    ToolDef {
        name: prefixed_name,
        description: format!("[MCP:{}] {}", server_name, description),
        parameters,
        handler: |args: Value| {
            // MCP 工具的实际调用在 AgentLoop 中通过 McpClient 处理
            // 这里只是占位 handler
            format!(
                "MCP 工具需要通过 McpClient 调用，参数: {}",
                serde_json::to_string(&args).unwrap_or_default()
            )
        },
    }
}

/// Get the tool prefix for an MCP server from config or command name
pub fn get_tool_prefix(command: &str) -> String {
    // Try to get prefix from config.toml [mcp] section
    // For now, derive from the command name
    let cmd_name = command
        .rsplit('/')
        .next()
        .unwrap_or(command)
        .trim_end_matches(".js")
        .trim_end_matches(".py")
        .trim_end_matches(".sh");

    // Sanitize: replace non-alphanumeric with underscore
    let sanitized: String = cmd_name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
        .collect();

    format!("mcp_{}", sanitized)
}

/// 连接 MCP Server 并返回工具列表
pub fn connect_and_list_tools(command: &str, args: &[&str]) -> Result<(McpClient, Vec<ToolDef>), String> {
    let mut client = McpClient::connect(command, args)?;

    // Initialize
    println!("  MCP: 发送 initialize...");
    let init_result = client.initialize()?;
    let server_name = init_result["serverInfo"]["name"]
        .as_str()
        .unwrap_or("unknown");
    println!("  MCP: 已连接 ({})", server_name);

    // Determine tool prefix from server name or command
    let prefix = get_tool_prefix(command);

    // List tools
    println!("  MCP: 获取工具列表...");
    let mcp_tools = client.list_tools()?;
    println!("  MCP: 获得 {} 个工具 (prefix: {})", mcp_tools.len(), prefix);

    let tool_defs: Vec<ToolDef> = mcp_tools
        .iter()
        .map(|t| mcp_tool_to_tooldef_with_prefix(t, &prefix))
        .collect();

    for t in &mcp_tools {
        println!("    - {}_{} : {}", prefix, t.name, t.description);
    }

    Ok((client, tool_defs))
}
