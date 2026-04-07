use serde_json::{json, Value};
use std::collections::HashSet;

use super::registry::ToolDef;

/// Exec command whitelist
const DEFAULT_WHITELIST: &[&str] = &["ls", "cat", "echo", "date", "curl", "python3", "pwd", "head", "tail", "wc", "grep", "find", "whoami", "hostname"];

/// Dangerous command patterns
const DANGEROUS_PATTERNS: &[&str] = &[
    "rm -rf",
    "sudo",
    "chmod 777",
    "dd ",
    "mkfs",
    "> /dev/",
    ":(){ :",
    "fork bomb",
    "shutdown",
    "reboot",
    "init 0",
    "init 6",
    "mv / ",
    "rm -r /",
];

/// Check if a command is safe to execute
fn is_safe_command(cmd: &str, whitelist: &HashSet<String>) -> Result<(), String> {
    let cmd_trimmed = cmd.trim();

    // Check dangerous patterns
    let lower = cmd_trimmed.to_lowercase();
    for pattern in DANGEROUS_PATTERNS {
        if lower.contains(pattern) {
            return Err(format!("Blocked: dangerous pattern '{}' detected", pattern));
        }
    }

    // Extract base command (first word, handle pipes by checking each segment)
    let segments: Vec<&str> = cmd_trimmed.split('|').collect();
    for segment in &segments {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        // Also split on && and ;
        for sub in segment.split("&&").flat_map(|s| s.split(';')) {
            let sub = sub.trim();
            if sub.is_empty() {
                continue;
            }
            let base_cmd = sub.split_whitespace().next().unwrap_or("");
            // Strip path prefix (e.g. /usr/bin/ls -> ls)
            let base_name = base_cmd.rsplit('/').next().unwrap_or(base_cmd);
            if !whitelist.contains(base_name) {
                return Err(format!(
                    "Blocked: command '{}' is not in whitelist. Allowed: {:?}",
                    base_name,
                    whitelist.iter().collect::<Vec<_>>()
                ));
            }
        }
    }

    Ok(())
}

/// exec_command tool definition
pub fn def() -> ToolDef {
    ToolDef {
        name: "exec_command".into(),
        description: "Execute a shell command in a sandboxed environment. Only whitelisted commands are allowed (ls, cat, echo, date, curl, python3, etc). Dangerous commands are blocked.".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute"
                },
                "working_dir": {
                    "type": "string",
                    "description": "Working directory for command execution (optional, defaults to current dir)"
                }
            },
            "required": ["command"]
        }),
        handler: |args: Value| {
            let command = args["command"].as_str().unwrap_or("");
            let working_dir = args["working_dir"].as_str().unwrap_or(".");

            if command.is_empty() {
                return "Error: command cannot be empty".into();
            }

            // Build whitelist from config or default
            let whitelist: HashSet<String> = {
                let config_list = crate::config::get_config().security.exec_whitelist.clone();
                if config_list.is_empty() {
                    DEFAULT_WHITELIST.iter().map(|s| s.to_string()).collect()
                } else {
                    config_list.into_iter().collect()
                }
            };

            // Safety check
            if let Err(e) = is_safe_command(command, &whitelist) {
                return e;
            }

            let timeout_secs = crate::config::get_config().security.exec_timeout_secs;

            // Execute with tokio (blocking context since handler is sync fn)
            tokio::task::block_in_place(|| {
                let rt = tokio::runtime::Handle::current();
                rt.block_on(async {
                    execute_command(command, working_dir, timeout_secs).await
                })
            })
        },
    }
}

/// Dangerous environment variables that should be removed before executing commands
const DANGEROUS_ENV_VARS: &[&str] = &[
    "BASH_ENV",
    "ENV",
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH",
    "DYLD_FRAMEWORK_PATH",
    "DYLD_FALLBACK_LIBRARY_PATH",
    "PYTHONPATH",
    "PYTHONSTARTUP",
    "PERL5LIB",
    "RUBYLIB",
    "NODE_OPTIONS",
];

/// Build a sanitized environment variable map (remove dangerous vars, keep the rest)
fn sanitize_env() -> Vec<(String, String)> {
    std::env::vars()
        .filter(|(key, _)| {
            let key_upper = key.to_uppercase();
            // Remove exact matches
            if DANGEROUS_ENV_VARS.contains(&key_upper.as_str()) {
                return false;
            }
            // Remove BASH_FUNC_* patterns
            if key_upper.starts_with("BASH_FUNC_") {
                return false;
            }
            // Remove any DYLD_* on macOS
            if key_upper.starts_with("DYLD_") {
                return false;
            }
            true
        })
        .collect()
}

/// Execute a command with timeout
async fn execute_command(command: &str, working_dir: &str, timeout_secs: u64) -> String {
    use tokio::process::Command;

    // SSRF check: if the command contains curl, validate URLs
    if command.contains("curl") {
        if let Err(e) = crate::security::check_curl_command(command) {
            return e;
        }
    }

    let safe_env = sanitize_env();

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(working_dir)
            .env_clear()
            .envs(safe_env)
            .output(),
    )
    .await;

    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let exit_code = output.status.code().unwrap_or(-1);

            let mut result = format!("[exit_code: {}]\n", exit_code);
            if !stdout.is_empty() {
                // Truncate long output
                let truncated = if stdout.len() > 4000 {
                    format!("{}...\n(truncated, total {} bytes)", &stdout[..4000], stdout.len())
                } else {
                    stdout
                };
                result.push_str(&truncated);
            }
            if !stderr.is_empty() {
                let truncated = if stderr.len() > 1000 {
                    format!("{}...", &stderr[..1000])
                } else {
                    stderr
                };
                result.push_str(&format!("\n[stderr] {}", truncated));
            }
            result
        }
        Ok(Err(e)) => format!("Command execution failed: {}", e),
        Err(_) => format!("Command timed out after {} seconds", timeout_secs),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_whitelist() -> HashSet<String> {
        DEFAULT_WHITELIST.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_exec_safe_command() {
        let wl = default_whitelist();
        assert!(is_safe_command("echo hello world", &wl).is_ok());
        assert!(is_safe_command("ls -la /tmp", &wl).is_ok());
        assert!(is_safe_command("cat /etc/hostname", &wl).is_ok());
    }

    #[test]
    fn test_exec_blocked_command() {
        let wl = default_whitelist();
        let result = is_safe_command("rm -rf /", &wl);
        assert!(result.is_err(), "rm -rf should be blocked");
        assert!(result.unwrap_err().contains("Blocked"));
    }

    #[test]
    fn test_exec_sudo_blocked() {
        let wl = default_whitelist();
        let result = is_safe_command("sudo rm -rf /", &wl);
        assert!(result.is_err(), "sudo should be blocked");
        assert!(result.unwrap_err().contains("Blocked"));
    }
}
