use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::env;
use std::sync::Arc;
use tokio::sync::RwLock;

// ─── OAuth: Access Token ──────────────────────────────────────────────

#[derive(Clone)]
pub struct WechatAuth {
    corp_id: String,
    corp_secret: String,
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
struct AccessTokenResp {
    errcode: Option<i64>,
    errmsg: Option<String>,
    access_token: Option<String>,
    expires_in: Option<u64>,
}

impl WechatAuth {
    pub fn from_env() -> Option<Self> {
        let corp_id = env::var("WECHAT_CORP_ID").unwrap_or_default();
        let corp_secret = env::var("WECHAT_CORP_SECRET").unwrap_or_default();
        if corp_id.is_empty() || corp_secret.is_empty() {
            return None;
        }
        Some(Self {
            corp_id,
            corp_secret,
            client: Client::new(),
            token_cache: Arc::new(RwLock::new(TokenCache::default())),
        })
    }

    /// Get access_token with caching and auto-refresh
    pub async fn get_access_token(&self) -> Result<String, String> {
        // Check cache
        {
            let cache = self.token_cache.read().await;
            if !cache.token.is_empty() && std::time::Instant::now() < cache.expires_at {
                return Ok(cache.token.clone());
            }
        }

        let url = format!(
            "https://qyapi.weixin.qq.com/cgi-bin/gettoken?corpid={}&corpsecret={}",
            self.corp_id, self.corp_secret
        );

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Wechat token request failed: {}", e))?;

        let data: AccessTokenResp = resp
            .json()
            .await
            .map_err(|e| format!("Wechat token parse failed: {}", e))?;

        let errcode = data.errcode.unwrap_or(0);
        if errcode != 0 {
            return Err(format!(
                "Wechat token error: errcode={} errmsg={}",
                errcode,
                data.errmsg.unwrap_or_default()
            ));
        }

        let token = data.access_token.unwrap_or_default();
        let expires_in = data.expires_in.unwrap_or(7200);

        // Cache with 5-minute safety margin
        {
            let mut cache = self.token_cache.write().await;
            cache.token = token.clone();
            cache.expires_at =
                std::time::Instant::now() + std::time::Duration::from_secs(expires_in.saturating_sub(300));
        }

        Ok(token)
    }
}

// ─── Message Sending ──────────────────────────────────────────────────

#[derive(Clone)]
pub struct WechatClient {
    pub auth: WechatAuth,
    agent_id: String,
    client: Client,
}

impl WechatClient {
    pub fn new(auth: WechatAuth, agent_id: String) -> Self {
        Self {
            auth,
            agent_id,
            client: Client::new(),
        }
    }

    pub fn from_env() -> Option<Self> {
        let auth = WechatAuth::from_env()?;
        let agent_id = env::var("WECHAT_AGENT_ID").unwrap_or_default();
        if agent_id.is_empty() {
            return None;
        }
        Some(Self::new(auth, agent_id))
    }

    /// Send text message to user
    pub async fn send_text(&self, to_user: &str, content: &str) -> Result<Value, String> {
        let token = self.auth.get_access_token().await?;

        let body = json!({
            "touser": to_user,
            "msgtype": "text",
            "agentid": self.agent_id.parse::<i64>().unwrap_or(0),
            "text": {
                "content": content
            }
        });

        self.post_message(&token, &body).await
    }

    /// Send markdown message to user
    pub async fn send_markdown(&self, to_user: &str, content: &str) -> Result<Value, String> {
        let token = self.auth.get_access_token().await?;

        let body = json!({
            "touser": to_user,
            "msgtype": "markdown",
            "agentid": self.agent_id.parse::<i64>().unwrap_or(0),
            "markdown": {
                "content": content
            }
        });

        self.post_message(&token, &body).await
    }

    /// Send text card message
    pub async fn send_card(
        &self,
        to_user: &str,
        title: &str,
        description: &str,
        url: &str,
        btn_txt: &str,
    ) -> Result<Value, String> {
        let token = self.auth.get_access_token().await?;

        let body = json!({
            "touser": to_user,
            "msgtype": "textcard",
            "agentid": self.agent_id.parse::<i64>().unwrap_or(0),
            "textcard": {
                "title": title,
                "description": description,
                "url": url,
                "btntxt": btn_txt
            }
        });

        self.post_message(&token, &body).await
    }

