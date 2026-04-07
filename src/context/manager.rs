use crate::agent::loop_engine::Message;
use crate::agent::llm_client::LLMClient;

const TOKEN_LIMIT: usize = 6000;
const KEEP_RECENT: usize = 6;

/// 上下文管理器
pub struct ContextManager;

impl ContextManager {
    /// 估算消息列表的 token 数
    pub fn estimate_tokens(messages: &[Message]) -> usize {
        let mut total = 0;
        for msg in messages {
            if let Some(content) = &msg.content {
                total += Self::estimate_str_tokens(content);
            }
            if let Some(calls) = &msg.tool_calls {
                for call in calls {
                    total += Self::estimate_str_tokens(&call.arguments) + 20;
                }
            }
        }
        total
    }

    fn estimate_str_tokens(s: &str) -> usize {
        let mut tokens = 0;
        for c in s.chars() {
            if c.is_ascii() {
                tokens += 1;
            } else {
                tokens += 2;
            }
        }
        (tokens as f64 * 0.7) as usize
    }

    /// 检查是否需要压缩
    pub fn check_overflow(messages: &[Message]) -> Option<(usize, usize)> {
        let total = Self::estimate_tokens(messages);
        if total <= TOKEN_LIMIT {
            return None;
        }

        if messages.len() <= KEEP_RECENT {
            return None;
        }

        let compress_end = messages.len() - KEEP_RECENT;
        Some((0, compress_end))
    }

    /// 用模型生成压缩摘要
    pub async fn compress_with_model(client: &LLMClient, messages: &[Message]) -> Message {
        let mut conversation_text = String::new();
        for msg in messages {
            if let Some(content) = &msg.content {
                let role_label = match msg.role.as_str() {
                    "user" => "用户",
                    "assistant" => "助手",
                    "tool" => "工具结果",
                    _ => &msg.role,
                };
                // 每条消息截取前 200 字避免输入过长
                let truncated = if content.chars().count() > 200 {
                    let end = content.char_indices().nth(200).map(|(i, _)| i).unwrap_or(content.len());
                    format!("{}...", &content[..end])
                } else {
                    content.clone()
                };
                conversation_text.push_str(&format!("[{}] {}\n", role_label, truncated));
            }
        }

        let system = "你是一个对话摘要助手。请用简洁的中文总结以下对话的关键信息，包括：用户的主要问题、助手的回答要点、执行过的操作结果。控制在 200 字以内。";

        match client.simple_chat(system, &conversation_text).await {
            Ok(summary) => Message {
                role: "system".into(),
                content: Some(format!("（以下是之前对话的摘要）\n{}", summary)),
                tool_calls: None,
                tool_call_id: None,
            },
            Err(_) => {
                // 模型压缩失败，回退到简单截断
                Self::compress_messages_fallback(messages)
            }
        }
    }

    #[cfg(test)]
    pub fn estimate_str_tokens_pub(s: &str) -> usize {
        Self::estimate_str_tokens(s)
    }

    /// 回退压缩方案（简单截断）
    fn compress_messages_fallback(messages: &[Message]) -> Message {
        let mut summary_parts = Vec::new();
        for msg in messages {
            if let Some(content) = &msg.content {
                let role_label = match msg.role.as_str() {
                    "user" => "用户",
                    "assistant" => "助手",
                    "tool" => "工具结果",
                    _ => &msg.role,
                };
                let truncated = if content.chars().count() > 100 {
                    let end = content.char_indices().nth(100).map(|(i, _)| i).unwrap_or(content.len());
                    format!("{}...", &content[..end])
                } else {
                    content.clone()
                };
                summary_parts.push(format!("[{}] {}", role_label, truncated));
            }
        }

        Message {
            role: "system".into(),
            content: Some(format!("（以下是之前对话的摘要）\n{}", summary_parts.join("\n"))),
            tool_calls: None,
            tool_call_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_msg(role: &str, content: &str) -> Message {
        Message {
            role: role.into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    #[test]
    fn test_estimate_tokens() {
        let english = ContextManager::estimate_str_tokens_pub("hello world");
        let chinese = ContextManager::estimate_str_tokens_pub("你好世界");
        // Chinese characters count as 2 each, so "你好世界" (4 chars * 2 = 8) * 0.7 = 5
        // "hello world" (11 chars * 1 = 11) * 0.7 = 7
        assert!(chinese > 0);
        assert!(english > 0);
        // 4 Chinese chars should produce fewer raw chars but more tokens per char
        // "你好世界" = 4*2*0.7 = 5.6 -> 5
        // "hello world" = 11*1*0.7 = 7.7 -> 7
        // So english > chinese here because english string is longer
        // Let's compare equal-length strings
        let en = ContextManager::estimate_str_tokens_pub("abcd");  // 4*1*0.7 = 2
        let cn = ContextManager::estimate_str_tokens_pub("你好世界"); // 4*2*0.7 = 5
        assert!(cn > en, "Chinese should estimate more tokens than same-length ASCII: cn={} en={}", cn, en);
    }

    #[test]
    fn test_no_overflow() {
        let msgs: Vec<Message> = (0..3)
            .map(|i| make_msg("user", &format!("short message {}", i)))
            .collect();
        assert!(ContextManager::check_overflow(&msgs).is_none(), "Few short messages should not overflow");
    }

    #[test]
    fn test_overflow_detected() {
        // Create many long messages to exceed TOKEN_LIMIT (6000)
        let long_text = "这是一段很长的中文文本用来测试上下文溢出。".repeat(100);
        let msgs: Vec<Message> = (0..20)
            .map(|_| make_msg("user", &long_text))
            .collect();
        let result = ContextManager::check_overflow(&msgs);
        assert!(result.is_some(), "Many long messages should trigger overflow");
        let (start, end) = result.unwrap();
        assert_eq!(start, 0);
        assert!(end > 0);
    }
}
