use serde_json::{json, Value};

/// Approval card for sensitive tool execution
/// Sent to the user before executing high-risk operations (exec_command, control_device, etc.)
pub struct ApprovalCard {
    pub action_description: String,
    pub tool_name: String,
    pub tool_args: Value,
    pub approve_callback: String,
    pub reject_callback: String,
}

impl ApprovalCard {
    /// Build an approval card for a sensitive tool call
    pub fn new(tool_name: &str, args: &Value) -> Self {
        let action_description = match tool_name {
            "exec_command" => {
                let cmd = args["command"].as_str().unwrap_or("<unknown>");
                format!("Execute command: {}", cmd)
            }
            "control_device" => {
                let device = args["device_id"].as_str().unwrap_or("<unknown>");
                let action = args["action"].as_str().unwrap_or("<unknown>");
                format!("Control device: {} -> {}", device, action)
            }
            _ => format!("Execute tool: {} with args: {}", tool_name, args),
        };

        let callback_id = uuid::Uuid::new_v4().to_string();

        Self {
            action_description,
            tool_name: tool_name.to_string(),
            tool_args: args.clone(),
            approve_callback: format!("approve_{}", callback_id),
            reject_callback: format!("reject_{}", callback_id),
        }
    }

    /// Build a Feishu interactive card JSON for this approval request
    pub fn build_feishu_card(&self) -> Value {
        json!({
            "config": {
                "wide_screen_mode": true
            },
            "header": {
                "title": {
                    "tag": "plain_text",
                    "content": "Action Approval Required"
                },
                "template": "orange"
            },
            "elements": [
                {
                    "tag": "markdown",
                    "content": format!(
                        "**Tool:** `{}`\n**Action:** {}\n**Args:**\n```json\n{}\n```",
                        self.tool_name,
                        self.action_description,
                        serde_json::to_string_pretty(&self.tool_args).unwrap_or_default()
                    )
                },
                {
                    "tag": "action",
                    "actions": [
                        {
                            "tag": "button",
                            "text": {
                                "tag": "plain_text",
                                "content": "Approve"
                            },
                            "type": "primary",
                            "value": {
                                "action": &self.approve_callback
                            }
                        },
                        {
                            "tag": "button",
                            "text": {
                                "tag": "plain_text",
                                "content": "Reject"
                            },
                            "type": "danger",
                            "value": {
                                "action": &self.reject_callback
                            }
                        }
                    ]
                }
            ]
        })
    }

    /// Check if a tool requires approval before execution
    pub fn requires_approval(tool_name: &str, args: &Value) -> bool {
        match tool_name {
            "exec_command" => true,
            "control_device" => {
                // Only high-risk device actions need approval
                let action = args["action"].as_str().unwrap_or("");
                matches!(
                    action,
                    "lock" | "unlock" | "open" | "close" | "reset" | "factory_reset"
                )
            }
            _ => false,
        }
    }
}

/// Build an approval card for use in the agent loop
pub fn build_approval_card(tool_name: &str, args: &Value) -> Value {
    let card = ApprovalCard::new(tool_name, args);
    card.build_feishu_card()
}
