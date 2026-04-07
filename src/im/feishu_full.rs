use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::env;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

// ─── OAuth: Tenant Access Token ───────────────────────────────────────

#[derive(Clone)]
pub struct FeishuAuth {
    app_id: String,
    app_secret: String,
    client: Client,
    token_cache: Arc<RwLock<TokenCache>>,
}

struct TokenCache {
    token: String,
    expires_at: std::time::Instant,
}

impl Default for TokenCache {
    fn default() -> Self {
        Self {
            token: String::new(),
            expires_at: std::time::Instant::now(),
        }
    }
}

#[derive(Deserialize)]
struct TenantTokenResp {
    code: i64,
    msg: String,
    tenant_access_token: Option<String>,
    expire: Option<u64>,
}

impl FeishuAuth {
    pub fn from_env() -> Option<Self> {
        let app_id = env::var("FEISHU_APP_ID").unwrap_or_default();
        let app_secret = env::var("FEISHU_APP_SECRET").unwrap_or_default();
        if app_id.is_empty() || app_secret.is_empty() {
            return None;
        }
        Some(Self {
            app_id,
            app_secret,
            client: Client::new(),
            token_cache: Arc::new(RwLock::new(TokenCache::default())),
        })
    }

    /// Get tenant_access_token, with caching and auto-refresh
    pub async fn get_tenant_access_token(&self) -> Result<String, String> {
        // Check cache first
        {
            let cache = self.token_cache.read().await;
            if !cache.token.is_empty() && std::time::Instant::now() < cache.expires_at {
                return Ok(cache.token.clone());
            }
        }

        // Fetch new token
        let body = json!({
            "app_id": self.app_id,
            "app_secret": self.app_secret,
        });

        let resp = self
            .client
            .post("https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Feishu token request failed: {}", e))?;

        let data: TenantTokenResp = resp
            .json()
            .await
            .map_err(|e| format!("Feishu token parse failed: {}", e))?;

        if data.code != 0 {
            return Err(format!("Feishu token error: code={} msg={}", data.code, data.msg));
        }

        let token = data.tenant_access_token.unwrap_or_default();
        let expire_secs = data.expire.unwrap_or(7200);

        // Cache with 5-minute safety margin
        {
            let mut cache = self.token_cache.write().await;
            cache.token = token.clone();
            cache.expires_at =
                std::time::Instant::now() + std::time::Duration::from_secs(expire_secs.saturating_sub(300));
        }

        Ok(token)
    }
}

// ─── Message Sending (API, not webhook) ───────────────────────────────

#[derive(Clone)]
pub struct FeishuClient {
    pub auth: FeishuAuth,
    client: Client,
}

impl FeishuClient {
    pub fn new(auth: FeishuAuth) -> Self {
        Self {
            auth,
            client: Client::new(),
        }
    }

    pub fn from_env() -> Option<Self> {
        FeishuAuth::from_env().map(|auth| Self::new(auth))
    }

    /// Send message via IM API
    /// msg_type: "text", "interactive", "image"
    /// content: JSON string of the message content
    pub async fn send_message(
        &self,
        receive_id: &str,
        msg_type: &str,
        content: &str,
    ) -> Result<Value, String> {
        let token = self.auth.get_tenant_access_token().await?;

        let body = json!({
            "receive_id": receive_id,
            "msg_type": msg_type,
            "content": content,
        });

        let resp = self
            .client
            .post("https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=chat_id")
            .header("Authorization", format!("Bearer {}", token))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Feishu send_message failed: {}", e))?;

        let result: Value = resp
            .json()
            .await
            .map_err(|e| format!("Feishu send_message parse failed: {}", e))?;

        let code = result["code"].as_i64().unwrap_or(-1);
        if code != 0 {
            return Err(format!(
                "Feishu send_message error: code={} msg={}",
                code,
                result["msg"].as_str().unwrap_or("unknown")
            ));
        }

        Ok(result)
    }

