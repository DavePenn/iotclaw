use serde_json::{json, Value};

use super::registry::ToolDef;

/// read_feishu_doc tool definition
pub fn read_doc_tool() -> ToolDef {
    ToolDef {
        name: "read_feishu_doc".into(),
        description: "Read content from a Feishu document by document_id. Requires FEISHU_APP_ID and FEISHU_APP_SECRET to be configured.".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "document_id": {
                    "type": "string",
                    "description": "The Feishu document ID (from the URL)"
                }
            },
            "required": ["document_id"]
        }),
        handler: |args: Value| {
            let document_id = args["document_id"].as_str().unwrap_or("");
            if document_id.is_empty() {
                return "Error: document_id is required".into();
            }

            tokio::task::block_in_place(|| {
                let rt = tokio::runtime::Handle::current();
                rt.block_on(async {
                    read_feishu_doc_impl(document_id).await
                })
            })
        },
    }
}

/// write_feishu_doc tool definition
pub fn write_doc_tool() -> ToolDef {
    ToolDef {
        name: "write_feishu_doc".into(),
        description: "Append text content to a Feishu document. Requires FEISHU_APP_ID and FEISHU_APP_SECRET to be configured.".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "document_id": {
                    "type": "string",
                    "description": "The Feishu document ID"
                },
                "content": {
                    "type": "string",
                    "description": "Text content to append to the document"
                }
            },
            "required": ["document_id", "content"]
        }),
        handler: |args: Value| {
            let document_id = args["document_id"].as_str().unwrap_or("");
            let content = args["content"].as_str().unwrap_or("");

            if document_id.is_empty() || content.is_empty() {
                return "Error: document_id and content are required".into();
            }

            tokio::task::block_in_place(|| {
                let rt = tokio::runtime::Handle::current();
                rt.block_on(async {
                    write_feishu_doc_impl(document_id, content).await
                })
            })
        },
    }
}

async fn get_tenant_token() -> Result<String, String> {
    let auth = crate::im::feishu_full::FeishuAuth::from_env()
        .ok_or("Feishu API not configured (FEISHU_APP_ID/FEISHU_APP_SECRET missing)")?;
    auth.get_tenant_access_token().await
}

async fn read_feishu_doc_impl(document_id: &str) -> String {
    let token = match get_tenant_token().await {
        Ok(t) => t,
        Err(e) => return format!("Auth error: {}", e),
    };

    let url = format!(
        "https://open.feishu.cn/open-apis/docx/v1/documents/{}/raw_content",
        document_id
    );

    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await;

    match resp {
        Ok(r) => {
            let status = r.status();
            let body: Value = r.json().await.unwrap_or(json!({}));
            if status.is_success() {
                let content = body["data"]["content"].as_str().unwrap_or("(empty)");
                format!("Document content:\n{}", content)
            } else {
                let msg = body["msg"].as_str().unwrap_or("unknown error");
                let code = body["code"].as_i64().unwrap_or(-1);
                format!("Feishu API error (code={}): {}", code, msg)
            }
        }
        Err(e) => format!("HTTP request failed: {}", e),
    }
}

async fn write_feishu_doc_impl(document_id: &str, content: &str) -> String {
    let token = match get_tenant_token().await {
        Ok(t) => t,
        Err(e) => return format!("Auth error: {}", e),
    };

    // First, get the document to find the last block
    let _url = format!(
        "https://open.feishu.cn/open-apis/docx/v1/documents/{}/blocks",
        document_id
    );

    let client = reqwest::Client::new();

    // Create a new text block at the end of the document
    let create_url = format!(
        "https://open.feishu.cn/open-apis/docx/v1/documents/{}/blocks/{}/children",
        document_id, document_id
    );

    let body = json!({
        "children": [{
            "block_type": 2,
            "text": {
                "elements": [{
                    "text_run": {
                        "content": content
                    }
                }],
                "style": {}
            }
        }]
    });

    let resp = client
        .post(&create_url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await;

    match resp {
        Ok(r) => {
            let status = r.status();
            let result: Value = r.json().await.unwrap_or(json!({}));
            if status.is_success() {
                format!("Content appended to document {}", document_id)
            } else {
                let msg = result["msg"].as_str().unwrap_or("unknown error");
                let code = result["code"].as_i64().unwrap_or(-1);
                format!("Feishu API error (code={}): {}", code, msg)
            }
        }
        Err(e) => format!("HTTP request failed: {}", e),
    }
}
