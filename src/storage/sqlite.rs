use rusqlite::{params, Connection};
use std::sync::Mutex;

const DB_PATH: &str = "data/iotclaw.db";

/// SQLite persistent storage
pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    /// Open or create the database, run migrations
    pub fn open() -> Result<Self, String> {
        let _ = std::fs::create_dir_all("data");
        let conn = Connection::open(DB_PATH)
            .map_err(|e| format!("SQLite open failed: {}", e))?;

        // Enable WAL mode for better concurrent reads
        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .map_err(|e| format!("SQLite WAL pragma failed: {}", e))?;

        // Create tables
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS group_members (
                group_id TEXT NOT NULL,
                user_id TEXT NOT NULL,
                name TEXT NOT NULL DEFAULT '',
                updated_at INTEGER NOT NULL,
                PRIMARY KEY(group_id, user_id)
            );
            CREATE TABLE IF NOT EXISTS sessions (
                session_id TEXT PRIMARY KEY,
                messages TEXT NOT NULL DEFAULT '[]',
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS memories (
                scope TEXT NOT NULL,
                key TEXT NOT NULL,
                value TEXT NOT NULL DEFAULT '',
                updated_at INTEGER NOT NULL,
                PRIMARY KEY(scope, key)
            );
            CREATE TABLE IF NOT EXISTS group_info (
                group_id TEXT PRIMARY KEY,
                name TEXT NOT NULL DEFAULT '',
                member_count INTEGER NOT NULL DEFAULT 0,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS identity_bindings (
                platform TEXT NOT NULL,
                platform_user_id TEXT NOT NULL,
                master_id TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                PRIMARY KEY(platform, platform_user_id)
            );",
        )
        .map_err(|e| format!("SQLite create tables failed: {}", e))?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Get a global singleton instance
    pub fn global() -> &'static Database {
        use std::sync::OnceLock;
        static INSTANCE: OnceLock<Database> = OnceLock::new();
        INSTANCE.get_or_init(|| {
            Database::open().expect("Failed to open SQLite database")
        })
    }

    // ─── Session Operations ──────────────────────────────────────

    /// Save session messages (upsert)
    pub fn save_session(&self, session_id: &str, messages_json: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("lock: {}", e))?;
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO sessions (session_id, messages, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3)
             ON CONFLICT(session_id) DO UPDATE SET messages=?2, updated_at=?3",
            params![session_id, messages_json, now],
        )
        .map_err(|e| format!("save_session: {}", e))?;
        Ok(())
    }

    /// Load session messages by id
    pub fn load_session(&self, session_id: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().map_err(|e| format!("lock: {}", e))?;
        let mut stmt = conn
            .prepare("SELECT messages FROM sessions WHERE session_id = ?1")
            .map_err(|e| format!("prepare: {}", e))?;
        let mut rows = stmt
            .query_map(params![session_id], |row| row.get::<_, String>(0))
            .map_err(|e| format!("query: {}", e))?;
        match rows.next() {
            Some(Ok(messages)) => Ok(Some(messages)),
            _ => Ok(None),
        }
    }

    // ─── Group Member Operations ─────────────────────────────────

    /// Save a group member (upsert)
    pub fn save_member(&self, group_id: &str, user_id: &str, name: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("lock: {}", e))?;
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO group_members (group_id, user_id, name, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(group_id, user_id) DO UPDATE SET name=?3, updated_at=?4",
            params![group_id, user_id, name, now],
        )
        .map_err(|e| format!("save_member: {}", e))?;
        Ok(())
    }

    /// Get all members of a group
    pub fn get_members(&self, group_id: &str) -> Result<Vec<(String, String)>, String> {
        let conn = self.conn.lock().map_err(|e| format!("lock: {}", e))?;
        let mut stmt = conn
            .prepare("SELECT user_id, name FROM group_members WHERE group_id = ?1")
            .map_err(|e| format!("prepare: {}", e))?;
        let rows = stmt
            .query_map(params![group_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("query: {}", e))?;
        let mut members = Vec::new();
        for row in rows {
            if let Ok(m) = row {
                members.push(m);
            }
        }
        Ok(members)
    }

    // ─── Group Info Operations ───────────────────────────────────

    /// Save group info (upsert)
    pub fn save_group_info(
        &self,
        group_id: &str,
        name: &str,
        member_count: i64,
    ) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("lock: {}", e))?;
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO group_info (group_id, name, member_count, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(group_id) DO UPDATE SET name=?2, member_count=?3, updated_at=?4",
            params![group_id, name, member_count, now],
        )
        .map_err(|e| format!("save_group_info: {}", e))?;
        Ok(())
    }

    /// Load group info, returns None if not found or expired (older than ttl_secs)
    pub fn load_group_info(
        &self,
        group_id: &str,
        ttl_secs: i64,
    ) -> Result<Option<(String, i64)>, String> {
        let conn = self.conn.lock().map_err(|e| format!("lock: {}", e))?;
        let cutoff = chrono::Utc::now().timestamp() - ttl_secs;
        let mut stmt = conn
            .prepare(
                "SELECT name, member_count FROM group_info WHERE group_id = ?1 AND updated_at > ?2",
            )
            .map_err(|e| format!("prepare: {}", e))?;
        let mut rows = stmt
            .query_map(params![group_id, cutoff], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|e| format!("query: {}", e))?;
        match rows.next() {
            Some(Ok(info)) => Ok(Some(info)),
            _ => Ok(None),
        }
    }

    // ─── Memory Operations ───────────────────────────────────────

    /// Save a scoped memory key-value
    pub fn save_memory(&self, scope: &str, key: &str, value: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("lock: {}", e))?;
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO memories (scope, key, value, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(scope, key) DO UPDATE SET value=?3, updated_at=?4",
            params![scope, key, value, now],
        )
        .map_err(|e| format!("save_memory: {}", e))?;
        Ok(())
    }

    /// Load a scoped memory value
    pub fn load_memory(&self, scope: &str, key: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().map_err(|e| format!("lock: {}", e))?;
        let mut stmt = conn
            .prepare("SELECT value FROM memories WHERE scope = ?1 AND key = ?2")
            .map_err(|e| format!("prepare: {}", e))?;
        let mut rows = stmt
            .query_map(params![scope, key], |row| row.get::<_, String>(0))
            .map_err(|e| format!("query: {}", e))?;
        match rows.next() {
            Some(Ok(v)) => Ok(Some(v)),
            _ => Ok(None),
        }
    }

    // ─── Identity Binding Operations ─────────────────────────────

    /// Bind a platform user to a master ID
    pub fn bind_identity(
        &self,
        platform: &str,
        platform_user_id: &str,
        master_id: &str,
    ) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("lock: {}", e))?;
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO identity_bindings (platform, platform_user_id, master_id, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(platform, platform_user_id) DO UPDATE SET master_id=?3",
            params![platform, platform_user_id, master_id, now],
        )
        .map_err(|e| format!("bind_identity: {}", e))?;
        Ok(())
    }

    /// Resolve a platform user to master ID
    pub fn resolve_identity(
        &self,
        platform: &str,
        platform_user_id: &str,
    ) -> Result<Option<String>, String> {
        let conn = self.conn.lock().map_err(|e| format!("lock: {}", e))?;
        let mut stmt = conn
            .prepare(
                "SELECT master_id FROM identity_bindings WHERE platform = ?1 AND platform_user_id = ?2",
            )
            .map_err(|e| format!("prepare: {}", e))?;
        let mut rows = stmt
            .query_map(params![platform, platform_user_id], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| format!("query: {}", e))?;
        match rows.next() {
            Some(Ok(id)) => Ok(Some(id)),
            _ => Ok(None),
        }
    }

    /// Get all identity bindings for a master ID
    pub fn get_all_bindings(
        &self,
        master_id: &str,
    ) -> Result<Vec<(String, String)>, String> {
        let conn = self.conn.lock().map_err(|e| format!("lock: {}", e))?;
        let mut stmt = conn
            .prepare(
                "SELECT platform, platform_user_id FROM identity_bindings WHERE master_id = ?1",
            )
            .map_err(|e| format!("prepare: {}", e))?;
        let rows = stmt
            .query_map(params![master_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("query: {}", e))?;
        let mut bindings = Vec::new();
        for row in rows {
            if let Ok(b) = row {
                bindings.push(b);
            }
        }
        Ok(bindings)
    }
}
