use serde_json::{json, Value};

/// Feishu Plugin action types
#[derive(Debug, Clone)]
pub enum PluginAction {
    ButtonClick { action_id: String, value: Value },
    FormSubmit { form_data: Value },
    SelectChange { action_id: String, selected: String },
    Unknown { raw: Value },
}

/// Feishu Plugin — handles plugin interaction actions and generates interactive cards
pub struct FeishuPlugin;

impl FeishuPlugin {
    /// Parse a plugin action from JSON
    pub fn parse_action(action_json: &Value) -> PluginAction {
        let action_type = action_json.get("action_type")
            .or_else(|| action_json.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        match action_type {
            "button_click" | "button" => {
                let action_id = action_json.get("action_id")
                    .or_else(|| action_json.pointer("/action/tag"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let value = action_json.get("value")
                    .or_else(|| action_json.pointer("/action/value"))
                    .cloned()
                    .unwrap_or(Value::Null);
                PluginAction::ButtonClick { action_id, value }
            }
            "form_submit" | "form" => {
                let form_data = action_json.get("form_data")
                    .or_else(|| action_json.get("value"))
                    .cloned()
                    .unwrap_or(json!({}));
                PluginAction::FormSubmit { form_data }
            }
            "select_change" | "select_static" | "select" => {
                let action_id = action_json.get("action_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let selected = action_json.get("selected")
                    .or_else(|| action_json.pointer("/option/value"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                PluginAction::SelectChange { action_id, selected }
            }
            _ => PluginAction::Unknown { raw: action_json.clone() },
        }
    }

    /// Handle a plugin action and return a response value
    pub fn handle_action(action_json: &Value) -> Value {
        let action = Self::parse_action(action_json);

        match action {
            PluginAction::ButtonClick { action_id, value } => {
                json!({
                    "status": "ok",
                    "action_type": "button_click",
                    "action_id": action_id,
                    "value": value,
                    "message": format!("Button '{}' clicked", action_id)
                })
            }
            PluginAction::FormSubmit { form_data } => {
                json!({
                    "status": "ok",
                    "action_type": "form_submit",
                    "form_data": form_data,
                    "message": "Form submitted"
                })
            }
            PluginAction::SelectChange { action_id, selected } => {
                json!({
                    "status": "ok",
                    "action_type": "select_change",
                    "action_id": action_id,
                    "selected": selected,
                    "message": format!("Selection changed to '{}'", selected)
                })
            }
            PluginAction::Unknown { raw } => {
                json!({
                    "status": "unknown_action",
                    "raw": raw,
                    "message": "Unknown action type"
                })
            }
        }
    }

    /// Generate a plugin interactive card with buttons
    pub fn build_button_card(title: &str, content: &str, buttons: &[(&str, &str, &str)]) -> Value {
        let button_elements: Vec<Value> = buttons.iter().map(|(text, action_id, action_value)| {
            json!({
                "tag": "button",
                "text": {
                    "tag": "plain_text",
                    "content": text
                },
                "type": "primary",
                "value": {
                    "action": action_value,
                    "action_id": action_id
                }
            })
        }).collect();

        let mut elements = vec![
            json!({
                "tag": "markdown",
                "content": content
            })
        ];

        if !button_elements.is_empty() {
            elements.push(json!({
                "tag": "action",
                "actions": button_elements
            }));
        }

        json!({
            "config": { "wide_screen_mode": true },
            "header": {
                "title": { "tag": "plain_text", "content": title },
                "template": "blue"
            },
            "elements": elements
        })
    }

    /// Generate a plugin form card
    pub fn build_form_card(title: &str, fields: &[(&str, &str, &str)]) -> Value {
        // fields: [(label, name, placeholder)]
        let form_elements: Vec<Value> = fields.iter().map(|(label, name, placeholder)| {
            json!({
                "tag": "div",
                "text": {
                    "tag": "plain_text",
                    "content": label
                },
                "extra": {
                    "tag": "input",
                    "name": name,
                    "placeholder": {
                        "tag": "plain_text",
                        "content": placeholder
                    }
                }
            })
        }).collect();

        let mut elements = form_elements;
        elements.push(json!({
            "tag": "action",
            "actions": [{
                "tag": "button",
                "text": { "tag": "plain_text", "content": "Submit" },
                "type": "primary",
                "value": { "action": "form_submit" }
            }]
        }));

        json!({
            "config": { "wide_screen_mode": true },
            "header": {
                "title": { "tag": "plain_text", "content": title },
                "template": "green"
            },
            "elements": elements
        })
    }

    /// Generate a plugin select card
    pub fn build_select_card(title: &str, label: &str, action_id: &str, options: &[(&str, &str)]) -> Value {
        let option_elements: Vec<Value> = options.iter().map(|(text, value)| {
            json!({
                "text": { "tag": "plain_text", "content": text },
                "value": value
            })
        }).collect();

        json!({
            "config": { "wide_screen_mode": true },
            "header": {
                "title": { "tag": "plain_text", "content": title },
                "template": "indigo"
            },
            "elements": [{
                "tag": "div",
                "text": { "tag": "plain_text", "content": label },
                "extra": {
                    "tag": "select_static",
                    "action_id": action_id,
                    "placeholder": { "tag": "plain_text", "content": "Select..." },
                    "options": option_elements
                }
            }]
        })
    }
}