    async fn post_message(&self, token: &str, body: &Value) -> Result<Value, String> {
        let url = format!(
            "https://qyapi.weixin.qq.com/cgi-bin/message/send?access_token={}",
            token
        );

        let resp = self
            .client
            .post(&url)
            .json(body)
            .send()
            .await
            .map_err(|e| format!("Wechat send failed: {}", e))?;

        let result: Value = resp
            .json()
            .await
            .map_err(|e| format!("Wechat send parse failed: {}", e))?;

        let errcode = result["errcode"].as_i64().unwrap_or(-1);
        if errcode != 0 {
            return Err(format!(
                "Wechat send error: errcode={} errmsg={}",
                errcode,
                result["errmsg"].as_str().unwrap_or("unknown")
            ));
        }

        Ok(result)
    }
}

// ─── Crypto: AES-CBC-256 + SHA1 Signature ─────────────────────────────

pub struct WechatCrypto {
    token: String,
    encoding_aes_key: Vec<u8>,
    corp_id: String,
}

impl WechatCrypto {
    pub fn from_env() -> Option<Self> {
        let token = env::var("WECHAT_TOKEN").unwrap_or_default();
        let aes_key_b64 = env::var("WECHAT_ENCODING_AES_KEY").unwrap_or_default();
        let corp_id = env::var("WECHAT_CORP_ID").unwrap_or_default();

        if token.is_empty() || aes_key_b64.is_empty() || corp_id.is_empty() {
            return None;
        }

        // EncodingAESKey is base64-encoded (43 chars) -> 32 bytes AES key
        use base64::Engine;
        let aes_key = base64::engine::general_purpose::STANDARD
            .decode(format!("{}=", aes_key_b64)) // Add padding
            .ok()?;

        if aes_key.len() < 32 {
            return None;
        }

        Some(Self {
            token,
            encoding_aes_key: aes_key,
            corp_id,
        })
    }

    pub fn new(token: &str, encoding_aes_key_b64: &str, corp_id: &str) -> Result<Self, String> {
        use base64::Engine;
        let aes_key = base64::engine::general_purpose::STANDARD
            .decode(format!("{}=", encoding_aes_key_b64))
            .map_err(|e| format!("Invalid EncodingAESKey: {}", e))?;

        if aes_key.len() < 32 {
            return Err("EncodingAESKey too short".into());
        }

        Ok(Self {
            token: token.to_string(),
            encoding_aes_key: aes_key,
            corp_id: corp_id.to_string(),
        })
    }

    /// Verify WeChat message signature
    /// signature = SHA1(sort(token, timestamp, nonce, encrypt_msg))
    pub fn verify_signature(
        &self,
        signature: &str,
        timestamp: &str,
        nonce: &str,
        encrypt_msg: &str,
    ) -> bool {
        let computed = self.compute_signature(timestamp, nonce, encrypt_msg);
        computed == signature
    }

    fn compute_signature(&self, timestamp: &str, nonce: &str, encrypt_msg: &str) -> String {
        use sha1::Digest;

        let mut parts = vec![
            self.token.clone(),
            timestamp.to_string(),
            nonce.to_string(),
            encrypt_msg.to_string(),
        ];
        parts.sort();

        let concat = parts.join("");
        let mut hasher = sha1::Sha1::new();
        hasher.update(concat.as_bytes());
        let result = hasher.finalize();

        // hex encode
        result.iter().map(|b| format!("{:02x}", b)).collect()
    }

    /// Decrypt WeChat message
    /// AES key = encoding_aes_key[..32], IV = encoding_aes_key[..16]
    /// Plaintext format: 16-byte random + 4-byte msg_len (network order) + msg + corp_id
    pub fn decrypt(&self, encrypted_b64: &str) -> Result<String, String> {
        use aes::cipher::{BlockDecryptMut, KeyIvInit};
        use base64::Engine;

        let encrypted = base64::engine::general_purpose::STANDARD
            .decode(encrypted_b64)
            .map_err(|e| format!("Base64 decode failed: {}", e))?;

        let key = &self.encoding_aes_key[..32];
        let iv = &self.encoding_aes_key[..16];

        type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;

        let mut buf = encrypted.to_vec();
        let decryptor = Aes256CbcDec::new(key.into(), iv.into());
        let decrypted = decryptor
            .decrypt_padded_mut::<aes::cipher::block_padding::Pkcs7>(&mut buf)
            .map_err(|e| format!("AES decrypt failed: {:?}", e))?;

        if decrypted.len() < 20 {
            return Err("Decrypted data too short".into());
        }

        // Skip 16-byte random prefix
        let msg_len_bytes = &decrypted[16..20];
        let msg_len = u32::from_be_bytes([
            msg_len_bytes[0],
            msg_len_bytes[1],
            msg_len_bytes[2],
            msg_len_bytes[3],
        ]) as usize;

        if decrypted.len() < 20 + msg_len {
            return Err("Message length mismatch".into());
        }

        let msg = &decrypted[20..20 + msg_len];
        let trailing = &decrypted[20 + msg_len..];

        // Verify corp_id
        let extracted_corp_id =
            String::from_utf8(trailing.to_vec()).map_err(|e| format!("Corp ID decode: {}", e))?;
        if extracted_corp_id != self.corp_id {
            return Err(format!(
                "Corp ID mismatch: expected={} got={}",
                self.corp_id, extracted_corp_id
            ));
        }

        String::from_utf8(msg.to_vec()).map_err(|e| format!("Message decode: {}", e))
    }