    /// Send text message
    pub async fn send_text(&self, chat_id: &str, text: &str) -> Result<Value, String> {
        let content = json!({"text": text}).to_string();
        self.send_message(chat_id, "text", &content).await
    }

    /// Upload and send an image to a chat
    pub async fn send_image(&self, chat_id: &str, image_path: &str) -> Result<Value, String> {
        let token = self.auth.get_tenant_access_token().await?;

        // Read image file
        let image_data = std::fs::read(image_path)
            .map_err(|e| format!("读取图片失败: {}", e))?;

        let file_name = std::path::Path::new(image_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("image.png")
            .to_string();

        // Upload image
        let form = reqwest::multipart::Form::new()
            .text("image_type", "message")
            .part("image", reqwest::multipart::Part::bytes(image_data)
                .file_name(file_name)
                .mime_str("application/octet-stream")
                .map_err(|e| format!("MIME error: {}", e))?);

        let upload_resp = self
            .client
            .post("https://open.feishu.cn/open-apis/im/v1/images")
            .header("Authorization", format!("Bearer {}", token))
            .multipart(form)
            .send()
            .await
            .map_err(|e| format!("图片上传失败: {}", e))?;

        let upload_result: Value = upload_resp
            .json()
            .await
            .map_err(|e| format!("图片上传响应解析失败: {}", e))?;

        let code = upload_result["code"].as_i64().unwrap_or(-1);
        if code != 0 {
            return Err(format!(
                "图片上传错误: code={} msg={}",
                code,
                upload_result["msg"].as_str().unwrap_or("unknown")
            ));
        }

        let image_key = upload_result["data"]["image_key"]
            .as_str()
            .ok_or("上传响应中无 image_key")?;

        // Send image message
        let content = json!({"image_key": image_key}).to_string();
        self.send_message(chat_id, "image", &content).await
    }

    /// Upload and send a file to a chat
    pub async fn send_file(&self, chat_id: &str, file_path: &str) -> Result<Value, String> {
        let token = self.auth.get_tenant_access_token().await?;

        // Read file
        let file_data = std::fs::read(file_path)
            .map_err(|e| format!("读取文件失败: {}", e))?;

        let file_name = std::path::Path::new(file_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();

        // Determine file type
        let file_type = match file_path.rsplit('.').next().unwrap_or("").to_lowercase().as_str() {
            "pdf" => "pdf",
            "doc" | "docx" => "doc",
            "xls" | "xlsx" => "xls",
            "ppt" | "pptx" => "ppt",
            "mp4" | "avi" | "mov" => "stream",
            _ => "stream",
        };

        // Upload file
        let form = reqwest::multipart::Form::new()
            .text("file_type", file_type.to_string())
            .text("file_name", file_name.clone())
            .part("file", reqwest::multipart::Part::bytes(file_data)
                .file_name(file_name.clone())
                .mime_str("application/octet-stream")
                .map_err(|e| format!("MIME error: {}", e))?);

        let upload_resp = self
            .client
            .post("https://open.feishu.cn/open-apis/im/v1/files")
            .header("Authorization", format!("Bearer {}", token))
            .multipart(form)
            .send()
            .await
            .map_err(|e| format!("文件上传失败: {}", e))?;

        let upload_result: Value = upload_resp
            .json()
            .await
            .map_err(|e| format!("文件上传响应解析失败: {}", e))?;

        let code = upload_result["code"].as_i64().unwrap_or(-1);
        if code != 0 {
            return Err(format!(
                "文件上传错误: code={} msg={}",
                code,
                upload_result["msg"].as_str().unwrap_or("unknown")
            ));
        }

        let file_key = upload_result["data"]["file_key"]
            .as_str()
            .ok_or("上传响应中无 file_key")?;

        // Send file message
        let content = json!({"file_key": file_key}).to_string();
        self.send_message(chat_id, "file", &content).await
    }

    /// Send interactive card message
    pub async fn send_card(&self, chat_id: &str, card_json: &Value) -> Result<Value, String> {
        let content = card_json.to_string();
        self.send_message(chat_id, "interactive", &content).await
    }

    /// Send typing indicator to show "typing..." in the chat
    pub async fn send_typing(&self, chat_id: &str) -> Result<(), String> {
        let token = self.auth.get_tenant_access_token().await?;

        let url = format!(
            "https://open.feishu.cn/open-apis/im/v1/chats/{}/members/typing",
            chat_id
        );

        let resp = self
            .client
            .patch(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&serde_json::json!({}))
            .send()
            .await
            .map_err(|e| format!("Feishu typing indicator failed: {}", e))?;

        let status = resp.status();
        if !status.is_success() {
            // Typing indicator failures are non-critical, just log
            let text = resp.text().await.unwrap_or_default();
            eprintln!("  Feishu typing indicator error ({}): {}", status, &text[..text.len().min(200)]);
        }

        Ok(())
    }

    /// Add a reaction (emoji) to a message
    pub async fn add_reaction(
        &self,
        message_id: &str,
        emoji_type: &str,
    ) -> Result<Value, String> {
        let token = self.auth.get_tenant_access_token().await?;

        let body = json!({
            "reaction_type": {
                "emoji_type": emoji_type
            }
        });

        let url = format!(
            "https://open.feishu.cn/open-apis/im/v1/messages/{}/reactions",
            message_id
        );

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Feishu add_reaction failed: {}", e))?;

        let result: Value = resp
            .json()
            .await
            .map_err(|e| format!("Feishu add_reaction parse failed: {}", e))?;

        let code = result["code"].as_i64().unwrap_or(-1);
        if code != 0 {
            return Err(format!(
                "Feishu add_reaction error: code={} msg={}",
                code,
                result["msg"].as_str().unwrap_or("unknown")
            ));
        }

        Ok(result)
    }

    /// Remove a reaction from a message
    pub async fn remove_reaction(
        &self,
        message_id: &str,
        reaction_id: &str,
    ) -> Result<(), String> {
        let token = self.auth.get_tenant_access_token().await?;

        let url = format!(
            "https://open.feishu.cn/open-apis/im/v1/messages/{}/reactions/{}",
            message_id, reaction_id
        );

        let resp = self
            .client
            .delete(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .map_err(|e| format!("Feishu remove_reaction failed: {}", e))?;

        let result: Value = resp
            .json()
            .await
            .map_err(|e| format!("Feishu remove_reaction parse failed: {}", e))?;

        let code = result["code"].as_i64().unwrap_or(-1);
        if code != 0 {
            return Err(format!(
                "Feishu remove_reaction error: code={} msg={}",
                code,
                result["msg"].as_str().unwrap_or("unknown")
            ));
        }

        Ok(())
    }

    /// Download a resource (image/file) from Feishu by message_id and file_key
    pub async fn download_resource(
        &self,
        message_id: &str,
        file_key: &str,
        resource_type: &str,
    ) -> Result<Vec<u8>, String> {
        let token = self.auth.get_tenant_access_token().await?;

        let url = format!(
            "https://open.feishu.cn/open-apis/im/v1/messages/{}/resources/{}?type={}",
            message_id, file_key, resource_type
        );

        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .map_err(|e| format!("Feishu download_resource failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!(
                "Feishu download_resource error: status={}",
                resp.status()
            ));
        }

        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| format!("Feishu download_resource read failed: {}", e))
    }

    /// Reply to a specific message
    pub async fn reply_message(
        &self,
        message_id: &str,
        msg_type: &str,
        content: &str,
    ) -> Result<Value, String> {
        let token = self.auth.get_tenant_access_token().await?;

        let body = json!({
            "msg_type": msg_type,
            "content": content,
        });

        let url = format!(
            "https://open.feishu.cn/open-apis/im/v1/messages/{}/reply",
            message_id
        );

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Feishu reply failed: {}", e))?;

        let result: Value = resp
            .json()
            .await
            .map_err(|e| format!("Feishu reply parse failed: {}", e))?;

        let code = result["code"].as_i64().unwrap_or(-1);
        if code != 0 {
            return Err(format!(
                "Feishu reply error: code={} msg={}",
                code,
                result["msg"].as_str().unwrap_or("unknown")
            ));
        }

        Ok(result)
    }
}

// ─── Card Builder ─────────────────────────────────────────────────────

pub struct CardBuilder;

impl CardBuilder {
    /// Build an interactive message card
    pub fn build_card(title: &str, content: &str, buttons: &[(&str, &str)]) -> Value {
        let mut button_elements: Vec<Value> = Vec::new();
        for (text, action_value) in buttons {
            button_elements.push(json!({
                "tag": "button",
                "text": {
                    "tag": "plain_text",
                    "content": text
                },
                "type": "primary",
                "value": {
                    "action": action_value
                }
            }));
        }

        let mut elements = vec![json!({
            "tag": "markdown",
            "content": content
        })];

        if !button_elements.is_empty() {
            elements.push(json!({
                "tag": "action",
                "actions": button_elements
            }));
        }

        json!({
            "config": {
                "wide_screen_mode": true
            },
            "header": {
                "title": {
                    "tag": "plain_text",
                    "content": title
                },
                "template": "blue"
            },
            "elements": elements
        })
    }
}

// ─── Event Crypto (AES-CBC decrypt for encrypted events) ──────────────

pub struct FeishuEventCrypto {
    encrypt_key: String,
}

impl FeishuEventCrypto {
    pub fn from_env() -> Option<Self> {
        let key = env::var("FEISHU_ENCRYPT_KEY").unwrap_or_default();
        if key.is_empty() {
            return None;
        }
        Some(Self { encrypt_key: key })
    }

