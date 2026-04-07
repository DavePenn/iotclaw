use chrono::Local;
use serde_json::json;
use std::fs::{self, OpenOptions};
use std::io::{Write, BufRead, BufReader};
use uuid::Uuid;

const LOG_DIR: &str = "data/logs";
/// Maximum log file size in bytes (5 MB)
const MAX_LOG_SIZE: u64 = 5 * 1024 * 1024;
/// Maximum number of lines to keep when truncating
const MAX_LOG_LINES: usize = 10_000;

/// NDJSON 结构化日志
pub struct Logger {
    session_id: String,
    file_path: String,
}

impl Logger {
    pub fn new() -> Self {
        let _ = fs::create_dir_all(LOG_DIR);
        let session_id = Uuid::new_v4().to_string();
        let file_path = format!("{}/{}.ndjson", LOG_DIR, session_id);

        Self {
            session_id,
            file_path,
        }
    }

    /// 记录一条日志事件
    pub fn log(&self, event: serde_json::Value) {
        let mut full_event = event;
        if let Some(obj) = full_event.as_object_mut() {
            obj.insert("timestamp".into(), json!(Local::now().format("%Y-%m-%dT%H:%M:%S%.3f%z").to_string()));
            obj.insert("session_id".into(), json!(self.session_id));
        }

        if let Ok(line) = serde_json::to_string(&full_event) {
            // Check if log rolling is needed before writing
            self.maybe_roll_log();

            if let Ok(mut file) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.file_path)
            {
                let _ = writeln!(file, "{}", line);
            }
        }
    }

    /// Roll/truncate the log file if it exceeds size or line limits.
    /// Strategy: keep the newest half of the lines, discard the older half.
    fn maybe_roll_log(&self) {
        let metadata = match fs::metadata(&self.file_path) {
            Ok(m) => m,
            Err(_) => return, // file doesn't exist yet
        };

        if metadata.len() < MAX_LOG_SIZE {
            return;
        }

        // Read all lines
        let file = match fs::File::open(&self.file_path) {
            Ok(f) => f,
            Err(_) => return,
        };
        let reader = BufReader::new(file);
        let lines: Vec<String> = reader.lines().filter_map(|l| l.ok()).collect();

        if lines.len() <= MAX_LOG_LINES / 2 {
            return; // not enough lines to truncate
        }

        // Keep the newest half
        let keep_from = lines.len() / 2;
        let kept_lines = &lines[keep_from..];

        // Rewrite the file with only the kept lines
        if let Ok(mut file) = fs::File::create(&self.file_path) {
            for line in kept_lines {
                let _ = writeln!(file, "{}", line);
            }
        }
    }

    /// 便捷方法：记录消息事件
    pub fn log_message(
        &self,
        role: &str,
        content: &str,
        tool_name: Option<&str>,
        tool_args: Option<&str>,
        tool_result: Option<&str>,
    ) {
        let mut event = json!({
            "role": role,
            "content": content,
        });

        if let Some(name) = tool_name {
            event["tool_name"] = json!(name);
        }
        if let Some(args) = tool_args {
            event["tool_args"] = json!(args);
        }
        if let Some(result) = tool_result {
            event["tool_result"] = json!(result);
        }

        self.log(event);
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }
}

/// Observer trait — 监听 Agent 事件
pub trait Observer: Send + Sync {
    fn on_event(&self, agent_id: &str, event_type: &str, data: &serde_json::Value);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_logger_creates_file() {
        let logger = Logger::new();
        logger.log(json!({"event": "test", "message": "hello"}));

        assert!(Path::new(&logger.file_path).exists(),
            "Log file should be created at: {}", logger.file_path);

        let content = fs::read_to_string(&logger.file_path).unwrap();
        assert!(content.contains("test"), "Log should contain event data");
        assert!(content.contains("session_id"), "Log should contain session_id");

        // Cleanup
        let _ = fs::remove_file(&logger.file_path);
    }
}

/// Logger 实现 Observer，子 Agent 事件记录到日志
impl Observer for Logger {
    fn on_event(&self, agent_id: &str, event_type: &str, data: &serde_json::Value) {
        let mut event = data.clone();
        if let Some(obj) = event.as_object_mut() {
            obj.insert("agent_id".into(), json!(agent_id));
            obj.insert("event_type".into(), json!(event_type));
        } else {
            event = json!({
                "agent_id": agent_id,
                "event_type": event_type,
                "data": data,
            });
        }
        self.log(event);
    }
}
