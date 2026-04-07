# IoTClaw 🦞

**智能家居 AI Agent** — 一个用 Rust 编写的 IoT AI Agent，通过自然语言控制智能家居设备。

编译为 ~3.8MB 单二进制文件，无 GC，可部署到 ARM IoT 网关。

**52 个 Rust 源文件 / 9943 行代码**

---

## 功能总览

### 一、核心 Agent 架构

#### Agent Loop 引擎
大模型决策 → 工具执行 → 循环，直到任务完成。支持 Function Calling，模型自主选择调用哪个工具。

#### Subagent 并行
主 Agent 可将复杂任务拆分为子任务，通过 `delegate_task` 工具启动子 Agent。子 Agent 拥有独立上下文，用 tokio 异步并行执行，结果汇总回主 Agent。

#### MCP Client（Model Context Protocol）
实现 Anthropic 提出的标准化工具接入协议，支持两种传输方式：
- **stdio 模式** — JSON-RPC over stdin/stdout，启动子进程通信
- **SSE 模式** — HTTP Server-Sent Events 接收事件流 + HTTP POST 发送请求

通过 `--mcp <command>` 启动时加载外部 MCP Server，自动发现并注册工具，即插即用。

#### Skill 系统
用 Markdown 文件定义 Agent 的角色、行为规则和可用工具：

```markdown
---
name: smart-home
description: 智能家居控制助手
tools: [get_current_time, list_devices, control_device]
---
你是智能家居控制助手。
## 设备安全分级
- 🟢 低风险（灯/窗帘）— 直接执行
- 🟡 中风险（空调）— 需用户确认
- 🔴 高风险（门锁/燃气）— 绝对不自动控制
```

新增能力只需写一个 `.md` 文件放到 `skills/` 目录，不改代码。运行时通过 `/skill <name>` 切换。

#### 流式输出（Streaming）
模型回复实时逐字输出（SSE），CLI 模式下边生成边显示，不需要等全部生成完。

#### Steering 转向中断
CancellationToken 机制，用户发 `/stop` 时中断当前 Agent 处理。每次 Agent Loop 迭代和工具调用前检查取消状态。

#### Provider 多模型系统
统一的 `ModelProvider` trait，支持多个大模型后端：
- **DashScope** — 通义千问（qwen-plus/qwen-turbo）
- **OpenAI 兼容** — DeepSeek、Moonshot、Claude API 等

通过 `.env` 或 `iotclaw.toml` 配置切换，运行时可动态切换 Provider。

#### Hooks 事件钩子
在 Agent 生命周期的关键节点触发自定义逻辑：

| Hook 事件 | 触发时机 |
|-----------|---------|
| `BeforeToolCall` | 工具调用前 |
| `AfterToolCall` | 工具调用后 |
| `BeforeModelCall` | 模型调用前 |
| `AfterModelCall` | 模型调用后 |
| `BeforeChat` | 开始处理用户消息 |
| `AfterChat` | 生成回复后 |
| `OnMessageReceive` | 收到 IM 消息 |
| `OnMessageSend` | 发送 IM 回复 |
| `OnError` | 发生错误 |

内置 `LoggingHook`：所有事件自动写入日志。可自定义 Hook 实现审计、监控、限流等功能。

---

### 二、记忆系统

#### Core Memory
长期用户偏好（姓名、习惯、常用设备），JSON 文件持久化。Agent 可通过 `save_memory` / `recall_memory` 工具读写。每次对话开始时自动加载到 system prompt。

#### Scoped Memory
按 scope_id（群聊 ID / 用户 ID）隔离存储。私聊只搜私聊记忆，A 群不会搜到 B 群的内容。文件结构：`data/memory/{scope_id}.json`。

#### Vector Memory（语义搜索）
调用 DashScope Embedding API（text-embedding-v3）将记忆向量化，余弦相似度 top-K 召回。用户说"上次那个问题"，能通过语义相似度找到，而不是关键词匹配。

---

### 三、上下文管理

大模型有 token 上限，Agent 持续对话会溢出。多层防御：

1. **Token 估算** — 每次调用前检查当前上下文 token 数
2. **LLM 摘要压缩** — 超限时调用轻量模型对历史对话生成摘要，替换原始消息
3. **截断回退** — 模型压缩失败时回退到简单截断
4. **降级链** — 压缩 → 只保留最近 N 轮 → 清空重开

