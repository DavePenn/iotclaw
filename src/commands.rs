use std::collections::HashMap;

/// 命令处理结果
pub enum CommandResult {
    /// 正常输出
    Output(String),
    /// 需要退出程序
    Exit,
    /// 继续（命令已处理，无额外输出）
    Continue,
}

/// 命令定义
pub struct Command {
    pub name: String,
    pub aliases: Vec<String>,
    pub description: String,
}

/// 命令注册表
pub struct CommandRegistry {
    commands: HashMap<String, usize>,  // name/alias -> index
    defs: Vec<Command>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self {
            commands: HashMap::new(),
            defs: Vec::new(),
        }
    }

    /// 注册命令
    pub fn register(&mut self, cmd: Command) {
        let idx = self.defs.len();
        self.commands.insert(format!("/{}", cmd.name), idx);
        for alias in &cmd.aliases {
            self.commands.insert(format!("/{}", alias), idx);
        }
        self.defs.push(cmd);
    }

    /// 查找命令
    pub fn find(&self, name: &str) -> Option<&Command> {
        self.commands.get(name).map(|&idx| &self.defs[idx])
    }

    /// 列出所有命令（去重，只列主名称）
    pub fn list(&self) -> Vec<(&str, &str)> {
        self.defs
            .iter()
            .map(|cmd| (cmd.name.as_str(), cmd.description.as_str()))
            .collect()
    }
}

/// 构建默认命令注册表
pub fn build_default_registry() -> CommandRegistry {
    let mut reg = CommandRegistry::new();

    let cmds = vec![
        ("quit", vec!["exit", "q"], "退出程序"),
        ("reset", vec![], "重置对话"),
        ("skills", vec![], "列出所有 Skill"),
        ("skill", vec![], "切换 Skill"),
        ("memory", vec![], "查看记忆"),
        ("feishu", vec![], "发送消息到飞书"),
        ("wechat", vec![], "发送消息到微信"),
        ("stop", vec![], "中断当前处理"),
        ("restart", vec![], "重启 Agent"),
        ("status", vec![], "查看状态"),
        ("cron", vec![], "定时任务管理"),
        ("bind", vec![], "绑定身份"),
        ("restore", vec![], "恢复对话"),
        ("help", vec![], "显示帮助"),
    ];

    for (name, aliases, desc) in cmds {
        reg.register(Command {
            name: name.to_string(),
            aliases: aliases.into_iter().map(String::from).collect(),
            description: desc.to_string(),
        });
    }

    reg
}