    pub fn new(encrypt_key: &str) -> Self {
        Self {
            encrypt_key: encrypt_key.to_string(),
        }
    }

    /// Decrypt feishu encrypted event body
    /// Feishu uses AES-256-CBC with SHA256(encrypt_key) as key, first 16 bytes of ciphertext as IV
    pub fn decrypt(&self, encrypted_b64: &str) -> Result<String, String> {
        use base64::Engine;
        use sha2::Digest;

        // Derive key: SHA256(encrypt_key)
        let mut hasher = sha2::Sha256::new();
        hasher.update(self.encrypt_key.as_bytes());
        let key_bytes = hasher.finalize();

        // Decode base64
        let encrypted = base64::engine::general_purpose::STANDARD
            .decode(encrypted_b64)
            .map_err(|e| format!("Base64 decode failed: {}", e))?;

        if encrypted.len() < 16 {
            return Err("Encrypted data too short".into());
        }

        // First 16 bytes = IV, rest = ciphertext
        let iv = &encrypted[..16];
        let ciphertext = &encrypted[16..];

        // AES-256-CBC decrypt
        use aes::cipher::{BlockDecryptMut, KeyIvInit};
        type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;

        let mut buf = ciphertext.to_vec();
        let decryptor = Aes256CbcDec::new(key_bytes.as_slice().into(), iv.into());
        let decrypted = decryptor
            .decrypt_padded_mut::<aes::cipher::block_padding::Pkcs7>(&mut buf)
            .map_err(|e| format!("AES decrypt failed: {:?}", e))?;

        String::from_utf8(decrypted.to_vec())
            .map_err(|e| format!("UTF8 decode failed: {}", e))
    }
}

/// Verify feishu event signature
/// signature = SHA256(timestamp + nonce + encrypt_key + body)
pub fn verify_feishu_signature(
    timestamp: &str,
    nonce: &str,
    encrypt_key: &str,
    body: &str,
    signature: &str,
) -> bool {
    use sha2::Digest;
    let content = format!("{}{}{}{}", timestamp, nonce, encrypt_key, body);
    let mut hasher = sha2::Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();
    let computed = hex::encode(result);
    computed == signature
}

// We implement hex::encode inline since we don't want to add a hex crate
mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        bytes
            .as_ref()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect()
    }
}