---

### 四、IoT 设备控制

10 台模拟智能设备（灯/窗帘/空调/扫地机/门锁/燃气阀/摄像头/音箱），三级安全分级：

| 等级 | 设备 | Agent 行为 |
|------|------|-----------|
| 🟢 低风险 | 灯、窗帘、音箱 | 直接执行，控错了最多不舒服 |
| 🟡 中风险 | 空调、热水器、扫地机 | 先告知用户，等确认再执行 |
| 🔴 高风险 | 门锁、燃气阀、摄像头 | 绝对不自动控制，只能提醒 |

> 推荐系统的默认策略是"宁可多推"，设备控制的默认策略必须是"没把握就别动"。

#### 中枢网关架构
概念性实现的 IoT 网关模块：
- 设备注册 / 心跳保活 / 状态上报
- MQTT-like 消息格式（topic/payload/qos/retain）
- mDNS 设备发现（UDP 广播）
- MQTT topic 通配符匹配（`+` 和 `#`）

---

### 五、工具体系

#### Slash 命令框架
可扩展的 `/command` 系统，`CommandRegistry` 注册和分发，内置 14 个命令（skill/skills/memory/stop/restart/status/cron/bind 等）。

#### 定时任务（Cron）
`/cron add <间隔秒> <命令>` 添加定时任务，到期自动调 agent.chat() 执行。后台 tokio 循环每秒检查。

#### Observer 系统
子 Agent 行为独立记录。Logger 实现 Observer trait，子 Agent 事件写入 NDJSON 日志（含 agent_id 字段），Session Viewer 可区分显示。

#### 工具经验沉淀
预制 + 可编辑的工具使用经验（`data/tool_experiences.json`），Agent 调用工具前自动注入 tips 到工具描述中，提升调用准确率。

---

### 五、工具体系

共 14+ 个 Function Calling 工具：

| 工具 | 说明 |
|------|------|
| `get_current_time` | 获取当前时间 |
| `get_weather` | 查询城市天气 |
| `list_devices` | 列出家中所有智能设备 |
| `control_device` | 控制设备（含安全分级） |
| `query_device_status` | 查询设备详细状态 |
| `save_memory` | 保存信息到长期记忆 |
| `recall_memory` | 回忆已保存的信息 |
| `remember_fact` | 向量化存储一条信息（Embedding） |
| `search_memory` | 语义搜索相关记忆 |
| `delegate_task` | 委派任务给子 Agent |
| `exec_command` | 执行 shell 命令（沙箱） |
| `take_screenshot` | 截取屏幕或网页截图 |
| `read_feishu_doc` | 读取飞书文档内容 |
| `write_feishu_doc` | 追加内容到飞书文档 |

| `analyze_image` | 图片理解（DashScope qwen-vl-plus 多模态） |

#### Vision 图片理解
调用 DashScope 多模态 API（qwen-vl-plus），支持本地图片（自动转 base64）和 URL。Agent 能"看懂"图片内容。

#### Exec 沙箱
Agent 可执行 shell 命令，但有严格安全限制：
- **命令白名单** — 仅允许 ls、cat、echo、date、curl、python3 等安全命令
- **危险模式拦截** — 自动拦截 rm -rf、sudo、chmod 777、dd、mkfs 等
- **管道/链式命令验证** — 拆分 `|`、`&&`、`;` 逐段检查
- **执行超时** — 可配置超时（默认 10 秒）
- **输出截断** — 超长输出自动截断避免上下文溢出

#### 截图能力
- **桌面截图** — macOS 调用 `screencapture`，Linux 调用 `scrot`
- **网页截图** — 自动检测 Chrome/Chromium，headless 模式截取指定 URL

---

### 六、IM 集成

#### 飞书（完整版）
| 能力 | 说明 |
|------|------|
| OAuth 认证 | tenant_access_token 自动获取 + 缓存 + 刷新 |
| 消息发送 | 文本 / 消息卡片 / 图片 |
| 事件接收 | axum HTTP Server 接收飞书事件回调 |
| 事件加解密 | AES-256-CBC 解密 + SHA256 签名验证 |
| WebSocket | 长连接模式（不需要公网 IP） |
| Typing 状态 | Agent 处理时显示"正在输入..." |
| 合并转发解析 | 解析合并转发消息中的各条原始消息 |
| 消息卡片 | 交互式卡片构建（标题 + 内容 + 按钮） |
| 飞书文档 | 读取和追加飞书文档内容 |
| 飞书插件 | 处理客户端插件交互动作（按钮/表单/选择） |

