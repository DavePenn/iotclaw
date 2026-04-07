use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::fs;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::agent::loop_engine::AgentLoop;

const LOG_DIR: &str = "data/logs";

/// 共享 Agent 状态
pub type SharedAgent = Arc<Mutex<AgentLoop>>;

/// 创建 viewer 路由（不带 chat）
pub fn viewer_routes() -> Router {
    Router::new()
        .route("/viewer", get(viewer_page))
        .route("/api/sessions", get(list_sessions))
        .route("/api/session/{id}", get(get_session))
}

/// 创建带 chat 功能的 viewer 路由
pub fn viewer_routes_with_chat(agent: SharedAgent) -> Router {
    Router::new()
        .route("/viewer", get(viewer_page))
        .route("/api/sessions", get(list_sessions))
        .route("/api/session/{id}", get(get_session))
        .route("/api/chat", post(chat_handler))
        .route("/api/chat/reset", post(chat_reset_handler))
        .route("/api/chat/load/{id}", post(chat_load_session))
        .with_state(agent)
}

/// POST /api/chat — 对话接口
async fn chat_handler(
    State(agent): State<SharedAgent>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let message = body["message"].as_str().unwrap_or("").to_string();
    if message.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "message is required"})),
        );
    }

    let mut agent = agent.lock().await;
    match agent.chat(&message).await {
        Ok(reply) => (StatusCode::OK, Json(json!({"reply": reply}))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e})),
        ),
    }
}

/// POST /api/chat/reset — 重置对话
async fn chat_reset_handler(State(agent): State<SharedAgent>) -> impl IntoResponse {
    let mut agent = agent.lock().await;
    agent.reset();
    (StatusCode::OK, Json(json!({"status": "reset"})))
}

/// POST /api/chat/load/:id — 从历史日志恢复对话上下文
async fn chat_load_session(
    State(agent): State<SharedAgent>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if id.contains('/') || id.contains('\\') || id.contains("..") {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": "invalid id"})));
    }

    let file_path = format!("{}/{}.ndjson", LOG_DIR, id);
    let content = match fs::read_to_string(&file_path) {
        Ok(c) => c,
        Err(_) => return (StatusCode::NOT_FOUND, Json(json!({"error": "session not found"}))),
    };

    // 从 NDJSON 重建 messages
    let mut messages = Vec::new();
    for line in content.lines() {
        if let Ok(entry) = serde_json::from_str::<Value>(line) {
            let role = entry["role"].as_str().unwrap_or("");
            let content = entry["content"].as_str().unwrap_or("");
            match role {
                "user" | "assistant" => {
                    if !content.is_empty() {
                        messages.push(crate::agent::loop_engine::Message {
                            role: role.to_string(),
                            content: Some(content.to_string()),
                            tool_calls: None,
                            tool_call_id: None,
                        });
                    }
                }
                _ => {} // 跳过 tool/model_response 等，简化重建
            }
        }
    }

    let msg_count = messages.len();
    let mut agent = agent.lock().await;
    agent.restore_messages(messages);

    (StatusCode::OK, Json(json!({"status": "loaded", "messages": msg_count})))
}

/// GET /viewer
async fn viewer_page() -> Html<&'static str> {
    Html(VIEWER_HTML)
}

/// GET /api/sessions
async fn list_sessions() -> impl IntoResponse {
    let mut sessions: Vec<Value> = Vec::new();

    if let Ok(entries) = fs::read_dir(LOG_DIR) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("ndjson") {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();
                let metadata = fs::metadata(&path).ok();
                let modified = metadata
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let size = fs::metadata(&path).ok().map(|m| m.len()).unwrap_or(0);

                sessions.push(json!({
                    "id": name,
                    "file": format!("{}.ndjson", name),
                    "modified": modified,
                    "size": size,
                }));
            }
        }
    }

    sessions.sort_by(|a, b| {
        let ta = a["modified"].as_u64().unwrap_or(0);
        let tb = b["modified"].as_u64().unwrap_or(0);
        tb.cmp(&ta)
    });

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(&json!({ "sessions": sessions })).unwrap_or_default(),
    )
}

/// GET /api/session/:id
async fn get_session(Path(id): Path<String>) -> impl IntoResponse {
    if id.contains('/') || id.contains('\\') || id.contains("..") {
        return (
            StatusCode::BAD_REQUEST,
            [(header::CONTENT_TYPE, "application/json")],
            json!({"error": "invalid session id"}).to_string(),
        );
    }

    let file_path = format!("{}/{}.ndjson", LOG_DIR, id);

    match fs::read_to_string(&file_path) {
        Ok(content) => {
            let lines: Vec<Value> = content
                .lines()
                .filter(|l| !l.trim().is_empty())
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect();

            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                json!({ "id": id, "messages": lines }).to_string(),
            )
        }
        Err(_) => (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "application/json")],
            json!({"error": "session not found"}).to_string(),
        ),
    }
}

const VIEWER_HTML: &str = r##"<!DOCTYPE html>
<html lang="zh">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>IoTClaw 🦞</title>
<style>
* { box-sizing: border-box; margin: 0; padding: 0; }
body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; background: #1a1a2e; color: #e0e0e0; display: flex; height: 100vh; }

