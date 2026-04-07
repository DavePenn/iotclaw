use serde_json::Value;
use std::collections::HashMap;

/// Sentiment types
#[derive(Debug, Clone, PartialEq)]
pub enum Sentiment {
    Positive,
    Negative,
    Neutral,
    Funny,
    Surprised,
    Thinking,
}

/// Emoji Manager — enhances messages with contextual emoji and manages reactions
pub struct EmojiManager {
    keyword_map: HashMap<&'static str, &'static str>,
}

impl EmojiManager {
    pub fn new() -> Self {
        let mut keyword_map = HashMap::new();
        keyword_map.insert("完成", "✅");
        keyword_map.insert("成功", "✅");
        keyword_map.insert("done", "✅");
        keyword_map.insert("错误", "❌");
        keyword_map.insert("失败", "❌");
        keyword_map.insert("error", "❌");
        keyword_map.insert("fail", "❌");
        keyword_map.insert("思考", "🤔");
        keyword_map.insert("分析", "🤔");
        keyword_map.insert("警告", "⚠️");
        keyword_map.insert("注意", "⚠️");
        keyword_map.insert("warning", "⚠️");
        keyword_map.insert("高兴", "😊");
        keyword_map.insert("开心", "😊");
        keyword_map.insert("好的", "👍");
        keyword_map.insert("收到", "👍");
        keyword_map.insert("设备", "🏠");
        keyword_map.insert("温度", "🌡️");
        keyword_map.insert("灯", "💡");
        keyword_map.insert("天气", "🌤️");
        keyword_map.insert("时间", "🕐");
        keyword_map.insert("记忆", "🧠");
        keyword_map.insert("搜索", "🔍");
        keyword_map.insert("帮助", "🆘");
        keyword_map.insert("你好", "👋");
        keyword_map.insert("hello", "👋");

        Self { keyword_map }
    }

    /// Enhance a message by prepending a relevant emoji based on content keywords
    pub fn enhance_message(&self, text: &str) -> String {
        let lower = text.to_lowercase();

        // Find the first matching keyword
        for (keyword, emoji) in &self.keyword_map {
            if lower.contains(keyword) {
                // Don't add if text already starts with a non-ASCII char (likely emoji)
                let first_char = text.chars().next().unwrap_or(' ');
                if !first_char.is_ascii() && !first_char.is_ascii_alphanumeric() {
                    return text.to_string();
                }
                return format!("{} {}", emoji, text);
            }
        }

        text.to_string()
    }

    /// Get a Feishu reaction emoji type based on sentiment
    pub fn get_reaction(sentiment: Sentiment) -> &'static str {
        match sentiment {
            Sentiment::Positive => "THUMBSUP",
            Sentiment::Negative => "Cry",
            Sentiment::Neutral => "OK",
            Sentiment::Funny => "JIAYI",
            Sentiment::Surprised => "Surprise",
            Sentiment::Thinking => "THINKING",
        }
    }

    /// Detect sentiment from text content
    pub fn detect_sentiment(text: &str) -> Sentiment {
        let lower = text.to_lowercase();
        if lower.contains("谢谢") || lower.contains("感谢") || lower.contains("太好了")
            || lower.contains("棒") || lower.contains("nice") || lower.contains("great")
            || lower.contains("thank")
        {
            Sentiment::Positive
        } else if lower.contains("不好") || lower.contains("糟糕") || lower.contains("失败")
            || lower.contains("错误") || lower.contains("error") || lower.contains("fail")
        {
            Sentiment::Negative
        } else if lower.contains("哈哈") || lower.contains("笑") || lower.contains("lol")
            || lower.contains("funny")
        {
            Sentiment::Funny
        } else if lower.contains("什么") || lower.contains("真的吗") || lower.contains("wow")
            || lower.contains("?") || lower.contains("？")
        {
            Sentiment::Surprised
        } else if lower.contains("想想") || lower.contains("让我") || lower.contains("考虑")
            || lower.contains("think")
        {
            Sentiment::Thinking
        } else {
            Sentiment::Neutral
        }
    }

    /// Add a reaction to a Feishu message (requires FeishuClient token)
    pub async fn add_reaction(
        client: &reqwest::Client,
        token: &str,
        message_id: &str,
        emoji_type: &str,
    ) -> Result<(), String> {
        let url = format!(
            "https://open.feishu.cn/open-apis/im/v1/messages/{}/reactions",
            message_id
        );

        let body = serde_json::json!({
            "reaction_type": {
                "emoji_type": emoji_type
            }
        });

        let resp = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Add reaction failed: {}", e))?;

        let result: Value = resp
            .json()
            .await
            .map_err(|e| format!("Parse reaction response failed: {}", e))?;

        let code = result["code"].as_i64().unwrap_or(-1);
        if code != 0 {
            return Err(format!(
                "Reaction error: code={} msg={}",
                code,
                result["msg"].as_str().unwrap_or("unknown")
            ));
        }

        Ok(())
    }
}

impl Default for EmojiManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enhance_message() {
        let em = EmojiManager::new();
        // Text starting with ASCII gets emoji prepended
        let result = em.enhance_message("Task completed 完成");
        assert!(result.contains("✅"), "Message with '完成' should get ✅: {}", result);

        let result2 = em.enhance_message("OK 操作成功");
        assert!(result2.contains("✅"), "Message with '成功' should get ✅: {}", result2);

        // Text starting with non-ASCII (Chinese) does NOT get emoji added (by design)
        let result3 = em.enhance_message("任务已完成");
        assert!(!result3.contains("✅"), "Chinese-starting text should not get emoji: {}", result3);
    }

    #[test]
    fn test_detect_sentiment() {
        assert_eq!(EmojiManager::detect_sentiment("谢谢你的帮助"), Sentiment::Positive);
        assert_eq!(EmojiManager::detect_sentiment("太好了"), Sentiment::Positive);
        assert_eq!(EmojiManager::detect_sentiment("操作失败了"), Sentiment::Negative);
        assert_eq!(EmojiManager::detect_sentiment("哈哈太搞笑了"), Sentiment::Funny);
        assert_eq!(EmojiManager::detect_sentiment("什么情况？"), Sentiment::Surprised);
        assert_eq!(EmojiManager::detect_sentiment("让我想想"), Sentiment::Thinking);
        assert_eq!(EmojiManager::detect_sentiment("打开灯吧"), Sentiment::Neutral);
    }
}
