use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;

const EXPERIENCE_FILE: &str = "data/tool_experiences.json";

/// 工具经验记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolExperience {
    pub tool_name: String,
    pub tips: Vec<String>,
}

/// 工具经验管理器
pub struct ExperienceManager {
    experiences: HashMap<String, Vec<String>>,
}

impl ExperienceManager {
    /// 加载经验（从 JSON 文件 + 内置经验）
    pub fn load() -> Self {
        let mut experiences: HashMap<String, Vec<String>> = HashMap::new();

        // 内置经验
        experiences.insert(
            "exec_command".into(),
            vec![
                "避免在大目录执行 ls -R，可能产生巨量输出".into(),
                "使用 head/tail 限制输出行数".into(),
                "curl 请求注意超时设置，建议加 --max-time 10".into(),
            ],
        );
        experiences.insert(
            "delegate_task".into(),
            vec![
                "任务描述要清晰具体，包含所有必要上下文".into(),
                "子 Agent 无法访问父 Agent 的对话历史".into(),
            ],
        );
        experiences.insert(
            "control_device".into(),
            vec![
                "先查询设备状态再执行控制操作".into(),
                "批量控制时注意设备响应延迟".into(),
            ],
        );
        experiences.insert(
            "analyze_image".into(),
            vec![
                "支持 PNG/JPG/WEBP 格式".into(),
                "图片 URL 必须可公网访问，或使用本地文件路径".into(),
            ],
        );

        // 从文件加载（合并，不覆盖内置）
        if let Ok(content) = fs::read_to_string(EXPERIENCE_FILE) {
            if let Ok(file_exp) = serde_json::from_str::<Vec<ToolExperience>>(&content) {
                for exp in file_exp {
                    let entry = experiences.entry(exp.tool_name).or_default();
                    for tip in exp.tips {
                        if !entry.contains(&tip) {
                            entry.push(tip);
                        }
                    }
                }
            }
        }

        Self { experiences }
    }

    /// 获取指定工具的使用经验
    pub fn get_tips(&self, tool_name: &str) -> Option<&Vec<String>> {
        self.experiences.get(tool_name)
    }

    /// 给工具描述注入经验提示
    pub fn enrich_description(&self, tool_name: &str, original_desc: &str) -> String {
        match self.get_tips(tool_name) {
            Some(tips) if !tips.is_empty() => {
                let tips_text = tips
                    .iter()
                    .map(|t| format!("- {}", t))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("{}\n\n[Usage Tips]\n{}", original_desc, tips_text)
            }
            _ => original_desc.to_string(),
        }
    }
}