#### 企业微信（完整版）
| 能力 | 说明 |
|------|------|
| OAuth 认证 | access_token 自动获取 + 缓存 + 刷新（7200s） |
| 消息发送 | 文本 / Markdown / 卡片 |
| 消息接收 | axum HTTP Server 接收企微回调 |
| 加解密 | AES-256-CBC + EncodingAESKey + PKCS7 填充 |
| 签名验证 | SHA1(sort(token, timestamp, nonce, encrypt)) |
| XML 解析 | 完整的企微 XML 消息格式解析与构建 |
| 加密回复 | 回复消息自动加密 + 签名 |

#### 群聊 Smart 策略
- **@检测** — @机器人名字时必回
- **LLM 判断** — 用轻量模型判断"这条消息是不是在跟我说话"
- **Debounce 防抖** — 收到消息后等 2 秒，合并连续消息再处理
- **群成员缓存** — 缓存群成员列表（5 分钟 TTL），用于 @检测

#### 表情包系统
- 27 个关键词到表情的映射
- `enhance_message()` — 自动在回复前添加合适表情
- `detect_sentiment()` — 情感分类（积极/消极/中性/警告/完成/错误）
- 飞书消息 Reaction 支持

---

### 七、工程能力

#### NDJSON 结构化日志
每轮对话完整记录到 `data/logs/{session_id}.ndjson`：
```json
{"timestamp":"2026-04-06T13:29:52+0800","session_id":"cd2d...","role":"user","content":"现在几点"}
{"timestamp":"2026-04-06T13:29:52+0800","session_id":"cd2d...","role":"tool","tool_name":"get_current_time","tool_result":"..."}
{"timestamp":"2026-04-06T13:29:53+0800","session_id":"cd2d...","role":"assistant","content":"现在是..."}
```

#### Session Viewer（Web UI）
内嵌的 Web 页面，`--server` 模式下访问 `http://localhost:PORT/viewer`：
- **💬 实时对话** — 直接在浏览器和 Agent 聊天
- **📋 历史回放** — 查看过往 session 的完整对话过程
- **继续历史对话** — 点击历史 session 自动恢复上下文，继续聊
- 消息按角色颜色区分（用户蓝/助手绿/工具橙/系统灰）

#### TOML 配置系统
支持 `iotclaw.toml` 配置文件（`.env` 优先级更高）：

```toml
[agent]
model = "qwen-turbo"
max_iterations = 10

[memory]
core_memory_path = "data/core_memory.json"
vector_memory_path = "data/vector_memory.json"

[server]
port = 3000

[security]
exec_whitelist = ["ls", "cat", "echo", "date", "curl"]
exec_timeout_secs = 10

[feishu]
webhook = ""
app_id = ""

[wechat]
webhook = ""
```

#### 安全加固
- **Prompt Injection 检测** — 拦截"ignore previous instructions"等注入攻击，支持 base64 编码指令检测
- **出站泄露扫描** — 自动脱敏 Agent 输出中的 API key（sk-*/ghp-*）、.env 变量、内网 IP
- **SSRF/DNS Rebinding 防护** — exec/curl 工具自动检查 URL，拒绝内网地址和危险协议
- **CORS + 安全响应头** — X-Content-Type-Options、X-Frame-Options、X-XSS-Protection
- **Exec 环境变量拦截** — 清除 BASH_ENV、LD_PRELOAD、DYLD_* 等危险变量
- **审批卡片** — 敏感操作（exec_command、高风险设备控制）需用户在飞书点击"同意"后执行

#### SQLite 持久化
群成员、Session 对话、记忆、身份绑定全部用 SQLite 存储，替代纯 JSON 文件。支持 Session 持久化与恢复。

#### 身份映射系统
多平台用户身份绑定（飞书/微信/小米ID），`/bind` 命令支持，实现跨平台记忆融合。

#### Daemon 后台运行
`--daemon` 参数 fork 到后台，PID 文件管理，`--stop` 停止服务。

#### 429 限流重试
API 调用遇到限流或服务端错误时自动重试：
- 429 Too Many Requests → 等 2 秒重试
- 500/502/503 → 等 1 秒重试
- 最多重试 3 次

