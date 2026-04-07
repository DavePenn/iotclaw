use crate::storage::Database;

/// Identity manager — maps platform-specific user IDs to a unified master ID.
/// Supports multi-platform identity binding (Feishu, WeChat, etc.)
pub struct IdentityManager;

impl IdentityManager {
    /// Bind a platform user to a master ID
    /// If the platform_user_id is already bound, update the master_id
    pub fn bind(platform: &str, platform_user_id: &str, master_id: &str) -> Result<(), String> {
        if platform.is_empty() || platform_user_id.is_empty() || master_id.is_empty() {
            return Err("platform, platform_user_id, and master_id must be non-empty".into());
        }
        Database::global().bind_identity(platform, platform_user_id, master_id)
    }

    /// Resolve a platform-specific user ID to its master ID
    /// Returns None if no binding exists
    pub fn resolve(platform: &str, platform_user_id: &str) -> Option<String> {
        Database::global()
            .resolve_identity(platform, platform_user_id)
            .ok()
            .flatten()
    }

    /// Get all platform bindings for a given master ID
    /// Returns Vec<(platform, platform_user_id)>
    pub fn get_all_bindings(master_id: &str) -> Vec<(String, String)> {
        Database::global()
            .get_all_bindings(master_id)
            .unwrap_or_default()
    }

    /// Process a /bind command from chat
    /// Format: /bind <platform> <platform_user_id> <master_id>
    pub fn handle_bind_command(args: &str) -> String {
        let parts: Vec<&str> = args.split_whitespace().collect();
        if parts.len() < 3 {
            return "Usage: /bind <platform> <platform_user_id> <master_id>\nExample: /bind feishu ou_xxxxx user_001".into();
        }

        let platform = parts[0];
        let platform_user_id = parts[1];
        let master_id = parts[2];

        match Self::bind(platform, platform_user_id, master_id) {
            Ok(()) => format!(
                "Identity bound: {}:{} -> {}",
                platform, platform_user_id, master_id
            ),
            Err(e) => format!("Bind failed: {}", e),
        }
    }

    /// Process a /whoami command — look up the caller's master ID
    pub fn handle_whoami_command(platform: &str, platform_user_id: &str) -> String {
        match Self::resolve(platform, platform_user_id) {
            Some(master_id) => {
                let bindings = Self::get_all_bindings(&master_id);
                let mut lines = vec![format!("Master ID: {}", master_id)];
                lines.push("Bindings:".into());
                for (p, uid) in &bindings {
                    lines.push(format!("  {} : {}", p, uid));
                }
                lines.join("\n")
            }
            None => format!(
                "No identity binding for {}:{}. Use /bind to set one.",
                platform, platform_user_id
            ),
        }
    }
}
