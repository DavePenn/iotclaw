use serde_json::{json, Value};
use std::fs;

use super::registry::ToolDef;

const SCREENSHOT_DIR: &str = "data/screenshots";

/// take_screenshot tool definition
pub fn def() -> ToolDef {
    ToolDef {
        name: "take_screenshot".into(),
        description: "Take a screenshot of the desktop or a webpage. Returns the file path of the saved screenshot.".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "URL to capture (optional). If omitted, captures the full desktop."
                },
                "filename": {
                    "type": "string",
                    "description": "Output filename (optional, auto-generated if omitted)"
                }
            }
        }),
        handler: |args: Value| {
            let url = args["url"].as_str().unwrap_or("");
            let filename = args["filename"].as_str().unwrap_or("");

            let _ = fs::create_dir_all(SCREENSHOT_DIR);

            let output_filename = if filename.is_empty() {
                let ts = chrono::Local::now().format("%Y%m%d_%H%M%S");
                if url.is_empty() {
                    format!("desktop_{}.png", ts)
                } else {
                    format!("webpage_{}.png", ts)
                }
            } else {
                filename.to_string()
            };

            let output_path = format!("{}/{}", SCREENSHOT_DIR, output_filename);

            tokio::task::block_in_place(|| {
                let rt = tokio::runtime::Handle::current();
                rt.block_on(async {
                    take_screenshot_impl(url, &output_path).await
                })
            })
        },
    }
}

async fn take_screenshot_impl(url: &str, output_path: &str) -> String {
    use tokio::process::Command;

    if url.is_empty() {
        // Desktop screenshot
        let result = if cfg!(target_os = "macos") {
            Command::new("screencapture")
                .arg("-x") // no sound
                .arg(output_path)
                .output()
                .await
        } else {
            // Linux: try scrot
            Command::new("scrot")
                .arg(output_path)
                .output()
                .await
        };

        match result {
            Ok(output) if output.status.success() => {
                format!("Desktop screenshot saved to: {}", output_path)
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                format!("Screenshot failed: {}", stderr)
            }
            Err(e) => format!("Screenshot command failed: {}", e),
        }
    } else {
        // Webpage screenshot using headless chromium
        let chromium_cmd = find_chromium();
        match chromium_cmd {
            Some(cmd) => {
                let result = Command::new(&cmd)
                    .arg("--headless")
                    .arg("--disable-gpu")
                    .arg("--no-sandbox")
                    .arg(&format!("--screenshot={}", output_path))
                    .arg("--window-size=1280,720")
                    .arg(url)
                    .output()
                    .await;

                match result {
                    Ok(output) if output.status.success() => {
                        format!("Webpage screenshot saved to: {}", output_path)
                    }
                    Ok(output) => {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        format!("Chromium screenshot failed: {}", stderr)
                    }
                    Err(e) => format!("Chromium command failed: {}", e),
                }
            }
            None => "No chromium/chrome found on system. Install Google Chrome or Chromium for webpage screenshots.".into(),
        }
    }
}

/// Find chromium or chrome binary on the system
fn find_chromium() -> Option<String> {
    let candidates = if cfg!(target_os = "macos") {
        vec![
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
            "chromium",
        ]
    } else {
        vec![
            "chromium-browser",
            "chromium",
            "google-chrome",
            "google-chrome-stable",
        ]
    };

    for cmd in candidates {
        if std::path::Path::new(cmd).exists() {
            return Some(cmd.to_string());
        }
        // Check if command is in PATH
        if std::process::Command::new("which")
            .arg(cmd)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Some(cmd.to_string());
        }
    }
    None
}