// ─── Event Processing (enhanced feishu_server.rs helpers) ─────────────

/// Parse a potentially encrypted event body
pub fn parse_event_body(
    body: &Value,
    crypto: Option<&FeishuEventCrypto>,
) -> Result<Value, String> {
    // Check if body is encrypted
    if let Some(encrypt_str) = body.get("encrypt").and_then(|v| v.as_str()) {
        let crypto = crypto.ok_or("Encrypted event but FEISHU_ENCRYPT_KEY not configured")?;
        let decrypted = crypto.decrypt(encrypt_str)?;
        serde_json::from_str(&decrypted).map_err(|e| format!("Parse decrypted event failed: {}", e))
    } else {
        Ok(body.clone())
    }
}

// ─── Merge Forward Message Parsing ───────────────────────────────────

/// A single message extracted from a merge_forward bundle
#[derive(Debug, Clone)]
pub struct ForwardedMessage {
    pub sender: String,
    pub content: String,
    pub msg_type: String,
}

/// Parse a merge_forward message content from Feishu
/// Feishu merge_forward content contains a list of messages in JSON format
pub fn parse_merge_forward(content: &Value) -> Vec<ForwardedMessage> {
    let mut messages = Vec::new();

    // merge_forward content typically has a "messages" array or similar structure
    // Try different known formats
    if let Some(msg_list) = content.get("messages").and_then(|v| v.as_array()) {
        for msg in msg_list {
            let sender = msg.get("sender")
                .or_else(|| msg.get("from"))
                .and_then(|v| {
                    v.as_str().map(|s| s.to_string())
                        .or_else(|| v.get("name").and_then(|n| n.as_str()).map(|s| s.to_string()))
                        .or_else(|| v.get("id").and_then(|n| n.as_str()).map(|s| s.to_string()))
                })
                .unwrap_or_else(|| "unknown".to_string());

            let msg_type = msg.get("msg_type")
                .and_then(|v| v.as_str())
                .unwrap_or("text")
                .to_string();

            let content_val = msg.get("content").unwrap_or(&Value::Null);
            let content_text = if let Some(s) = content_val.as_str() {
                // Try to parse as JSON to extract text field
                if let Ok(parsed) = serde_json::from_str::<Value>(s) {
                    parsed.get("text").and_then(|v| v.as_str()).unwrap_or(s).to_string()
                } else {
                    s.to_string()
                }
            } else if let Some(text) = content_val.get("text").and_then(|v| v.as_str()) {
                text.to_string()
            } else {
                content_val.to_string()
            };

            messages.push(ForwardedMessage {
                sender,
                content: content_text,
                msg_type,
            });
        }
    }

    // Fallback: try to extract from a flat structure
    if messages.is_empty() {
        if let Some(arr) = content.as_array() {
            for item in arr {
                let sender = item.get("sender_name")
                    .or_else(|| item.get("from_name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();

                let text = item.get("text")
                    .or_else(|| item.get("content"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                if !text.is_empty() {
                    messages.push(ForwardedMessage {
                        sender,
                        content: text,
                        msg_type: "text".to_string(),
                    });
                }
            }
        }
    }

    messages
}

// ─── WebSocket Long Connection ────────────────────────────────────────

use futures_util::{SinkExt, StreamExt};

pub struct FeishuWsClient {
    pub auth: FeishuAuth,
    client: Client,
}

#[derive(Deserialize)]
struct WsEndpointResp {
    code: i64,
    msg: String,
    data: Option<WsEndpointData>,
}

#[derive(Deserialize)]
struct WsEndpointData {
    #[serde(rename = "URL")]
    url: Option<String>,
    #[allow(dead_code)]
    client_config: Option<Value>,
}

impl FeishuWsClient {
    pub fn new(auth: FeishuAuth) -> Self {
        Self {
            auth,
            client: Client::new(),
        }
    }

    pub fn from_env() -> Option<Self> {
        FeishuAuth::from_env().map(|auth| Self::new(auth))
    }

    /// Get WebSocket endpoint URL from Feishu API
    async fn get_ws_endpoint(&self) -> Result<String, String> {
        let token = self.auth.get_tenant_access_token().await?;

        let resp = self
            .client
            .post("https://open.feishu.cn/open-apis/callback/ws/endpoint")
            .header("Authorization", format!("Bearer {}", token))
            .json(&json!({}))
            .send()
            .await
            .map_err(|e| format!("Feishu WS endpoint request failed: {}", e))?;

        let data: WsEndpointResp = resp
            .json()
            .await
            .map_err(|e| format!("Feishu WS endpoint parse failed: {}", e))?;

        if data.code != 0 {
            return Err(format!(
                "Feishu WS endpoint error: code={} msg={}",
                data.code, data.msg
            ));
        }

        data.data
            .and_then(|d| d.url)
            .ok_or_else(|| "Feishu WS endpoint URL not found in response".into())
    }

    /// Start WebSocket connection and process events
    /// agent_mutex: shared agent for processing messages
    /// feishu_client: for sending replies
    pub async fn run(
        &self,
        agent_mutex: Arc<Mutex<crate::agent::loop_engine::AgentLoop>>,
        feishu_client: FeishuClient,
    ) -> Result<(), String> {
        loop {
            match self.connect_and_listen(agent_mutex.clone(), feishu_client.clone()).await {
                Ok(()) => {
                    println!("Feishu WS: connection closed, reconnecting in 5s...");
                }
                Err(e) => {
                    eprintln!("Feishu WS: error: {}, reconnecting in 5s...", e);
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    }

    async fn connect_and_listen(
        &self,
        agent_mutex: Arc<Mutex<crate::agent::loop_engine::AgentLoop>>,
        feishu_client: FeishuClient,
    ) -> Result<(), String> {
        let ws_url = self.get_ws_endpoint().await?;
        println!("Feishu WS: connecting to {}", ws_url);

        let (ws_stream, _) = tokio_tungstenite::connect_async(&ws_url)
            .await
            .map_err(|e| format!("WebSocket connect failed: {}", e))?;

        println!("Feishu WS: connected");

        let (mut write, mut read) = ws_stream.split();

        // Heartbeat task
        let heartbeat_handle = tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                let ping = tokio_tungstenite::tungstenite::Message::Ping(vec![].into());
                if write.send(ping).await.is_err() {
                    break;
                }
            }
        });

        // Read messages
        while let Some(msg_result) = read.next().await {
            match msg_result {
                Ok(msg) => {
                    if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
                        let text_str: &str = text.as_ref();
                        if let Ok(event) = serde_json::from_str::<Value>(text_str) {
                            // Process event
                            self.handle_ws_event(
                                &event,
                                agent_mutex.clone(),
                                feishu_client.clone(),
                            )
                            .await;
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Feishu WS: read error: {}", e);
                    break;
                }
            }
        }

        heartbeat_handle.abort();
        Ok(())
    }

    async fn handle_ws_event(
        &self,
        event: &Value,
        agent_mutex: Arc<Mutex<crate::agent::loop_engine::AgentLoop>>,
        feishu_client: FeishuClient,
    ) {
        // WS events have same structure as HTTP callback events
        let event_type = event
            .pointer("/header/event_type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        if event_type != "im.message.receive_v1" {
            return;
        }

        let message_type = event
            .pointer("/event/message/message_type")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let chat_id = event
            .pointer("/event/message/chat_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let message_id = event
            .pointer("/event/message/message_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let sender = event
            .pointer("/event/sender/sender_id/open_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let content_str = event
            .pointer("/event/message/content")
            .and_then(|v| v.as_str())
            .unwrap_or("{}");

        let content_json: Value = serde_json::from_str(content_str).unwrap_or(json!({}));

        // Handle different message types
        let user_text = match message_type {
            "text" => {
                let text = content_json["text"].as_str().unwrap_or("").to_string();
                if text.is_empty() {
                    return;
                }
                text
            }
            "image" => {
                // Image message: download and notify agent for vision analysis
                let image_key = content_json["image_key"].as_str().unwrap_or("");
                if image_key.is_empty() {
                    return;
                }
                println!("Feishu WS: image from={} key={}", sender, image_key);
                let fc = feishu_client.clone();
                let mid = message_id.clone();
                let ik = image_key.to_string();
                // Download image and save
                match fc.download_resource(&mid, &ik, "image").await {
                    Ok(data) => {
                        let _ = std::fs::create_dir_all("data/files");
                        let path = format!("data/files/{}.png", ik);
                        let _ = std::fs::write(&path, &data);
                        format!("[Image received and saved to {}. {} bytes. Please analyze this image.]", path, data.len())
                    }
                    Err(e) => {
                        eprintln!("Feishu WS: image download failed: {}", e);
                        format!("[Image message received but download failed: {}]", e)
                    }
                }
            }
            "file" => {
                // File message: save to data/files/ and notify agent
                let file_key = content_json["file_key"].as_str().unwrap_or("");
                let file_name = content_json["file_name"].as_str().unwrap_or("unknown_file");
                if file_key.is_empty() {
                    return;
                }
                println!("Feishu WS: file from={} name={}", sender, file_name);
                let fc = feishu_client.clone();
                let mid = message_id.clone();
                let fk = file_key.to_string();
                let fname = file_name.to_string();
                match fc.download_resource(&mid, &fk, "file").await {
                    Ok(data) => {
                        let _ = std::fs::create_dir_all("data/files");
                        let path = format!("data/files/{}", fname);
                        let _ = std::fs::write(&path, &data);
                        format!("[File '{}' received and saved to {}. {} bytes.]", fname, path, data.len())
                    }
                    Err(e) => {
                        format!("[File '{}' received but download failed: {}]", fname, e)
                    }
                }
            }
            _ => {
                // Unsupported message type, skip
                return;
            }
        };

        println!("Feishu WS: message from={} type={} text={}", sender, message_type, &user_text[..user_text.len().min(80)]);

        // Process asynchronously
        let agent_mutex = agent_mutex.clone();
        let feishu_client = feishu_client.clone();
        tokio::spawn(async move {
            // Send typing indicator before processing
            if !chat_id.is_empty() {
                let _ = feishu_client.send_typing(&chat_id).await;
            }

            let mut agent = agent_mutex.lock().await;
            match agent.chat(&user_text).await {
                Ok(reply) => {
                    println!(
                        "Feishu WS: reply: {}",
                        &reply[..reply.len().min(100)]
                    );
                    // Add a checkmark reaction to acknowledge the message
                    if !message_id.is_empty() {
                        let _ = feishu_client.add_reaction(&message_id, "DONE").await;
                    }
                    // Try to reply to the message; fall back to sending to chat
                    if !message_id.is_empty() {
                        let content = json!({"text": &reply}).to_string();
                        if let Err(e) = feishu_client.reply_message(&message_id, "text", &content).await {
                            eprintln!("Feishu WS: reply failed: {}, trying send to chat", e);
                            if !chat_id.is_empty() {
                                let _ = feishu_client.send_text(&chat_id, &reply).await;
                            }
                        }
                    } else if !chat_id.is_empty() {
                        if let Err(e) = feishu_client.send_text(&chat_id, &reply).await {
                            eprintln!("Feishu WS: send failed: {}", e);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Feishu WS: agent error: {}", e);
                    if !chat_id.is_empty() {
                        let _ = feishu_client
                            .send_text(&chat_id, &format!("Error: {}", e))
                            .await;
                    }
                }
            }
        });
    }
}