    /// Encrypt a reply message
    /// Plaintext = 16-byte random + 4-byte msg_len (BE) + msg + corp_id
    pub fn encrypt(&self, plaintext: &str) -> Result<String, String> {
        use aes::cipher::{BlockEncryptMut, KeyIvInit};
        use base64::Engine;

        let key = &self.encoding_aes_key[..32];
        let iv = &self.encoding_aes_key[..16];

        // Build plaintext buffer
        let mut buf = Vec::new();

        // 16-byte random
        let random_bytes: [u8; 16] = rand::random();
        buf.extend_from_slice(&random_bytes);

        // 4-byte message length (network order)
        let msg_bytes = plaintext.as_bytes();
        let msg_len = msg_bytes.len() as u32;
        buf.extend_from_slice(&msg_len.to_be_bytes());

        // Message
        buf.extend_from_slice(msg_bytes);

        // Corp ID
        buf.extend_from_slice(self.corp_id.as_bytes());

        // PKCS7 padding
        let block_size = 16;
        let pad_len = block_size - (buf.len() % block_size);
        buf.extend(std::iter::repeat(pad_len as u8).take(pad_len));

        // Encrypt
        type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;

        // We already did PKCS7 padding manually, so use NoPadding mode
        let mut output = vec![0u8; buf.len()];
        let encryptor = Aes256CbcEnc::new(key.into(), iv.into());
        let result = encryptor
            .encrypt_padded_b2b_mut::<aes::cipher::block_padding::NoPadding>(&buf, &mut output)
            .map_err(|e| format!("AES encrypt failed: {:?}", e))?;

        Ok(base64::engine::general_purpose::STANDARD.encode(result))
    }

