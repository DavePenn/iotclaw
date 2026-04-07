use serde_json::Value;
use std::sync::Arc;

/// Hook event types
#[derive(Debug, Clone)]
pub enum HookEvent {
    /// Before tool call
    BeforeToolCall {
        tool_name: String,
        arguments: Value,
    },
    /// After tool call
    AfterToolCall {
        tool_name: String,
        arguments: Value,
        result: String,
    },
    /// Before model call
    BeforeModelCall {
        messages: Vec<Value>,
    },
    /// After model call
    AfterModelCall {
        response: Value,
    },
    /// Before chat (user input received)
    BeforeChat {
        user_input: String,
    },
    /// After chat (reply generated)
    AfterChat {
        reply: String,
    },
    /// On message receive (from IM channels)
    OnMessageReceive {
        source: String,     // "feishu", "wechat", "cli"
        sender: String,
        content: String,
    },
    /// On message send (reply sent to IM channels)
    OnMessageSend {
        target: String,     // "feishu", "wechat", "cli"
        content: String,
    },
    /// On error
    OnError {
        message: String,
    },
}

impl HookEvent {
    pub fn event_name(&self) -> &str {
        match self {
            HookEvent::BeforeToolCall { .. } => "before_tool_call",
            HookEvent::AfterToolCall { .. } => "after_tool_call",
            HookEvent::BeforeModelCall { .. } => "before_model_call",
            HookEvent::AfterModelCall { .. } => "after_model_call",
            HookEvent::BeforeChat { .. } => "before_chat",
            HookEvent::AfterChat { .. } => "after_chat",
            HookEvent::OnMessageReceive { .. } => "on_message_receive",
            HookEvent::OnMessageSend { .. } => "on_message_send",
            HookEvent::OnError { .. } => "on_error",
        }
    }

    /// Serialize event data to JSON
    pub fn to_value(&self) -> Value {
        use serde_json::json;
        match self {
            HookEvent::BeforeToolCall { tool_name, arguments } => {
                json!({"tool_name": tool_name, "arguments": arguments})
            }
            HookEvent::AfterToolCall { tool_name, arguments, result } => {
                json!({"tool_name": tool_name, "arguments": arguments, "result": result})
            }
            HookEvent::BeforeModelCall { messages } => {
                json!({"message_count": messages.len()})
            }
            HookEvent::AfterModelCall { response } => {
                json!({"response_preview": response.to_string().chars().take(200).collect::<String>()})
            }
            HookEvent::BeforeChat { user_input } => {
                json!({"user_input": user_input})
            }
            HookEvent::AfterChat { reply } => {
                json!({"reply": reply})
            }
            HookEvent::OnMessageReceive { source, sender, content } => {
                json!({"source": source, "sender": sender, "content": content})
            }
            HookEvent::OnMessageSend { target, content } => {
                json!({"target": target, "content": content})
            }
            HookEvent::OnError { message } => {
                json!({"error": message})
            }
        }
    }
}

/// Hook trait -- all hooks must implement
pub trait Hook: Send + Sync {
    fn on_event(&self, event: &HookEvent, data: &Value);
}

// --- Built-in Hooks ---

/// LoggingHook -- logs all events
pub struct LoggingHook;

impl Hook for LoggingHook {
    fn on_event(&self, event: &HookEvent, _data: &Value) {
        let now = chrono::Local::now().format("%H:%M:%S");
        match event {
            HookEvent::BeforeToolCall { tool_name, .. } => {
                println!("  [Hook {}] before_tool_call: {}", now, tool_name);
            }
            HookEvent::AfterToolCall { tool_name, result, .. } => {
                let preview = if result.len() > 80 {
                    format!("{}...", &result[..80])
                } else {
                    result.clone()
                };
                println!("  [Hook {}] after_tool_call: {} -> {}", now, tool_name, preview);
            }
            HookEvent::BeforeModelCall { messages } => {
                println!("  [Hook {}] before_model_call: {} messages", now, messages.len());
            }
            HookEvent::AfterModelCall { .. } => {
                println!("  [Hook {}] after_model_call", now);
            }
            HookEvent::BeforeChat { user_input } => {
                println!("  [Hook {}] before_chat: {}", now, user_input);
            }
            HookEvent::AfterChat { reply } => {
                let preview = if reply.len() > 80 {
                    format!("{}...", &reply[..80])
                } else {
                    reply.clone()
                };
                println!("  [Hook {}] after_chat: {}", now, preview);
            }
            HookEvent::OnMessageReceive { source, sender, content } => {
                println!("  [Hook {}] on_message_receive [{}] from={}: {}", now, source, sender, content);
            }
            HookEvent::OnMessageSend { target, content } => {
                let preview = if content.len() > 80 {
                    format!("{}...", &content[..80])
                } else {
                    content.clone()
                };
                println!("  [Hook {}] on_message_send [{}]: {}", now, target, preview);
            }
            HookEvent::OnError { message } => {
                eprintln!("  [Hook {}] on_error: {}", now, message);
            }
        }
    }
}

// --- Hook Manager ---

pub struct HookManager {
    hooks: Vec<Arc<dyn Hook>>,
}

impl HookManager {
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    /// Create with the default LoggingHook
    pub fn with_logging() -> Self {
        let mut mgr = Self::new();
        mgr.register(Arc::new(LoggingHook));
        mgr
    }

    /// Register a hook
    pub fn register(&mut self, hook: Arc<dyn Hook>) {
        self.hooks.push(hook);
    }

    /// Trigger event to all registered hooks
    pub fn trigger(&self, event: &HookEvent) {
        let data = event.to_value();
        for hook in &self.hooks {
            hook.on_event(event, &data);
        }
    }

    /// Convenience: before_tool_call
    pub fn before_tool_call(&self, tool_name: &str, arguments: &Value) {
        self.trigger(&HookEvent::BeforeToolCall {
            tool_name: tool_name.to_string(),
            arguments: arguments.clone(),
        });
    }

    /// Convenience: after_tool_call
    pub fn after_tool_call(&self, tool_name: &str, arguments: &Value, result: &str) {
        self.trigger(&HookEvent::AfterToolCall {
            tool_name: tool_name.to_string(),
            arguments: arguments.clone(),
            result: result.to_string(),
        });
    }

    /// Convenience: before_model_call
    pub fn before_model_call(&self, messages: &[Value]) {
        self.trigger(&HookEvent::BeforeModelCall {
            messages: messages.to_vec(),
        });
    }

    /// Convenience: after_model_call
    pub fn after_model_call(&self, response: &Value) {
        self.trigger(&HookEvent::AfterModelCall {
            response: response.clone(),
        });
    }

    /// Convenience: on_message_receive
    pub fn on_message_receive(&self, source: &str, sender: &str, content: &str) {
        self.trigger(&HookEvent::OnMessageReceive {
            source: source.to_string(),
            sender: sender.to_string(),
            content: content.to_string(),
        });
    }

    /// Convenience: on_message_send
    pub fn on_message_send(&self, target: &str, content: &str) {
        self.trigger(&HookEvent::OnMessageSend {
            target: target.to_string(),
            content: content.to_string(),
        });
    }

    /// Whether any hooks are registered
    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }
}

impl Default for HookManager {
    fn default() -> Self {
        Self::new()
    }
}

// Keep HookRegistry as alias for backward compatibility
pub type HookRegistry = HookManager;
