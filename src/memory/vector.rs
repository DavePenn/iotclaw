use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::env;
use std::fs;
use std::path::Path;

use crate::tools::registry::ToolDef;

const VECTOR_MEMORY_FILE: &str = "data/vector_memory.json";
const EMBEDDING_MODEL: &str = "text-embedding-v3";

/// 向量记忆条目
#[derive(Serialize, Deserialize, Clone)]
struct VectorEntry {
    text: String,
    vector: Vec<f64>,
}

/// 向量记忆存储（同步版本，使用 reqwest::blocking）
struct VectorMemorySync {
    api_key: String,
    base_url: String,
}

impl VectorMemorySync {
    fn new() -> Self {
        let api_key = env::var("DASHSCOPE_API_KEY").expect("DASHSCOPE_API_KEY not set");
        let base_url = env::var("DASHSCOPE_BASE_URL")
            .unwrap_or_else(|_| "https://dashscope.aliyuncs.com/compatible-mode/v1".into());

        Self { api_key, base_url }
    }

    /// 同步调用 DashScope Embedding API
    fn embed(&self, text: &str) -> Result<Vec<f64>, String> {
        let url = format!("{}/embeddings", self.base_url);
        let body = json!({
            "model": EMBEDDING_MODEL,
            "input": text,
        });

        // 使用 block_in_place 来在 tokio runtime 中执行同步 HTTP
        // 这样不会阻塞 tokio worker thread
        let response = tokio::task::block_in_place(|| {
            let client = reqwest::blocking::Client::new();
            client
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
        })
        .map_err(|e| format!("Embedding HTTP 请求失败: {}", e))?;

        let status = response.status();
        let text_body = response
            .text()
            .map_err(|e| format!("读取 Embedding 响应失败: {}", e))?;

        if !status.is_success() {
            return Err(format!(
                "Embedding API 错误 ({}): {}",
                status,
                &text_body[..text_body.len().min(200)]
            ));
        }

        let parsed: Value =
            serde_json::from_str(&text_body).map_err(|e| format!("JSON 解析失败: {}", e))?;

        let embedding = parsed["data"]
            .get(0)
            .and_then(|d| d["embedding"].as_array())
            .ok_or("Embedding 响应格式错误")?;

        let vector: Vec<f64> = embedding.iter().filter_map(|v| v.as_f64()).collect();

        if vector.is_empty() {
            return Err("Embedding 返回空向量".into());
        }

        Ok(vector)
    }

    /// 添加一条文本到向量记忆
    fn add(&self, text: &str) -> Result<(), String> {
        let vector = self.embed(text)?;

        let mut entries = Self::load_entries();
        entries.push(VectorEntry {
            text: text.to_string(),
            vector,
        });
        Self::save_entries(&entries);

        Ok(())
    }

    /// 语义搜索，返回 top-K 最相似的文本
    fn search(&self, query: &str, top_k: usize) -> Result<Vec<(String, f64)>, String> {
        let query_vec = self.embed(query)?;
        let entries = Self::load_entries();

        if entries.is_empty() {
            return Ok(vec![]);
        }

        let mut scored: Vec<(String, f64)> = entries
            .iter()
            .map(|entry| {
                let sim = cosine_similarity(&query_vec, &entry.vector);
                (entry.text.clone(), sim)
            })
            .collect();

        // 按相似度降序排序
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_k);

        Ok(scored)
    }

    fn load_entries() -> Vec<VectorEntry> {
        if Path::new(VECTOR_MEMORY_FILE).exists() {
            let content = fs::read_to_string(VECTOR_MEMORY_FILE).unwrap_or_default();
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            Vec::new()
        }
    }

    fn save_entries(entries: &[VectorEntry]) {
        let _ = fs::create_dir_all("data");
        let json = serde_json::to_string_pretty(entries).unwrap_or_default();
        let _ = fs::write(VECTOR_MEMORY_FILE, json);
    }
}

/// 余弦相似度: dot(a,b) / (norm(a) * norm(b))
fn cosine_similarity(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
    let norm_b: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    dot / (norm_a * norm_b)
}

/// remember_fact 工具 -- 向量化存储一条信息
pub fn remember_fact_tool() -> ToolDef {
    ToolDef {
        name: "remember_fact".into(),
        description: "将一条重要信息存入向量记忆库，支持后续语义搜索。适合存储事实、偏好、关键信息。".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "要记住的信息文本"
                }
            },
            "required": ["text"]
        }),
        handler: |args: Value| {
            let text = args["text"].as_str().unwrap_or("");
            if text.is_empty() {
                return "错误: text 不能为空".into();
            }
            let text_owned = text.to_string();
            let vm = VectorMemorySync::new();
            match vm.add(&text_owned) {
                Ok(()) => format!("已存入向量记忆: {}", text_owned),
                Err(e) => format!("向量记忆存储失败: {}", e),
            }
        },
    }
}

/// search_memory 工具 -- 语义搜索相关记忆
pub fn search_memory_tool() -> ToolDef {
    ToolDef {
        name: "search_memory".into(),
        description: "在向量记忆库中语义搜索相关信息。返回最相关的记忆条目。".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "搜索查询文本"
                },
                "top_k": {
                    "type": "integer",
                    "description": "返回结果数量，默认 3"
                }
            },
            "required": ["query"]
        }),
        handler: |args: Value| {
            let query = args["query"].as_str().unwrap_or("");
            if query.is_empty() {
                return "错误: query 不能为空".into();
            }
            let top_k = args["top_k"].as_u64().unwrap_or(3) as usize;

            let query_owned = query.to_string();
            let vm = VectorMemorySync::new();
            match vm.search(&query_owned, top_k) {
                Ok(results) => {
                    if results.is_empty() {
                        "向量记忆库为空，没有找到相关记忆".into()
                    } else {
                        let items: Vec<String> = results
                            .iter()
                            .enumerate()
                            .map(|(i, (text, score))| {
                                format!("{}. [相似度 {:.2}] {}", i + 1, score, text)
                            })
                            .collect();
                        format!("搜索结果:\n{}", items.join("\n"))
                    }
                }
                Err(e) => format!("向量记忆搜索失败: {}", e),
            }
        },
    }
}