    /// Build encrypted XML reply
    pub fn build_encrypted_reply(
        &self,
        encrypted: &str,
        timestamp: &str,
        nonce: &str,
    ) -> String {
        let signature = self.compute_signature(timestamp, nonce, encrypted);
        format!(
            "<xml>\n\
             <Encrypt><![CDATA[{}]]></Encrypt>\n\
             <MsgSignature><![CDATA[{}]]></MsgSignature>\n\
             <TimeStamp>{}</TimeStamp>\n\
             <Nonce><![CDATA[{}]]></Nonce>\n\
             </xml>",
            encrypted, signature, timestamp, nonce
        )
    }
}

// ─── XML Message Parsing ──────────────────────────────────────────────

/// Parsed WeChat callback message
#[derive(Debug, Default)]
pub struct WechatMessage {
    pub to_user_name: String,
    pub from_user_name: String,
    pub create_time: String,
    pub msg_type: String,
    pub content: String,
    pub msg_id: String,
    pub agent_id: String,
}

/// Parse the outer encrypted XML envelope
pub fn parse_encrypted_xml(xml_str: &str) -> Result<(String, String, String), String> {
    // Extract: ToUserName, AgentID, Encrypt
    let to_user = extract_xml_field(xml_str, "ToUserName").unwrap_or_default();
    let agent_id = extract_xml_field(xml_str, "AgentID").unwrap_or_default();
    let encrypt = extract_xml_field(xml_str, "Encrypt")
        .ok_or("Missing Encrypt field in XML")?;
    Ok((to_user, agent_id, encrypt))
}

/// Parse decrypted XML message into WechatMessage
pub fn parse_message_xml(xml_str: &str) -> Result<WechatMessage, String> {
    Ok(WechatMessage {
        to_user_name: extract_xml_field(xml_str, "ToUserName").unwrap_or_default(),
        from_user_name: extract_xml_field(xml_str, "FromUserName").unwrap_or_default(),
        create_time: extract_xml_field(xml_str, "CreateTime").unwrap_or_default(),
        msg_type: extract_xml_field(xml_str, "MsgType").unwrap_or_default(),
        content: extract_xml_field(xml_str, "Content").unwrap_or_default(),
        msg_id: extract_xml_field(xml_str, "MsgId").unwrap_or_default(),
        agent_id: extract_xml_field(xml_str, "AgentID").unwrap_or_default(),
    })
}

/// Build a text reply XML (before encryption)
pub fn build_reply_xml(from: &str, to: &str, content: &str) -> String {
    let timestamp = chrono::Utc::now().timestamp();
    format!(
        "<xml>\n\
         <ToUserName><![CDATA[{}]]></ToUserName>\n\
         <FromUserName><![CDATA[{}]]></FromUserName>\n\
         <CreateTime>{}</CreateTime>\n\
         <MsgType><![CDATA[text]]></MsgType>\n\
         <Content><![CDATA[{}]]></Content>\n\
         </xml>",
        to, from, timestamp, content
    )
}

/// Simple XML field extractor using quick_xml
fn extract_xml_field(xml_str: &str, field: &str) -> Option<String> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_str(xml_str);
    let mut in_target = false;
    let mut result = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                if e.name().as_ref() == field.as_bytes() {
                    in_target = true;
                }
            }
            Ok(Event::Text(ref e)) => {
                if in_target {
                    result = e.unescape().ok().map(|s| s.to_string());
                    break;
                }
            }
            Ok(Event::CData(ref e)) => {
                if in_target {
                    result = String::from_utf8(e.to_vec()).ok();
                    break;
                }
            }
            Ok(Event::End(_)) => {
                if in_target {
                    break;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }

    result
}

// ─── QR Code Login Framework ─────────────────────────────────────

/// WeChat QR code login framework (placeholder for actual WeChat Web login protocol)
pub struct WechatQrLogin {
    client: Client,
    uuid: Option<String>,
}

/// QR login status
#[derive(Debug, Clone, PartialEq)]
pub enum QrLoginStatus {
    /// Waiting for user to scan
    WaitingScan,
    /// User scanned, waiting for confirmation
    WaitingConfirm,
    /// Login confirmed
    Confirmed { redirect_url: String },
    /// Login expired or failed
    Expired,
}

impl WechatQrLogin {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            uuid: None,
        }
    }

    /// Get a QR code URL for login
    /// Returns a URL string that should be rendered as a QR code
    pub async fn get_qr_code(&mut self) -> Result<String, String> {
        // Framework: In production, this would call WeChat's login API
        // to get a UUID and generate a QR code URL
        let uuid = uuid::Uuid::new_v4().to_string();
        self.uuid = Some(uuid.clone());

        // The actual WeChat QR login URL format
        let qr_url = format!(
            "https://login.weixin.qq.com/qrcode/{}",
            uuid
        );

        // Print ASCII art QR representation in terminal
        print_ascii_qr(&qr_url);

        Ok(qr_url)
    }

    /// Poll login status
    pub async fn poll_login_status(&self) -> Result<QrLoginStatus, String> {
        let uuid = self.uuid.as_ref().ok_or("No QR code generated yet")?;

        // Framework: In production, this would poll
        // https://login.wx.qq.com/cgi-bin/mmwebwx-bin/login?uuid=...
        // and check the response status code

        // Placeholder: always return WaitingScan
        let _ = uuid;
        Ok(QrLoginStatus::WaitingScan)
    }

    /// Run the full QR login flow in terminal
    pub async fn login_interactive(&mut self) -> Result<String, String> {
        println!("WeChat QR Login");
        println!("===============");

        let qr_url = self.get_qr_code().await?;
        println!("\nScan this QR code with WeChat: {}\n", qr_url);

        // Poll for login status (with timeout)
        let timeout = std::time::Duration::from_secs(120);
        let start = std::time::Instant::now();

        loop {
            if start.elapsed() > timeout {
                return Err("QR login timed out after 120 seconds".into());
            }

            match self.poll_login_status().await? {
                QrLoginStatus::WaitingScan => {
                    // Still waiting
                }
                QrLoginStatus::WaitingConfirm => {
                    println!("Scanned! Please confirm on your phone...");
                }
                QrLoginStatus::Confirmed { redirect_url } => {
                    println!("Login confirmed!");
                    return Ok(redirect_url);
                }
                QrLoginStatus::Expired => {
                    return Err("QR code expired, please try again".into());
                }
            }

            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }
}

