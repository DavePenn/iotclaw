#![allow(dead_code, unused_variables, unused_imports)]
mod agent;
mod tools;
mod skills;
mod memory;
mod context;
mod im;
mod logging;
mod mcp;
mod viewer;
mod config;
mod hooks;
mod gateway;
mod storage;
mod identity;
mod daemon;
mod security;
mod commands;
mod cron;

use agent::loop_engine::AgentLoop;
use tools::registry::ToolRegistry;
use skills::loader::SkillLoader;
use memory::core_memory::CoreMemory;
use memory::scoped::{save_scoped_memory_tool, recall_scoped_memory_tool};
use memory::vector::{remember_fact_tool, search_memory_tool};
use im::feishu::FeishuBot;
use im::feishu_full::{FeishuClient, FeishuWsClient};
use im::wechat::WechatBot;
use im::wechat_full::{WechatAppState, WechatClient, WechatCrypto};
use rustyline::DefaultEditor;

use std::sync::Arc;
use tokio::sync::Mutex;

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();

    let args: Vec<String> = std::env::args().collect();
    let server_mode = args.contains(&"--server".to_string());
    let ws_mode = args.contains(&"--ws".to_string());
    let daemon_mode = args.contains(&"--daemon".to_string());
    let stop_mode = args.contains(&"--stop".to_string());
    let mcp_index = args.iter().position(|a| a == "--mcp");

    // Handle --stop: send SIGTERM to running daemon
    if stop_mode {
        match daemon::stop_daemon() {
            Ok(()) => std::process::exit(0),
            Err(e) => {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
    }

    // Handle --daemon: fork to background
    if daemon_mode {
        match daemon::daemonize() {
            Ok(true) => {
                // We are the daemon child, continue execution
            }
            Ok(false) => {
                // We are the parent, exit
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("Daemon error: {}", e);
                std::process::exit(1);
            }
        }
    }

    println!("IoTClaw -- Smart Home AI Agent\n");

    // Initialize SQLite database
    let _ = storage::Database::global();

    // 加载记忆
    let core_memory = CoreMemory::load();

    // 加载 Skills
    println!("Loading Skills...");
    let skill_loader = SkillLoader::load_from_dir("skills");
    println!();

    // 注册工具
    let mut registry = ToolRegistry::new();
    registry.register(tools::time_tool::def());
    registry.register(tools::weather::def());
    registry.register(tools::iot_device::list_devices_tool());
    registry.register(tools::iot_device::control_device_tool());
    registry.register(tools::iot_device::query_device_status_tool());
    registry.register(save_scoped_memory_tool());
    registry.register(recall_scoped_memory_tool());
    registry.register(tools::delegate::def());
    // 向量记忆工具
    registry.register(remember_fact_tool());
    registry.register(search_memory_tool());
    // 新增工具
    registry.register(tools::exec::def());
    registry.register(tools::screenshot::def());
    registry.register(tools::feishu_doc::read_doc_tool());
    registry.register(tools::feishu_doc::write_doc_tool());
    registry.register(tools::vision::def());

    // MCP 工具注册
    let mut _mcp_client: Option<mcp::client::McpClient> = None;
    if let Some(idx) = mcp_index {
        if idx + 1 < args.len() {
            let mcp_command = &args[idx + 1];
            // 收集 --mcp 命令后面的参数（直到下一个 -- 开头的参数）
            let mcp_args: Vec<&str> = args[idx + 2..]
                .iter()
                .take_while(|a| !a.starts_with("--"))
                .map(|s| s.as_str())
                .collect();

            println!("MCP: 连接 {} {:?}", mcp_command, mcp_args);
            match mcp::client::connect_and_list_tools(mcp_command, &mcp_args) {
                Ok((client, tool_defs)) => {
                    for td in tool_defs {
                        registry.register(td);
                    }
                    _mcp_client = Some(client);
                    println!("MCP: 工具已注册\n");
                }
                Err(e) => {
                    eprintln!("MCP: 连接失败: {}\n", e);
                }
            }
        } else {
            eprintln!("用法: --mcp <command> [args...]\n");
        }
    }

    // 创建 Agent
    let mut agent = AgentLoop::new(registry, core_memory);

    // CLI 模式启用流式输出
    if !server_mode && !ws_mode {
        agent.set_streaming(true);
    }

    // 默认加载 default Skill
    if let Some(skill) = skill_loader.get("default") {
        agent.load_skill(skill);
    }

    // Print configured IM channels
    {
        println!("IM 通道:");
        if std::env::var("FEISHU_WEBHOOK").unwrap_or_default().len() > 0 {
            println!("  [OK] 飞书 Webhook");
        }
        if std::env::var("FEISHU_APP_ID").unwrap_or_default().len() > 0 {
            println!("  [OK] 飞书 API (app_id configured)");
        }
        if std::env::var("WECHAT_WEBHOOK").unwrap_or_default().len() > 0 {
            println!("  [OK] 微信 Webhook");
        }
        if std::env::var("WECHAT_CORP_ID").unwrap_or_default().len() > 0 {
            println!("  [OK] 企业微信 API (corp_id configured)");
        }
        println!();
    }

    if ws_mode {
        // 飞书 WebSocket 长连接模式
        println!("启动飞书 WebSocket 模式\n");

        let ws_client = FeishuWsClient::from_env();
        let feishu_client = FeishuClient::from_env();

        match (ws_client, feishu_client) {
            (Some(ws), Some(fc)) => {
                let agent_mutex = Arc::new(Mutex::new(agent));
                println!("Feishu WS: 正在连接...\n");
                if let Err(e) = ws.run(agent_mutex, fc).await {
                    eprintln!("Feishu WS 退出: {}", e);
                }
            }
            _ => {
                eprintln!("错误: --ws 模式需要配置 FEISHU_APP_ID 和 FEISHU_APP_SECRET");
            }
        }
    } else if server_mode {
        // HTTP Server 模式（飞书 + 微信回调 + Session Viewer）
        let port: u16 = std::env::var("FEISHU_SERVER_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(3000);

        println!("启动 HTTP Server 模式 (port={})\n", port);

        // Feishu state + routes
        let feishu_state = Arc::new(im::feishu_server::AppState {
            agent: Mutex::new(agent),
        });

        let feishu_router = axum::Router::new()
            .route("/feishu/event", axum::routing::post(handle_feishu_event_with_recovery))
            .with_state(feishu_state.clone());

        // WeChat state + routes (if configured)
        let wechat_crypto = WechatCrypto::from_env();
        let wechat_client = WechatClient::from_env();

        // We need a separate agent instance for WeChat since feishu already owns one.
        // For simplicity, share the same agent via a new Arc<Mutex>.
        // Re-create agent for WeChat callback
        let wechat_registry = {
            let mut r = ToolRegistry::new();
            r.register(tools::time_tool::def());
            r.register(tools::weather::def());
            r.register(tools::iot_device::list_devices_tool());
            r.register(tools::iot_device::control_device_tool());
            r.register(tools::iot_device::query_device_status_tool());
            r.register(save_scoped_memory_tool());
            r.register(recall_scoped_memory_tool());
            r.register(tools::delegate::def());
            r.register(remember_fact_tool());
            r.register(search_memory_tool());
            r.register(tools::exec::def());
            r.register(tools::screenshot::def());
            r.register(tools::feishu_doc::read_doc_tool());
            r.register(tools::feishu_doc::write_doc_tool());
            r.register(tools::vision::def());
            r
        };
        let wechat_core_memory = CoreMemory::load();
        let mut wechat_agent = AgentLoop::new(wechat_registry, wechat_core_memory);
        if let Some(skill) = skill_loader.get("default") {
            wechat_agent.load_skill(skill);
        }

        let wechat_state = Arc::new(WechatAppState {
            agent: Mutex::new(wechat_agent),
            crypto: wechat_crypto,
            client: wechat_client,
        });

        let wechat_router = axum::Router::new()
            .route(
                "/wechat/callback",
                axum::routing::get(im::wechat_full::wechat_verify)
                    .post(im::wechat_full::wechat_callback),
            )
            .with_state(wechat_state);

        // Viewer + Chat routes (共享 agent)
        let chat_agent = {
            let mut chat_registry = ToolRegistry::new();
            chat_registry.register(tools::time_tool::def());
            chat_registry.register(tools::weather::def());
            chat_registry.register(tools::iot_device::list_devices_tool());
            chat_registry.register(tools::iot_device::control_device_tool());
            chat_registry.register(tools::iot_device::query_device_status_tool());
            chat_registry.register(save_scoped_memory_tool());
            chat_registry.register(recall_scoped_memory_tool());
            chat_registry.register(tools::delegate::def());
            chat_registry.register(memory::vector::remember_fact_tool());
            chat_registry.register(memory::vector::search_memory_tool());
            chat_registry.register(tools::exec::def());
            chat_registry.register(tools::screenshot::def());
            chat_registry.register(tools::feishu_doc::read_doc_tool());
            chat_registry.register(tools::feishu_doc::write_doc_tool());
            chat_registry.register(tools::vision::def());
            let chat_memory = CoreMemory::load();
            let mut a = AgentLoop::new(chat_registry, chat_memory);
            if let Some(skill) = skill_loader.get("default") {
                a.load_skill(skill);
            }
            Arc::new(Mutex::new(a))
        };

        let app = feishu_router
            .merge(wechat_router)
            .merge(viewer::viewer_routes_with_chat(chat_agent))
            .layer(tower_http::cors::CorsLayer::permissive())
            .layer(axum::middleware::from_fn(security_headers_middleware));

        let addr = format!("0.0.0.0:{}", port);
        println!("Server 启动: http://{}", addr);
        println!("  - POST /feishu/event     (飞书事件回调)");
        println!("  - GET  /wechat/callback  (企微 URL 验证)");
        println!("  - POST /wechat/callback  (企微消息回调)");
        println!("  - GET  /viewer           (对话 + Session Viewer)");
        println!("  - POST /api/chat         (Web 对话接口)");
        println!("  - GET  /api/sessions     (日志列表)");
        println!("  - GET  /api/session/:id  (日志详情)\n");

        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .expect("绑定端口失败");

        axum::serve(listener, app)
            .await
            .expect("HTTP Server 启动失败");
    } else {
        // CLI 交互模式
        let cancellation_token = agent.cancellation_token().clone();
        let cron_manager = cron::CronManager::new();

        println!("   输入消息开始对话");
        println!("   /skill <name>  切换 Skill");
        println!("   /skills        列出所有 Skill");
        println!("   /feishu <msg>  发送消息到飞书");
        println!("   /wechat <msg>  发送消息到微信");
        println!("   /status        查看状态");
        println!("   /stop          中断当前处理");
        println!("   /cron          定时任务管理");
        println!("   /reset         重置对话");
        println!("   /quit          退出\n");

        let mut rl = DefaultEditor::new().expect("readline 初始化失败");

        loop {
            let readline = rl.readline("你> ");
            match readline {
                Ok(line) => {
                    let input = line.trim();
                    if input.is_empty() {
                        continue;
                    }

                    if input.starts_with('/') {
                        let parts: Vec<&str> = input.splitn(2, ' ').collect();
                        match parts[0] {
                            "/quit" | "/exit" | "/q" => {
                                println!("再见!");
                                break;
                            }
                            "/stop" => {
                                cancellation_token.cancel();
                                println!("已发送中断信号\n");
                                continue;
                            }
                            "/restart" => {
                                // 重新创建 Agent
                                let new_memory = CoreMemory::load();
                                let mut new_registry = ToolRegistry::new();
                                new_registry.register(tools::time_tool::def());
                                new_registry.register(tools::weather::def());
                                new_registry.register(tools::iot_device::list_devices_tool());
                                new_registry.register(tools::iot_device::control_device_tool());
                                new_registry.register(tools::iot_device::query_device_status_tool());
                                new_registry.register(save_scoped_memory_tool());
                                new_registry.register(recall_scoped_memory_tool());
                                new_registry.register(tools::delegate::def());
                                new_registry.register(remember_fact_tool());
                                new_registry.register(search_memory_tool());
                                new_registry.register(tools::exec::def());
                                new_registry.register(tools::screenshot::def());
                                new_registry.register(tools::feishu_doc::read_doc_tool());
                                new_registry.register(tools::feishu_doc::write_doc_tool());
                                new_registry.register(tools::vision::def());
                                agent = AgentLoop::new(new_registry, new_memory);
                                agent.set_streaming(true);
                                // 重新加载 Skills
                                let new_skill_loader = SkillLoader::load_from_dir("skills");
                                if let Some(skill) = new_skill_loader.get("default") {
                                    agent.load_skill(skill);
                                }
                                println!("Agent 已重启\n");
                                continue;
                            }
                            "/status" => {
                                println!("{}\n", agent.status_info());
                                continue;
                            }
                            "/cron" => {
                                let sub_parts: Vec<&str> = if parts.len() > 1 {
                                    parts[1].splitn(3, ' ').collect()
                                } else {
                                    vec![]
                                };
                                match sub_parts.first().copied().unwrap_or("help") {
                                    "add" => {
                                        if sub_parts.len() < 3 {
                                            println!("用法: /cron add <interval_secs> <command>\n");
                                            continue;
                                        }
                                        let interval: u64 = match sub_parts[1].parse() {
                                            Ok(n) => n,
                                            Err(_) => {
                                                println!("间隔秒数无效\n");
                                                continue;
                                            }
                                        };
                                        let command = sub_parts[2];
                                        let name = format!("cron_{}", chrono::Local::now().timestamp());
                                        tokio::task::block_in_place(|| {
                                            tokio::runtime::Handle::current().block_on(
                                                cron_manager.add(&name, interval, command)
                                            )
                                        }).ok();
                                        println!("已添加定时任务: {} (每 {}s 执行: {})\n", name, interval, command);
                                    }
                                    "list" => {
                                        let jobs = tokio::task::block_in_place(|| {
                                            tokio::runtime::Handle::current().block_on(cron_manager.list())
                                        });
                                        if jobs.is_empty() {
                                            println!("暂无定时任务\n");
                                        } else {
                                            println!("定时任务:");
                                            for job in &jobs {
                                                println!("  {} -- 每 {}s -- {}", job.name, job.interval_secs, job.command);
                                            }
                                            println!();
                                        }
                                    }
                                    "remove" => {
                                        if sub_parts.len() < 2 {
                                            println!("用法: /cron remove <name>\n");
                                            continue;
                                        }
                                        match tokio::task::block_in_place(|| {
                                            tokio::runtime::Handle::current().block_on(
                                                cron_manager.remove(sub_parts[1])
                                            )
                                        }) {
                                            Ok(()) => println!("已删除任务: {}\n", sub_parts[1]),
                                            Err(e) => println!("{}\n", e),
                                        }
                                    }
                                    _ => {
                                        println!("  /cron add <interval_secs> <command>");
                                        println!("  /cron list");
                                        println!("  /cron remove <name>\n");
                                    }
                                }
                                continue;
                            }
                            "/reset" => {
                                agent.reset();
                                println!("对话已重置\n");
                                continue;
                            }
                            "/skills" => {
                                println!("可用 Skills:");
                                for s in skill_loader.list() {
                                    println!("   {} -- {}", s.name, s.description);
                                }
                                println!();
                                continue;
                            }
                            "/skill" => {
                                if parts.len() < 2 {
                                    println!("用法: /skill <name>\n");
                                    continue;
                                }
                                match skill_loader.get(parts[1].trim()) {
                                    Some(skill) => {
                                        agent.load_skill(skill);
                                        println!();
                                    }
                                    None => println!("未找到 Skill: {}\n", parts[1]),
                                }
                                continue;
                            }
                            "/memory" => {
                                let fresh = CoreMemory::load();
                                let mem = fresh.list();
                                if mem.is_empty() {
                                    println!("暂无记忆\n");
                                } else {
                                    println!("Core Memory:");
                                    for (k, v) in &mem {
                                        println!("   {} = {}", k, v);
                                    }
                                    println!();
                                }
                                continue;
                            }
                            "/feishu" => {
                                if parts.len() < 2 {
                                    println!("用法: /feishu <msg>\n");
                                    continue;
                                }
                                let msg = parts[1].trim();
                                let bot = FeishuBot::from_env();
                                match bot.send(msg).await {
                                    Ok(()) => println!("已发送到飞书: {}\n", msg),
                                    Err(e) => println!("飞书发送失败: {}\n", e),
                                }
                                continue;
                            }
                            "/wechat" => {
                                if parts.len() < 2 {
                                    println!("用法: /wechat <msg>\n");
                                    continue;
                                }
                                let msg = parts[1].trim();
                                let bot = WechatBot::from_env();
                                match bot.send(msg).await {
                                    Ok(()) => println!("已发送到微信: {}\n", msg),
                                    Err(e) => println!("微信发送失败: {}\n", e),
                                }
                                continue;
                            }
                            "/bind" => {
                                if parts.len() < 2 {
                                    println!("用法: /bind <platform> <platform_user_id> <master_id>\n");
                                    continue;
                                }
                                let result = identity::IdentityManager::handle_bind_command(parts[1].trim());
                                println!("{}\n", result);
                                continue;
                            }
                            "/restore" => {
                                if parts.len() < 2 {
                                    println!("用法: /restore <session_id>\n");
                                    continue;
                                }
                                match agent.restore_session(parts[1].trim()) {
                                    Ok(()) => println!("Session restored\n"),
                                    Err(e) => println!("Restore failed: {}\n", e),
                                }
                                continue;
                            }
                            "/help" => {
                                println!("  /skill <name>  切换 Skill");
                                println!("  /skills        列出所有 Skill");
                                println!("  /memory        查看记忆");
                                println!("  /feishu <msg>  发送消息到飞书");
                                println!("  /wechat <msg>  发送消息到微信");
                                println!("  /status        查看状态");
                                println!("  /stop          中断当前处理");
                                println!("  /restart       重启 Agent");
                                println!("  /cron          定时任务管理");
                                println!("  /bind          绑定身份");
                                println!("  /restore <id>  恢复对话");
                                println!("  /reset         重置对话");
                                println!("  /quit          退出\n");
                                continue;
                            }
                            _ => {
                                println!("未知命令: {}\n", parts[0]);
                                continue;
                            }
                        }
                    }

                    let _ = rl.add_history_entry(input);

                    // Panic recovery: 使用 AssertUnwindSafe + catch_unwind 保护 agent.chat()
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        // 在 catch_unwind 里不能直接用 async，用 block_on 包裹
                        // 但我们已经在 tokio runtime 中，所以用 block_in_place
                        tokio::task::block_in_place(|| {
                            tokio::runtime::Handle::current().block_on(agent.chat(input))
                        })
                    }));

                    match result {
                        Ok(Ok(reply)) => println!("\n> {}\n", reply),
                        Ok(Err(e)) => eprintln!("错误: {}\n", e),
                        Err(panic_err) => {
                            let panic_msg = if let Some(s) = panic_err.downcast_ref::<&str>() {
                                s.to_string()
                            } else if let Some(s) = panic_err.downcast_ref::<String>() {
                                s.clone()
                            } else {
                                "未知 panic".to_string()
                            };
                            eprintln!("Agent panic 已恢复: {}\n继续接受输入...\n", panic_msg);
                        }
                    }
                }
                Err(_) => {
                    println!("再见!");
                    break;
                }
            }
        }
    }
}