#### Panic 恢复
Agent 处理消息时如果发生 panic，不会崩溃整个程序：
- CLI 模式：catch_unwind 包裹，打印错误继续接受输入
- Server 模式：外层 spawn 监控内层 JoinHandle，panic 时发送 fallback 回复

---

## 快速开始

### 1. 安装 Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### 2. 配置

```bash
cp .env.example .env
# 编辑 .env，填入你的 DashScope API Key
```

或者使用 TOML 配置：
```bash
cp iotclaw.toml.example iotclaw.toml
# 编辑 iotclaw.toml
```

### 3. 编译运行

```bash
# CLI 交互模式
cargo run

# HTTP Server 模式（Session Viewer + 飞书/微信回调 + Web 对话）
cargo run -- --server

# 飞书 WebSocket 长连接模式
cargo run -- --ws

# 加载 MCP Server
cargo run -- --mcp <command> [args...]

# Release 编译（~2.8MB）
cargo build --release
./target/release/iotclaw
```

### 4. 使用示例

```
🦞 IoTClaw — Smart Home AI Agent

你> 家里有哪些设备？
  🔧 调用工具: list_devices({})
  📋 结果: 设备列表 (10 台):
    客厅灯(客厅) [light_living] 状态:off 风险:🟢低
    卧室空调(卧室) [ac_bedroom] 状态:off 风险:🟡中
    前门门锁(玄关) [lock_front] 状态:locked 风险:🔴高
    ...

🦞> 家里有 10 台设备，包括客厅灯、卧室灯、...

你> 打开客厅灯
  🔧 调用工具: control_device({"device_id":"light_living","action":"on"})
  📋 结果: ✅ 已执行: 客厅灯 → on

🦞> 客厅灯已打开。

你> 帮我把前门打开
🦞> 🔴 前门门锁属于高风险设备，不能自动控制。请手动操作或在 App 中确认。

你> 我叫一平，记住
  🔧 调用工具: save_memory({"key":"姓名","value":"一平"})
  📋 结果: 已记住: 姓名 = 一平

🦞> 好的，已记住你叫一平！
```

## CLI 命令

| 命令 | 说明 |
|------|------|
| `/skill <name>` | 切换 Skill（default/smart-home/chat） |
| `/skills` | 列出所有可用 Skill |
| `/memory` | 查看 Core Memory |
| `/feishu <msg>` | 发送消息到飞书群 |
| `/wechat <msg>` | 发送消息到企业微信 |
| `/reset` | 重置对话上下文 |
| `/help` | 帮助 |
| `/quit` | 退出 |

## 启动参数

| 参数 | 说明 |
|------|------|
| `--server` | HTTP Server 模式（Session Viewer + Web 对话 + IM 回调） |
| `--ws` | 飞书 WebSocket 长连接模式 |
| `--mcp <cmd> [args]` | 加载 MCP Server 并注册其工具 |

## 项目结构

