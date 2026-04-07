use crate::agent::llm_client::LLMClient;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Group chat message context
pub struct GroupContext {
    pub bot_name: String,
    pub recent_messages: Vec<String>,
}

/// Cached group member info
#[derive(Debug, Clone)]
pub struct GroupMember {
    pub user_id: String,
    pub name: String,
}

/// Cached group info (name + member_count)
#[derive(Debug, Clone)]
pub struct GroupInfo {
    pub name: String,
    pub member_count: i64,
    pub updated_at: std::time::Instant,
}

/// Three-level group info cache: Memory HashMap -> SQLite -> API call
pub struct GroupInfoCache {
    /// In-memory cache: group_id -> GroupInfo
    memory_cache: Arc<RwLock<HashMap<String, GroupInfo>>>,
    /// Memory cache TTL (5 minutes)
    memory_ttl: std::time::Duration,
    /// SQLite TTL (1 hour)
    sqlite_ttl: i64,
}

impl GroupInfoCache {
    pub fn new() -> Self {
        Self {
            memory_cache: Arc::new(RwLock::new(HashMap::new())),
            memory_ttl: std::time::Duration::from_secs(300),  // 5 minutes
            sqlite_ttl: 3600,                                   // 1 hour
        }
    }

    /// Get group info with three-level cache: Memory -> SQLite -> API
    pub async fn get_group_info(&self, group_id: &str) -> Option<GroupInfo> {
        // Level 1: Memory cache
        {
            let cache = self.memory_cache.read().await;
            if let Some(info) = cache.get(group_id) {
                if info.updated_at.elapsed() < self.memory_ttl {
                    return Some(info.clone());
                }
            }
        }

        // Level 2: SQLite cache
        let db = crate::storage::Database::global();
        if let Ok(Some((name, member_count))) = db.load_group_info(group_id, self.sqlite_ttl) {
            let info = GroupInfo {
                name: name.clone(),
                member_count,
                updated_at: std::time::Instant::now(),
            };
            // Backfill memory cache
            let mut cache = self.memory_cache.write().await;
            cache.insert(group_id.to_string(), info.clone());
            return Some(info);
        }

        // Level 3: API call (Feishu)
        if let Some(info) = fetch_group_info_from_feishu(group_id).await {
            // Backfill both caches
            let _ = db.save_group_info(group_id, &info.name, info.member_count);
            let mut cache = self.memory_cache.write().await;
            cache.insert(group_id.to_string(), info.clone());
            return Some(info);
        }

        None
    }

    /// Invalidate cache for a group
    pub async fn invalidate(&self, group_id: &str) {
        let mut cache = self.memory_cache.write().await;
        cache.remove(group_id);
    }
}

impl Default for GroupInfoCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Fetch group info from Feishu API
async fn fetch_group_info_from_feishu(chat_id: &str) -> Option<GroupInfo> {
    let auth = crate::im::feishu_full::FeishuAuth::from_env()?;
    let token = auth.get_tenant_access_token().await.ok()?;

    let url = format!(
        "https://open.feishu.cn/open-apis/im/v1/chats/{}",
        chat_id
    );

    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .ok()?;

    let body: serde_json::Value = resp.json().await.ok()?;
    let data = body.get("data")?;
    let name = data.get("name")?.as_str()?.to_string();
    let member_count = data
        .get("user_count")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    Some(GroupInfo {
        name,
        member_count,
        updated_at: std::time::Instant::now(),
    })
}

/// Group member cache with TTL
pub struct GroupMemberCache {
    /// chat_id -> (members, fetched_at)
    cache: Arc<RwLock<HashMap<String, (Vec<GroupMember>, std::time::Instant)>>>,
    ttl: std::time::Duration,
}