/// Server 模式的飞书事件处理（带 panic recovery）
async fn handle_feishu_event_with_recovery(
    axum::extract::State(state): axum::extract::State<Arc<im::feishu_server::AppState>>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> (axum::http::StatusCode, axum::Json<serde_json::Value>) {
    use serde_json::json;

    // 飞书 URL 验证请求
    if let Some(challenge) = body.get("challenge").and_then(|v| v.as_str()) {
        println!("  Feishu: URL 验证请求, challenge={}", challenge);
        return (
            axum::http::StatusCode::OK,
            axum::Json(json!({ "challenge": challenge })),
        );
    }

    let event_type = body
        .pointer("/header/event_type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    if event_type != "im.message.receive_v1" {
        return (axum::http::StatusCode::OK, axum::Json(json!({ "code": 0 })));
    }

    let message_type = body
        .pointer("/event/message/message_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if message_type != "text" {
        return (axum::http::StatusCode::OK, axum::Json(json!({ "code": 0 })));
    }

    let content_str = body
        .pointer("/event/message/content")
        .and_then(|v| v.as_str())
        .unwrap_or("{}");

    let content_json: serde_json::Value =
        serde_json::from_str(content_str).unwrap_or(json!({}));
    let user_text = content_json["text"]
        .as_str()
        .unwrap_or("")
        .to_string();

    if user_text.is_empty() {
        return (axum::http::StatusCode::OK, axum::Json(json!({ "code": 0 })));
    }

    let sender = body
        .pointer("/event/sender/sender_id/open_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    println!("  Feishu: 收到消息 from={} text={}", sender, user_text);

    // 异步处理，带 panic recovery
    // tokio::spawn 会捕获 panic，JoinHandle 返回 Err(JoinError)
    let state_clone = state.clone();
    let inner_handle = tokio::spawn(async move {
        let mut agent = state_clone.agent.lock().await;
        agent.chat(&user_text).await
    });

    tokio::spawn(async move {
        match inner_handle.await {
            Ok(Ok(reply)) => {
                println!("  Feishu: Agent 回复: {}", &reply[..reply.len().min(100)]);
                let bot = FeishuBot::from_env();
                if let Err(e) = bot.send(&reply).await {
                    eprintln!("  Feishu: 回复发送失败: {}", e);
                }
            }
            Ok(Err(e)) => {
                eprintln!("  Feishu: Agent 错误: {}", e);
                let bot = FeishuBot::from_env();
                let _ = bot.send(&format!("处理出错: {}", e)).await;
            }
            Err(join_err) => {
                eprintln!("  Feishu: Agent panic 已恢复: {}", join_err);
                let bot = FeishuBot::from_env();
                let _ = bot.send("系统内部错误，请稍后重试").await;
            }
        }
    });

    (axum::http::StatusCode::OK, axum::Json(json!({ "code": 0 })))
}

/// 安全响应头中间件
async fn security_headers_middleware(
    request: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert("X-Content-Type-Options", "nosniff".parse().unwrap());
    headers.insert("X-Frame-Options", "DENY".parse().unwrap());
    headers.insert("X-XSS-Protection", "1; mode=block".parse().unwrap());
    response
}
