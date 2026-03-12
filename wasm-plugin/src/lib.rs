use serde::Deserialize;

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
#[derive(Debug, Clone)]
struct ToolSpec {
    name: String,
    description: String,
    parameters_schema: String,
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
#[derive(Debug, Clone)]
struct PluginResult {
    success: bool,
    output: String,
    error: Option<String>,
}

pub struct EmailTool;

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
#[derive(Debug, Deserialize)]
struct DispatchArgs {
    tool: String,
    account_id: Option<i64>,
    message_id: Option<String>,
    query: Option<String>,
    to: Option<String>,
    subject: Option<String>,
    body_text: Option<String>,
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
impl EmailTool {
    fn get_spec_impl() -> ToolSpec {
        ToolSpec {
            name: "email.dispatch".to_string(),
            description: "PRX email tool dispatcher. Supported tools: email.sync, email.list, email.get, email.search, email.send, email.reply".to_string(),
            parameters_schema: r#"{
  "type": "object",
  "properties": {
    "tool": {
      "type": "string",
      "enum": ["email.sync", "email.list", "email.get", "email.search", "email.send", "email.reply"],
      "description": "Email tool name to execute"
    },
    "account_id": { "type": "integer" },
    "message_id": { "type": "string" },
    "query": { "type": "string" },
    "to": { "type": "string" },
    "subject": { "type": "string" },
    "body_text": { "type": "string" }
  },
  "required": ["tool"]
}"#
            .to_string(),
        }
    }

    fn execute_impl(args_json: &str) -> PluginResult {
        let args: DispatchArgs = match serde_json::from_str(args_json) {
            Ok(v) => v,
            Err(e) => {
                return PluginResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("invalid json args: {e}")),
                }
            }
        };

        let account_id = args.account_id.unwrap_or_default();

        let output = match args.tool.as_str() {
            "email.sync" => serde_json::json!({
                "tool": "email.sync",
                "ok": true,
                "account_id": account_id,
                "note": "wasm tool shim invoked"
            }),
            "email.list" => serde_json::json!({
                "tool": "email.list",
                "ok": true,
                "account_id": account_id,
                "messages": []
            }),
            "email.get" => serde_json::json!({
                "tool": "email.get",
                "ok": true,
                "account_id": account_id,
                "message_id": args.message_id,
                "message": null
            }),
            "email.search" => serde_json::json!({
                "tool": "email.search",
                "ok": true,
                "account_id": account_id,
                "query": args.query,
                "messages": []
            }),
            "email.send" => serde_json::json!({
                "tool": "email.send",
                "ok": true,
                "account_id": account_id,
                "to": args.to,
                "subject": args.subject,
                "body_text": args.body_text,
                "status": "queued"
            }),
            "email.reply" => serde_json::json!({
                "tool": "email.reply",
                "ok": true,
                "account_id": account_id,
                "message_id": args.message_id,
                "body_text": args.body_text,
                "status": "queued"
            }),
            other => {
                return PluginResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("unsupported tool: {other}")),
                }
            }
        };

        PluginResult {
            success: true,
            output: output.to_string(),
            error: None,
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "tool",
    });
}

#[cfg(target_arch = "wasm32")]
mod wasm_exports {
    use super::{EmailTool, PluginResult, ToolSpec};
    use crate::bindings::exports::prx::plugin::tool_exports::{Guest, PluginResult as WitPluginResult, ToolSpec as WitToolSpec};

    impl Guest for EmailTool {
        fn get_spec() -> WitToolSpec {
            let ToolSpec {
                name,
                description,
                parameters_schema,
            } = EmailTool::get_spec_impl();
            WitToolSpec {
                name,
                description,
                parameters_schema,
            }
        }

        fn execute(args: String) -> WitPluginResult {
            let PluginResult {
                success,
                output,
                error,
            } = EmailTool::execute_impl(&args);
            WitPluginResult {
                success,
                output,
                error,
            }
        }
    }

    crate::bindings::export!(EmailTool with_types_in crate::bindings);
}

#[cfg(test)]
mod tests {
    use super::EmailTool;

    #[test]
    fn spec_contains_required_tools() {
        let spec = EmailTool::get_spec_impl();
        assert!(spec.parameters_schema.contains("email.sync"));
        assert!(spec.parameters_schema.contains("email.list"));
        assert!(spec.parameters_schema.contains("email.get"));
        assert!(spec.parameters_schema.contains("email.search"));
        assert!(spec.parameters_schema.contains("email.send"));
        assert!(spec.parameters_schema.contains("email.reply"));
    }

    #[test]
    fn execute_dispatch_works() {
        let out = EmailTool::execute_impl(r#"{"tool":"email.list","account_id":1}"#);
        assert!(out.success);
        assert!(out.output.contains("\"email.list\""));
    }
}
