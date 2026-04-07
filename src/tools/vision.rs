use serde_json::{json, Value};
use std::env;

use super::registry::ToolDef;

/// analyze_image 工具定义 — 调用 DashScope 多模态 API
pub fn def() -> ToolDef {
    ToolDef {
        name: "analyze_image".into(),
        description: "Analyze an image using vision AI model. Accepts a local file path or a public URL. Returns a text description of the image content.".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "image": {
                    "type": "string",
                    "description": "Image file path (local) or URL (public). Supported formats: PNG, JPG, WEBP."
                },
                "prompt": {
                    "type": "string",
                    "description": "What to analyze in the image (optional, defaults to general description)"
                }
            },
            "required": ["image"]
        }),
        handler: |args: Value| {
            let image = args["image"].as_str().unwrap_or("");
            let prompt = args["prompt"].as_str().unwrap_or("请详细描述这张图片的内容。");

            if image.is_empty() {
                return "Error: image path or URL is required".into();
            }

            tokio::task::block_in_place(|| {
                let rt = tokio::runtime::Handle::current();
                rt.block_on(async { analyze_image_impl(image, prompt).await })
            })
        },
    }
}

async fn analyze_image_impl(image: &str, prompt: &str) -> String {
    let api_key = match env::var("DASHSCOPE_API_KEY") {
        Ok(k) => k,
        Err(_) => return "Error: DASHSCOPE_API_KEY not set".into(),
    };

    let base_url = env::var("DASHSCOPE_BASE_URL")
        .unwrap_or_else(|_| "https://dashscope.aliyuncs.com/compatible-mode/v1".into());
    let model = env::var("VISION_MODEL").unwrap_or_else(|_| "qwen-vl-plus".into());

    // 构建图片 URL
    let image_url = if image.starts_with("http://") || image.starts_with("https://") {
        image.to_string()
    } else {
        // 本地文件：读取并转为 base64 data URI
        match std::fs::read(image) {
            Ok(data) => {
                use base64::Engine;
                let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
                let ext = image.rsplit('.').next().unwrap_or("png").to_lowercase();
                let mime = match ext.as_str() {
                    "jpg" | "jpeg" => "image/jpeg",
                    "webp" => "image/webp",
                    "gif" => "image/gif",
                    _ => "image/png",
                };
                format!("data:{};base64,{}", mime, b64)
            }
            Err(e) => return format!("Error reading image file: {}", e),
        }
    };

    let body = json!({
        "model": model,
        "messages": [{
            "role": "user",
            "content": [
                {
                    "type": "image_url",
                    "image_url": {
                        "url": image_url
                    }
                },
                {
                    "type": "text",
                    "text": prompt
                }
            ]
        }]
    });

    let client = reqwest::Client::new();
    let url = format!("{}/chat/completions", base_url);

    let resp = match client
        .post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return format!("Vision API request failed: {}", e),
    };

    let status = resp.status();
    let text = match resp.text().await {
        Ok(t) => t,
        Err(e) => return format!("Vision API read response failed: {}", e),
    };

    if !status.is_success() {
        return format!("Vision API error ({}): {}", status, &text[..text.len().min(300)]);
    }

    let result: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => return format!("Vision API JSON parse failed: {}", e),
    };

    result["choices"]
        .get(0)
        .and_then(|c| c["message"]["content"].as_str())
        .unwrap_or("(no response from vision model)")
        .to_string()
}