```
iotclaw/                          52 个 Rust 源文件 / 9943 行代码
├── src/
│   ├── main.rs                   # 入口（CLI / Server / WS 模式）
│   ├── config.rs                 # TOML 配置系统
│   ├── hooks.rs                  # Hooks 事件钩子系统
│   ├── logging.rs                # NDJSON 结构化日志
│   ├── viewer.rs                 # Session Viewer + Web 对话
│   ├── gateway.rs                # 中枢网关架构（设备注册/心跳/mDNS）
│   │
│   ├── agent/                    # Agent 核心
│   │   ├── llm_client.rs         # LLM API 调用（流式 + 非流式）
│   │   ├── loop_engine.rs        # Agent Loop 核心引擎
│   │   ├── subagent.rs           # Subagent 并行执行
│   │   ├── provider.rs           # 多模型 Provider 系统
│   │   └── cancellation.rs       # CancellationToken 中断机制
│   │
│   ├── tools/                    # 14+ 个 Function Calling 工具
│   │   ├── registry.rs           # 工具注册表
│   │   ├── time_tool.rs          # 时间查询
│   │   ├── weather.rs            # 天气查询
│   │   ├── iot_device.rs         # IoT 设备控制（安全分级）
│   │   ├── delegate.rs           # 任务委派（Subagent）
│   │   ├── exec.rs               # Shell 命令执行（沙箱 + 环境变量拦截）
│   │   ├── screenshot.rs         # 截图（桌面 + 网页）
│   │   ├── feishu_doc.rs         # 飞书文档读写
│   │   ├── vision.rs             # 图片理解（多模态模型）
│   │   └── experience.rs         # 工具经验沉淀系统
│   │
│   ├── skills/                   # Skill 加载器
│   │   └── loader.rs             # Markdown Skill 解析
│   │
│   ├── memory/                   # 三层记忆系统
│   │   ├── core_memory.rs        # Core Memory（JSON 持久化）
│   │   ├── scoped.rs             # Scoped Memory（按群/用户隔离）
│   │   └── vector.rs             # Vector Memory（Embedding 语义搜索）
│   │
│   ├── context/                  # 上下文管理
│   │   └── manager.rs            # Token 估算 / LLM 压缩 / 降级链
│   │
│   ├── im/                       # IM 集成
│   │   ├── feishu.rs             # 飞书 Webhook
│   │   ├── feishu_full.rs        # 飞书完整 SDK（OAuth/卡片/加密/WS/Typing/合并转发）
│   │   ├── feishu_server.rs      # 飞书事件回调 Server
│   │   ├── feishu_plugin.rs      # 飞书客户端插件
│   │   ├── wechat.rs             # 企微 Webhook
│   │   ├── wechat_full.rs        # 企微完整 SDK（OAuth/AES/XML/签名）
│   │   ├── smart.rs              # 群聊 Smart 策略 + 群成员缓存 + 群名三级缓存
│   │   ├── emoji.rs              # 表情包系统
│   │   ├── voice.rs              # 语音消息处理
│   │   └── approval.rs           # 审批卡片（敏感操作确认）
│   │
│   ├── storage/                  # 数据存储
│   │   └── sqlite.rs             # SQLite 持久化（Session/群成员/记忆/身份）
│   │
│   └── mcp/                      # MCP 协议
│       ├── client.rs             # MCP Client（stdio + SSE 双传输）
│       └── test_server.rs        # MCP 测试 Server
│
├── skills/                       # Skill 定义文件
│   ├── default.md                # 通用助手
│   ├── smart-home.md             # 智能家居控制
│   └── chat.md                   # 纯聊天模式
│
├── Cargo.toml                    # 依赖配置
├── iotclaw.toml.example          # TOML 配置模板
├── .env.example                  # 环境变量模板
├── .gitignore
│
├── src/commands.rs               # Slash 命令框架
├── src/cron.rs                   # 定时任务系统
├── src/security.rs               # 安全扫描（注入/泄露/SSRF）
├── src/identity.rs               # 多平台身份映射
└── src/daemon.rs                 # 后台 Daemon 运行
```

## 技术栈

| 领域 | 技术 |
|------|------|
| **语言** | Rust |
| **异步运行时** | tokio |
| **HTTP 客户端** | reqwest（含 blocking） |
| **HTTP 服务端** | axum |
| **AI 模型** | DashScope (通义千问) — OpenAI 兼容接口 |
| **Embedding** | DashScope text-embedding-v3 |
| **WebSocket** | tokio-tungstenite |
| **加解密** | aes + cbc + sha1 + sha2 + hmac |
| **XML** | quick-xml |
| **配置** | toml + dotenv |
| **序列化** | serde + serde_json |

## 设计理念

### IoT 安全哲学

> IoT 设备控制和内容推荐有本质区别：推荐错了是"烦"，控错了是"吓人甚至危险"。物理世界不能"撤销"。

推荐系统的默认策略是"宁可多推"，设备控制的默认策略必须是"没把握就别动"。

### Agent 架构选择

> Agent = 大模型 + 工具 + 循环。模型负责想，工具负责干，循环让它能连续干好几步。

单一模型扛不住越来越多的能力，所以采用 Subagent 并行架构：主 Agent 做决策和拆分，子 Agent 专注执行具体子任务。

### 记忆设计

> 模型有通用知识但不知道你的事，知识库补的就是这部分。

三层记忆各司其职：Core 记住你是谁，Scoped 记住每个群的上下文，Vector 让"上次那个事"能被语义找到。

### Skill 即配置

> 工具是锤子螺丝刀，Skill 是宜家家具说明书。写个文件就能新增，不用改代码。

这就是"今天不会的事，明天就能学会"。