impl GroupMemberCache {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            ttl: std::time::Duration::from_secs(300), // 5 minutes TTL
        }
    }

    pub fn with_ttl(ttl_secs: u64) -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            ttl: std::time::Duration::from_secs(ttl_secs),
        }
    }

    /// Get cached members for a chat, returns None if expired or missing
    pub async fn get(&self, chat_id: &str) -> Option<Vec<GroupMember>> {
        let cache = self.cache.read().await;
        if let Some((members, fetched_at)) = cache.get(chat_id) {
            if fetched_at.elapsed() < self.ttl {
                return Some(members.clone());
            }
        }
        None
    }

    /// Store members in cache
    pub async fn set(&self, chat_id: &str, members: Vec<GroupMember>) {
        let mut cache = self.cache.write().await;
        cache.insert(chat_id.to_string(), (members, std::time::Instant::now()));
    }

    /// Invalidate cache for a chat
    pub async fn invalidate(&self, chat_id: &str) {
        let mut cache = self.cache.write().await;
        cache.remove(chat_id);
    }

    /// Check if a user_id is the bot itself in the given chat
    pub async fn is_bot_mentioned(&self, chat_id: &str, bot_user_id: &str, message: &str) -> bool {
        // Check if @bot_user_id appears in message
        if message.contains(&format!("@{}", bot_user_id)) {
            return true;
        }

        // Check by name lookup
        if let Some(members) = self.get(chat_id).await {
            for member in &members {
                if member.user_id == bot_user_id {
                    if message.contains(&format!("@{}", member.name)) {
                        return true;
                    }
                }
            }
        }

        false
    }

    /// Get or fetch members for a chat (using Feishu API)
    pub async fn get_or_fetch(&self, chat_id: &str) -> Vec<GroupMember> {
        // Return cached if valid
        if let Some(members) = self.get(chat_id).await {
            return members;
        }

        // Fetch from Feishu API
        let members = fetch_group_members_from_feishu(chat_id).await;
        self.set(chat_id, members.clone()).await;
        members
    }
}

impl Default for GroupMemberCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Fetch group members from Feishu API
async fn fetch_group_members_from_feishu(chat_id: &str) -> Vec<GroupMember> {
    let auth = match crate::im::feishu_full::FeishuAuth::from_env() {
        Some(a) => a,
        None => return Vec::new(),
    };

    let token = match auth.get_tenant_access_token().await {
        Ok(t) => t,
        Err(e) => {
            eprintln!("  GroupMemberCache: token error: {}", e);
            return Vec::new();
        }
    };

    let url = format!(
        "https://open.feishu.cn/open-apis/im/v1/chats/{}/members?member_id_type=open_id",
        chat_id
    );

    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await;

    match resp {
        Ok(r) => {
            let body: serde_json::Value = r.json().await.unwrap_or(serde_json::json!({}));
            let items = body.pointer("/data/items").and_then(|v| v.as_array());
            match items {
                Some(arr) => {
                    arr.iter().filter_map(|item| {
                        let user_id = item.get("member_id").and_then(|v| v.as_str())?.to_string();
                        let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        Some(GroupMember { user_id, name })
                    }).collect()
                }
                None => Vec::new(),
            }
        }
        Err(e) => {
            eprintln!("  GroupMemberCache: fetch error: {}", e);
            Vec::new()
        }
    }
}

/// Smart group chat reply filter
pub struct SmartFilter {
    llm_client: LLMClient,
    debounce_ms: u64,
    member_cache: GroupMemberCache,
}

impl SmartFilter {
    pub fn new(llm_client: LLMClient) -> Self {
        Self {
            llm_client,
            debounce_ms: 2000,
            member_cache: GroupMemberCache::new(),
        }
    }

    /// Get the member cache reference
    pub fn member_cache(&self) -> &GroupMemberCache {
        &self.member_cache
    }

    /// Determine if the bot should respond to a group message
    pub async fn should_respond(&self, message: &str, context: &GroupContext) -> bool {
        // Rule 1: @mention of bot name
        if message.contains(&format!("@{}", context.bot_name))
            || message.contains(&context.bot_name)
        {
            return true;
        }

        // Rule 2: Debounce
        tokio::time::sleep(tokio::time::Duration::from_millis(self.debounce_ms)).await;

        // Rule 3: LLM-based judgment
        let recent = context.recent_messages.join("\n");
        let prompt = format!(
            "You are a group chat assistant named {}.\n\
            Recent messages:\n{}\n\n\
            Latest message: {}\n\n\
            Should you reply? Answer YES or NO.\n\
            Criteria:\n\
            - YES if the message asks you a question or needs your help\n\
            - NO if it's casual chat unrelated to you\n\
            - YES if it's about smart home/IoT/device control",
            context.bot_name, recent, message
        );

        match self.llm_client.simple_chat(
            "You are a judgment assistant. Only answer YES or NO.",
            &prompt,
        ).await {
            Ok(response) => {
                let answer = response.trim().to_uppercase();
                answer.contains("YES")
            }
            Err(_) => false,
        }
    }
}