impl Default for WechatQrLogin {
    fn default() -> Self {
        Self::new()
    }
}

/// Print a simplified ASCII art QR-like pattern for a URL in the terminal
fn print_ascii_qr(url: &str) {
    // Simple visual representation using block characters
    // In production, use a proper QR code library
    let hash_bytes: Vec<u8> = {
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(url.as_bytes());
        hasher.finalize().to_vec()
    };

    let size = 21; // QR version 1 is 21x21
    println!();
    // Top border
    print!("  ");
    for _ in 0..size + 2 {
        print!("\u{2588}\u{2588}");
    }
    println!();

    for row in 0..size {
        print!("  \u{2588}\u{2588}"); // Left border
        for col in 0..size {
            // Position-detection patterns (corners)
            let in_corner = (row < 7 && col < 7)
                || (row < 7 && col >= size - 7)
                || (row >= size - 7 && col < 7);

            if in_corner {
                // Fixed corner pattern
                let r = if row >= size - 7 { row - (size - 7) } else { row };
                let c = if col >= size - 7 { col - (size - 7) } else { col };
                let is_dark = r == 0 || r == 6 || c == 0 || c == 6
                    || (r >= 2 && r <= 4 && c >= 2 && c <= 4);
                if is_dark {
                    print!("\u{2588}\u{2588}");
                } else {
                    print!("  ");
                }
            } else {
                // Use hash bytes to generate a pseudo-random pattern
                let idx = (row * size + col) % hash_bytes.len();
                if hash_bytes[idx] & (1 << (col % 8)) != 0 {
                    print!("\u{2588}\u{2588}");
                } else {
                    print!("  ");
                }
            }
        }
        println!("\u{2588}\u{2588}"); // Right border
    }

    // Bottom border
    print!("  ");
    for _ in 0..size + 2 {
        print!("\u{2588}\u{2588}");
    }
    println!();
    println!("  (Scan with WeChat)");
}

// ─── Enhanced Message Type Handling ──────────────────────────────

/// Parsed WeChat callback message (extended for image/file support)
#[derive(Debug, Default)]
pub struct WechatMediaMessage {
    pub msg_type: String,
    pub media_id: String,
    pub pic_url: String,
    pub format: String,
}

/// Parse media info from decrypted WeChat XML
pub fn parse_media_xml(xml_str: &str) -> WechatMediaMessage {
    WechatMediaMessage {
        msg_type: extract_xml_field(xml_str, "MsgType").unwrap_or_default(),
        media_id: extract_xml_field(xml_str, "MediaId").unwrap_or_default(),
        pic_url: extract_xml_field(xml_str, "PicUrl").unwrap_or_default(),
        format: extract_xml_field(xml_str, "Format").unwrap_or_default(),
    }
}

// ─── Axum Callback Handler ────────────────────────────────────────────

use axum::extract::Query;
use tokio::sync::Mutex;

#[derive(Deserialize)]
pub struct WechatCallbackQuery {
    pub msg_signature: Option<String>,
    pub timestamp: Option<String>,
    pub nonce: Option<String>,
    pub echostr: Option<String>,
}

/// Shared state for WeChat callback handler
pub struct WechatAppState {
    pub agent: Mutex<crate::agent::loop_engine::AgentLoop>,
    pub crypto: Option<WechatCrypto>,
    pub client: Option<WechatClient>,
}

/// GET /wechat/callback - URL verification
pub async fn wechat_verify(
    axum::extract::State(state): axum::extract::State<Arc<WechatAppState>>,
    Query(params): Query<WechatCallbackQuery>,
) -> (axum::http::StatusCode, String) {
    let crypto = match &state.crypto {
        Some(c) => c,
        None => {
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "WeChat crypto not configured".into(),
            );
        }
    };

    let msg_signature = params.msg_signature.unwrap_or_default();
    let timestamp = params.timestamp.unwrap_or_default();
    let nonce = params.nonce.unwrap_or_default();
    let echostr = params.echostr.unwrap_or_default();

    // Verify signature
    if !crypto.verify_signature(&msg_signature, &timestamp, &nonce, &echostr) {
        return (axum::http::StatusCode::FORBIDDEN, "Invalid signature".into());
    }

    // Decrypt echostr and return plaintext
    match crypto.decrypt(&echostr) {
        Ok(plaintext) => (axum::http::StatusCode::OK, plaintext),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Decrypt failed: {}", e),
        ),
    }
}

