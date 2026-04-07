/// 简单的 Echo MCP Server，用于验证 MCP Client 代码
///
/// 运行方式: cargo run -- --mcp-test
/// 或作为独立进程: 读 stdin JSON-RPC，写 stdout JSON-RPC
///
/// 支持的方法:
/// - initialize → 返回 serverInfo
/// - tools/list → 返回 echo 工具定义
/// - tools/call (echo) → 回显输入文本

use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

/// 启动测试 MCP Server（阻塞式，读 stdin 写 stdout）
pub fn run_test_server() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout_lock = stdout.lock();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let request: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue, // 忽略无效 JSON
        };

        // 通知消息没有 id，不需要响应
        if request.get("id").is_none() {
            continue;
        }

        let id = request["id"].clone();
        let method = request["method"].as_str().unwrap_or("");

        let result = match method {
            "initialize" => json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "iotclaw-test-server",
                    "version": "0.1.0"
                }
            }),
            "tools/list" => json!({
                "tools": [
                    {
                        "name": "echo",
                        "description": "Echo back the input text",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "text": {
                                    "type": "string",
                                    "description": "Text to echo"
                                }
                            },
                            "required": ["text"]
                        }
                    }
                ]
            }),
            "tools/call" => {
                let name = request["params"]["name"].as_str().unwrap_or("");
                let arguments = &request["params"]["arguments"];
                match name {
                    "echo" => {
                        let text = arguments["text"].as_str().unwrap_or("");
                        json!({
                            "content": [
                                {
                                    "type": "text",
                                    "text": format!("Echo: {}", text)
                                }
                            ]
                        })
                    }
                    _ => json!({
                        "content": [
                            {
                                "type": "text",
                                "text": format!("Unknown tool: {}", name)
                            }
                        ]
                    }),
                }
            }
            _ => {
                // 未知方法，返回错误
                let response = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {
                        "code": -32601,
                        "message": format!("Method not found: {}", method)
                    }
                });
                let resp_str = serde_json::to_string(&response).unwrap_or_default();
                let _ = writeln!(stdout_lock, "{}", resp_str);
                let _ = stdout_lock.flush();
                continue;
            }
        };

        let response = json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        });

        let resp_str = serde_json::to_string(&response).unwrap_or_default();
        let _ = writeln!(stdout_lock, "{}", resp_str);
        let _ = stdout_lock.flush();
    }
}
