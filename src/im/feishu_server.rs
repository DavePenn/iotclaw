use axum::{
    extract::State,
    http::StatusCode,
    routing::post,
    Json, Router,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::agent::loop_engine::AgentLoop;
use crate::im::feishu::FeishuBot;

/// 飞书 HTTP Server 共享状态
pub struct AppState {
    pub agent: Mutex<AgentLoop>,
}

/// 启动飞书事件回调 HTTP Server
pub async fn start_server(port: u16, agent: AgentLoop) {
    let state = Arc::new(AppState {
        agent: Mutex::new(agent),
    });

    let app = Router::new()
        .route("/feishu/event", post(handle_feishu_event))
        .with_state(state);

    let addr = format!("0.0.0.0:{}", port);
    println!("Feishu Server 启动: http://{}", addr);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("绑定端口失败");

    axum::serve(listener, app)
        .await
        .expect("HTTP Server 启动失败");
}

/// 处理飞书事件回调
async fn handle_feishu_event(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    // 飞书 URL 验证请求
    if let Some(challenge) = body.get("challenge").and_then(|v| v.as_str()) {
        println!("  Feishu: URL 验证请求, challenge={}", challenge);
        return (
            StatusCode::OK,
            Json(json!({ "challenge": challenge })),
        );
    }

    // 飞书事件 v2.0 格式
    // 去重: 检查 header.event_id（这里简化处理，只打印）
    let event_id = body
        .pointer("/header/event_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let event_type = body
        .pointer("/header/event_type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    println!("  Feishu: 收到事件 type={} id={}", event_type, event_id);

    // 只处理 im.message.receive_v1 消息事件
    if event_type != "im.message.receive_v1" {
        return (StatusCode::OK, Json(json!({ "code": 0 })));
    }

    // 解析消息内容
    let message_type = body
        .pointer("/event/message/message_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if message_type != "text" {
        println!("  Feishu: 忽略非文本消息 type={}", message_type);
        return (StatusCode::OK, Json(json!({ "code": 0 })));
    }

    // 飞书消息 content 是 JSON 字符串: {"text":"实际内容"}
    let content_str = body
        .pointer("/event/message/content")
        .and_then(|v| v.as_str())
        .unwrap_or("{}");

    let content_json: Value = serde_json::from_str(content_str).unwrap_or(json!({}));
    let user_text = content_json["text"]
        .as_str()
        .unwrap_or("")
        .to_string();

    if user_text.is_empty() {
        return (StatusCode::OK, Json(json!({ "code": 0 })));
    }

    let sender = body
        .pointer("/event/sender/sender_id/open_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    println!("  Feishu: 收到消息 from={} text={}", sender, user_text);

    // 异步处理: 调用 agent 并回复
    let state_clone = state.clone();
    let user_text_clone = user_text.clone();
    tokio::spawn(async move {
        let mut agent = state_clone.agent.lock().await;
        match agent.chat(&user_text_clone).await {
            Ok(reply) => {
                println!("  Feishu: Agent 回复: {}", &reply[..reply.len().min(100)]);
                // 通过 webhook 发回消息
                let bot = FeishuBot::from_env();
                if let Err(e) = bot.send(&reply).await {
                    eprintln!("  Feishu: 回复发送失败: {}", e);
                }
            }
            Err(e) => {
                eprintln!("  Feishu: Agent 错误: {}", e);
                let bot = FeishuBot::from_env();
                let _ = bot.send(&format!("处理出错: {}", e)).await;
            }
        }
    });

    // 立即返回 200，避免飞书超时重试
    (StatusCode::OK, Json(json!({ "code": 0 })))
}