/* Sidebar */
#sidebar { width: 260px; background: #16213e; border-right: 1px solid #0f3460; display: flex; flex-direction: column; flex-shrink: 0; }
#sidebar-header { padding: 16px; border-bottom: 1px solid #0f3460; }
#sidebar-header h2 { color: #e94560; font-size: 18px; }
.tab-bar { display: flex; margin-top: 10px; gap: 4px; }
.tab-btn { flex: 1; padding: 6px; border: 1px solid #0f3460; background: transparent; color: #888; border-radius: 4px; cursor: pointer; font-size: 12px; }
.tab-btn.active { background: #0f3460; color: #e94560; border-color: #e94560; }
#session-list { flex: 1; overflow-y: auto; padding: 10px; }
.session-item { padding: 10px; margin-bottom: 6px; background: #1a1a2e; border-radius: 6px; cursor: pointer; border: 1px solid transparent; transition: all 0.2s; }
.session-item:hover { border-color: #e94560; }
.session-item.active { border-color: #e94560; background: #0f3460; }
.session-item .id { font-size: 11px; color: #888; font-family: monospace; word-break: break-all; }
.session-item .meta { font-size: 11px; color: #666; margin-top: 4px; }

/* Main */
#main { flex: 1; display: flex; flex-direction: column; overflow: hidden; }
#header { padding: 10px 20px; background: #16213e; border-bottom: 1px solid #0f3460; font-size: 14px; color: #888; display: flex; align-items: center; justify-content: space-between; }
#header .title { font-weight: bold; }
#header button { padding: 4px 12px; background: #e94560; border: none; color: white; border-radius: 4px; cursor: pointer; font-size: 12px; }
#header button:hover { background: #c73e54; }

/* Messages */
#messages { flex: 1; overflow-y: auto; padding: 20px; }
.msg { margin-bottom: 12px; padding: 12px 16px; border-radius: 8px; max-width: 80%; line-height: 1.6; white-space: pre-wrap; word-break: break-word; }
.msg.user { background: #0f3460; margin-left: auto; border: 1px solid #1a5276; }
.msg.assistant { background: #1b4332; border: 1px solid #2d6a4f; }
.msg.tool { background: #3d1f00; border: 1px solid #6b3a00; font-family: monospace; font-size: 13px; }
.msg.model_response { background: #2a1a3e; border: 1px solid #4a2d6e; font-size: 12px; }
.msg.system { background: #333; border: 1px solid #555; font-size: 12px; color: #aaa; text-align: center; max-width: 100%; }
.msg .role { font-size: 11px; font-weight: bold; text-transform: uppercase; margin-bottom: 4px; }
.msg.user .role { color: #5dade2; }
.msg.assistant .role { color: #58d68d; }
.msg.tool .role { color: #f0b27a; }
.tool-info { font-size: 12px; color: #aaa; margin-bottom: 6px; }
.empty { text-align: center; color: #666; margin-top: 40px; }
.loading { color: #e94560; }

/* Chat Input */
#chat-input-area { padding: 12px 20px; background: #16213e; border-top: 1px solid #0f3460; display: none; }
#chat-input-area.show { display: flex; gap: 8px; }
#chat-input { flex: 1; padding: 10px 14px; background: #1a1a2e; border: 1px solid #0f3460; border-radius: 6px; color: #e0e0e0; font-size: 14px; outline: none; }
#chat-input:focus { border-color: #e94560; }
#chat-input::placeholder { color: #555; }
#send-btn { padding: 10px 20px; background: #e94560; border: none; color: white; border-radius: 6px; cursor: pointer; font-size: 14px; font-weight: bold; }
#send-btn:hover { background: #c73e54; }
#send-btn:disabled { background: #555; cursor: not-allowed; }
</style>
</head>
<body>
<div id="sidebar">
  <div id="sidebar-header">
    <h2>🦞 IoTClaw</h2>
    <div class="tab-bar">
      <button class="tab-btn active" onclick="switchTab('chat')">💬 对话</button>
      <button class="tab-btn" onclick="switchTab('history')">📋 历史</button>
    </div>
  </div>
  <div id="session-list"></div>
</div>
<div id="main">
  <div id="header">
    <span class="title">💬 实时对话</span>
    <button onclick="resetChat()">🔄 重置</button>
  </div>
  <div id="messages"><div class="empty">发送消息开始对话 🦞</div></div>
  <div id="chat-input-area" class="show">
    <input type="text" id="chat-input" placeholder="输入消息... (Enter 发送)" onkeydown="if(event.key==='Enter')sendMessage()">
    <button id="send-btn" onclick="sendMessage()">发送</button>
  </div>
</div>

<script>
let currentTab = 'chat';
let chatMessages = [];

function switchTab(tab) {
  currentTab = tab;
  document.querySelectorAll('.tab-btn').forEach(b => b.classList.remove('active'));
  event.target.classList.add('active');

  const inputArea = document.getElementById('chat-input-area');
  const header = document.getElementById('header');

  if (tab === 'chat') {
    inputArea.classList.add('show');
    header.innerHTML = '<span class="title">💬 实时对话</span><button onclick="resetChat()">🔄 重置</button>';
    renderChatMessages();
    loadSessions(); // refresh sidebar
  } else {
    inputArea.classList.remove('show');
    header.innerHTML = '<span class="title">📋 历史记录</span>';
    loadSessions();
    document.getElementById('messages').innerHTML = '<div class="empty">点击左侧 session 查看历史</div>';
  }
}

function renderChatMessages() {
  const container = document.getElementById('messages');
  if (chatMessages.length === 0) {
    container.innerHTML = '<div class="empty">发送消息开始对话 🦞</div>';
    return;
  }
  container.innerHTML = chatMessages.map(m => {
    return `<div class="msg ${m.role}"><div class="role">${m.role}</div><div>${escapeHtml(m.content)}</div></div>`;
  }).join('');
  container.scrollTop = container.scrollHeight;
}

async function sendMessage() {
  const input = document.getElementById('chat-input');
  const btn = document.getElementById('send-btn');
  const msg = input.value.trim();
  if (!msg) return;

  // 显示用户消息
  chatMessages.push({ role: 'user', content: msg });
  renderChatMessages();
  input.value = '';
  btn.disabled = true;
  btn.textContent = '思考中...';

  // 添加加载指示
  const container = document.getElementById('messages');
  container.innerHTML += '<div class="msg assistant" id="loading-msg"><div class="role">assistant</div><div class="loading">🦞 思考中...</div></div>';
  container.scrollTop = container.scrollHeight;

  try {
    const resp = await fetch('/api/chat', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ message: msg })
    });
    const data = await resp.json();

    // 移除加载指示
    const loadingMsg = document.getElementById('loading-msg');
    if (loadingMsg) loadingMsg.remove();

    if (data.reply) {
      chatMessages.push({ role: 'assistant', content: data.reply });
    } else if (data.error) {
      chatMessages.push({ role: 'system', content: '❌ ' + data.error });
    }
    renderChatMessages();
  } catch (e) {
    const loadingMsg = document.getElementById('loading-msg');
    if (loadingMsg) loadingMsg.remove();
    chatMessages.push({ role: 'system', content: '❌ 网络错误: ' + e.message });
    renderChatMessages();
  }

  btn.disabled = false;
  btn.textContent = '发送';
  input.focus();
}

async function resetChat() {
  try {
    await fetch('/api/chat/reset', { method: 'POST' });
    chatMessages = [];
    renderChatMessages();
  } catch(e) {
    alert('重置失败: ' + e.message);
  }
}

async function loadSessions() {
  const list = document.getElementById('session-list');
  try {
    const resp = await fetch('/api/sessions');
    const data = await resp.json();
    if (!data.sessions || data.sessions.length === 0) {
      list.innerHTML = '<div class="empty">暂无历史</div>';
      return;
    }
    list.innerHTML = data.sessions.map(s => {
      const date = s.modified ? new Date(s.modified * 1000).toLocaleString() : '';
      const kb = (s.size / 1024).toFixed(1);
      return `<div class="session-item" onclick="loadSession('${s.id}', this)">
        <div class="id">${s.id.substring(0, 8)}...</div>
        <div class="meta">${date} · ${kb}KB</div>
      </div>`;
    }).join('');
  } catch(e) {
    list.innerHTML = '<div class="empty">加载失败</div>';
  }
}

async function loadSession(id, el) {
  document.querySelectorAll('.session-item').forEach(e => e.classList.remove('active'));
  if (el) el.classList.add('active');

  // 直接加载历史并进入可继续对话状态
  await continueSession(id);
}

async function continueSession(id) {
  try {
    const resp = await fetch('/api/chat/load/' + id, { method: 'POST' });
    const data = await resp.json();
    if (data.status === 'loaded') {
      // 从历史消息重建 chatMessages
      const sessionResp = await fetch('/api/session/' + id);
      const sessionData = await sessionResp.json();
      chatMessages = (sessionData.messages || [])
        .filter(m => m.role === 'user' || m.role === 'assistant')
        .filter(m => m.content)
        .map(m => ({ role: m.role, content: m.content }));
      // 切换到对话 tab
      currentTab = 'chat';
      document.querySelectorAll('.tab-btn').forEach(b => b.classList.remove('active'));
      document.querySelectorAll('.tab-btn')[0].classList.add('active');
      document.getElementById('chat-input-area').classList.add('show');
      document.getElementById('header').innerHTML = '<span class="title">💬 继续对话（已加载 ' + data.messages + ' 条历史）</span><button onclick="resetChat()">🔄 重置</button>';
      renderChatMessages();
      document.getElementById('chat-input').focus();
    }
  } catch(e) {
    alert('加载失败: ' + e.message);
  }
}

function escapeHtml(text) {
  const div = document.createElement('div');
  div.textContent = text;
  return div.innerHTML;
}

loadSessions();
</script>
</body>
</html>"##;
