use reqwest::Client;
use serde_json::json;
use std::env;

/// 飞书 Webhook Bot
pub struct FeishuBot {
    webhook_url: String,
    client: Client,
}

impl FeishuBot {
    /// 从环境变量创建
    pub fn from_env() -> Self {
        let webhook_url = env::var("FEISHU_WEBHOOK")
            .unwrap_or_else(|_| String::new());
        Self {
            webhook_url,
            client: Client::new(),
        }
    }

    /// 发送文本消息到飞书
    pub async fn send(&self, text: &str) -> Result<(), String> {
        if self.webhook_url.is_empty() {
            return Err("FEISHU_WEBHOOK 未配置".into());
        }

        let body = json!({
            "msg_type": "text",
            "content": {
                "text": text
            }
        });

        let resp = self.client
            .post(&self.webhook_url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("飞书 Webhook 请求失败: {}", e))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("飞书 Webhook 错误 ({}): {}", status, &text[..text.len().min(200)]));
        }

        Ok(())
    }
}
