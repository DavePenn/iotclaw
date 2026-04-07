use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Skill 定义
#[derive(Debug, Clone)]
pub struct SkillDef {
    pub name: String,
    pub description: String,
    pub tools: Vec<String>,           // 允许使用的工具名列表
    pub system_prompt: String,        // Markdown body 作为 system prompt
    pub content_path: Option<String>, // 延迟加载时的文件路径
}

impl SkillDef {
    /// 按需加载完整 Skill 内容（如果 system_prompt 为空且 content_path 存在）
    pub fn load_content(&mut self) {
        if !self.system_prompt.is_empty() {
            return; // 已加载
        }
        if let Some(path) = &self.content_path {
            match fs::read_to_string(path) {
                Ok(content) => {
                    // 跳过 frontmatter，取 body
                    if content.starts_with("---") {
                        let parts: Vec<&str> = content.splitn(3, "---").collect();
                        if parts.len() >= 3 {
                            self.system_prompt = parts[2].trim().to_string();
                            return;
                        }
                    }
                    self.system_prompt = content;
                }
                Err(e) => {
                    eprintln!("  加载 Skill 内容失败 ({}): {}", path, e);
                }
            }
        }
    }
}

/// Skill 加载器
pub struct SkillLoader {
    skills: HashMap<String, SkillDef>,
}

impl SkillLoader {
    /// 从目录加载所有 .md Skill 文件（索引模式：只解析 frontmatter）
    pub fn load_from_dir(dir: &str) -> Self {
        let mut skills = HashMap::new();
        let path = Path::new(dir);

        if !path.exists() {
            eprintln!("Skill 目录不存在: {}", dir);
            return Self { skills };
        }

        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                let file_path = entry.path();
                if file_path.extension().map_or(false, |ext| ext == "md") {
                    match Self::parse_skill_index(&file_path) {
                        Ok(skill) => {
                            println!("  Skill: {} -- {}", skill.name, skill.description);
                            skills.insert(skill.name.clone(), skill);
                        }
                        Err(e) => {
                            eprintln!("  跳过 {:?}: {}", file_path, e);
                        }
                    }
                }
            }
        }

        Self { skills }
    }

    /// 解析 Skill 文件的 frontmatter（索引模式，不读 body）
    fn parse_skill_index(path: &Path) -> Result<SkillDef, String> {
        let content = fs::read_to_string(path)
            .map_err(|e| format!("读取失败: {}", e))?;

        // 解析 frontmatter (--- ... ---)
        if !content.starts_with("---") {
            return Err("缺少 frontmatter".into());
        }

        let parts: Vec<&str> = content.splitn(3, "---").collect();
        if parts.len() < 3 {
            return Err("frontmatter 格式错误".into());
        }

        let frontmatter = parts[1].trim();
        let body = parts[2].trim();

        // 简单解析 YAML-like frontmatter
        let mut name = String::new();
        let mut description = String::new();
        let mut tools: Vec<String> = Vec::new();

        for line in frontmatter.lines() {
            let line = line.trim();
            if let Some(val) = line.strip_prefix("name:") {
                name = val.trim().to_string();
            } else if let Some(val) = line.strip_prefix("description:") {
                description = val.trim().to_string();
            } else if let Some(val) = line.strip_prefix("tools:") {
                // 解析 [tool1, tool2, tool3]
                let val = val.trim();
                if val.starts_with('[') && val.ends_with(']') {
                    let inner = &val[1..val.len() - 1];
                    tools = inner
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
            }
        }

        if name.is_empty() {
            return Err("缺少 name 字段".into());
        }

        let content_path = path.to_str().map(String::from);

        Ok(SkillDef {
            name,
            description,
            tools,
            system_prompt: body.to_string(),
            content_path,
        })
    }

    /// 获取指定 Skill（返回 clone，调用方可 load_content）
    pub fn get(&self, name: &str) -> Option<&SkillDef> {
        self.skills.get(name)
    }

    /// 列出所有 Skill
    pub fn list(&self) -> Vec<&SkillDef> {
        self.skills.values().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_parse_skill() {
        let dir = "data/test_skills_parse";
        let _ = fs::create_dir_all(dir);

        let skill_content = r#"---
name: test_skill
description: A test skill
tools: [tool_a, tool_b]
---
This is the skill body.
"#;
        let file_path = format!("{}/test.md", dir);
        let mut f = fs::File::create(&file_path).unwrap();
        f.write_all(skill_content.as_bytes()).unwrap();

        let loader = SkillLoader::load_from_dir(dir);
        let skill = loader.get("test_skill");
        assert!(skill.is_some(), "Should find test_skill");
        let skill = skill.unwrap();
        assert_eq!(skill.name, "test_skill");
        assert_eq!(skill.description, "A test skill");
        assert_eq!(skill.tools, vec!["tool_a", "tool_b"]);
        assert!(skill.system_prompt.contains("skill body"));

        // cleanup
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_load_content() {
        let dir = "data/test_skills_load";
        let _ = fs::create_dir_all(dir);

        let skill_content = r#"---
name: lazy_skill
description: Lazy loaded
tools: []
---
Lazy body content here.
"#;
        let file_path = format!("{}/lazy.md", dir);
        fs::write(&file_path, skill_content).unwrap();

        // Create a SkillDef with empty system_prompt to simulate lazy loading
        let mut skill = SkillDef {
            name: "lazy_skill".into(),
            description: "Lazy loaded".into(),
            tools: vec![],
            system_prompt: String::new(),
            content_path: Some(file_path),
        };

        assert!(skill.system_prompt.is_empty());
        skill.load_content();
        assert!(skill.system_prompt.contains("Lazy body content"));

        // cleanup
        let _ = fs::remove_dir_all(dir);
    }
}
