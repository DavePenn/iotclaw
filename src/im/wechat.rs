use reqwest::Client;
use serde_json::json;
use std::env;

/// 微信群机器人 Webhook Bot
pub struct WechatBot {
    webhook_url: String,
    client: Client,
}

impl WechatBot {
    /// 从环境变量创建
    pub fn from_env() -> Self {
        let webhook_url = env::var("WECHAT_WEBHOOK").unwrap_or_default();
        Self {
            webhook_url,
            client: Client::new(),
        }
    }

    /// 发送文本消息到微信群机器人
    /// POST https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=xxx
    /// Body: {"msgtype":"text","text":{"content":"消息内容"}}
    pub async fn send(&self, text: &str) -> Result<(), String> {
        if self.webhook_url.is_empty() {
            return Err("WECHAT_WEBHOOK 未配置".into());
        }

        let body = json!({
            "msgtype": "text",
            "text": {
                "content": text
            }
        });

        let resp = self
            .client
            .post(&self.webhook_url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("微信 Webhook 请求失败: {}", e))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(format!(
                "微信 Webhook 错误 ({}): {}",
                status,
                &text[..text.len().min(200)]
            ));
        }

        Ok(())
    }
}