/// POST /wechat/callback - Message callback
pub async fn wechat_callback(
    axum::extract::State(state): axum::extract::State<Arc<WechatAppState>>,
    Query(params): Query<WechatCallbackQuery>,
    body: String,
) -> (axum::http::StatusCode, String) {
    let crypto = match &state.crypto {
        Some(c) => c,
        None => {
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "WeChat crypto not configured".into(),
            );
        }
    };

    let msg_signature = params.msg_signature.unwrap_or_default();
    let timestamp = params.timestamp.unwrap_or_default();
    let nonce = params.nonce.unwrap_or_default();

    // Parse outer XML to get Encrypt field
    let (_to_user, _agent_id, encrypt) = match parse_encrypted_xml(&body) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Wechat: XML parse failed: {}", e);
            return (axum::http::StatusCode::BAD_REQUEST, "Invalid XML".into());
        }
    };

    // Verify signature
    if !crypto.verify_signature(&msg_signature, &timestamp, &nonce, &encrypt) {
        return (axum::http::StatusCode::FORBIDDEN, "Invalid signature".into());
    }

    // Decrypt
    let plaintext = match crypto.decrypt(&encrypt) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Wechat: decrypt failed: {}", e);
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Decrypt failed".into(),
            );
        }
    };

    // Parse message
    let msg = match parse_message_xml(&plaintext) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("Wechat: message parse failed: {}", e);
            return (axum::http::StatusCode::BAD_REQUEST, "Parse failed".into());
        }
    };

    println!(
        "Wechat: message from={} type={} content={}",
        msg.from_user_name, msg.msg_type, msg.content
    );

    // Handle different message types
    let user_text = match msg.msg_type.as_str() {
        "text" => {
            if msg.content.is_empty() {
                return (axum::http::StatusCode::OK, "success".into());
            }
            msg.content.clone()
        }
        "image" => {
            // Parse media info for image messages
            let media = parse_media_xml(&plaintext);
            if !media.pic_url.is_empty() {
                // Download and save image
                let _ = std::fs::create_dir_all("data/files");
                let filename = format!("wechat_{}.jpg", chrono::Utc::now().timestamp());
                if let Ok(resp) = reqwest::Client::new().get(&media.pic_url).send().await {
                    if let Ok(bytes) = resp.bytes().await {
                        let path = format!("data/files/{}", filename);
                        let _ = std::fs::write(&path, &bytes);
                        format!("[Image received from WeChat and saved to {}. {} bytes. Please analyze.]", path, bytes.len())
                    } else {
                        "[Image received but download failed]".into()
                    }
                } else {
                    "[Image received but download failed]".into()
                }
            } else {
                return (axum::http::StatusCode::OK, "success".into());
            }
        }
        "voice" => {
            format!("[Voice message received. Media ID: {}]",
                extract_xml_field(&plaintext, "MediaId").unwrap_or_default())
        }
        "file" => {
            let filename = extract_xml_field(&plaintext, "FileName").unwrap_or_else(|| "unknown".into());
            format!("[File '{}' received from WeChat]", filename)
        }
        _ => {
            return (axum::http::StatusCode::OK, "success".into());
        }
    };

    // Process async: call agent and reply via API
    let state_clone = state.clone();
    let from_user = msg.from_user_name.clone();

    tokio::spawn(async move {
        let mut agent = state_clone.agent.lock().await;
        match agent.chat(&user_text).await {
            Ok(reply) => {
                println!(
                    "Wechat: reply to {}: {}",
                    from_user,
                    &reply[..reply.len().min(100)]
                );
                if let Some(ref client) = state_clone.client {
                    if let Err(e) = client.send_text(&from_user, &reply).await {
                        eprintln!("Wechat: send reply failed: {}", e);
                    }
                }
            }
            Err(e) => {
                eprintln!("Wechat: agent error: {}", e);
                if let Some(ref client) = state_clone.client {
                    let _ = client
                        .send_text(&from_user, &format!("Error: {}", e))
                        .await;
                }
            }
        }
    });

    // Return empty "success" response immediately
    (axum::http::StatusCode::OK, "success".into())
}
