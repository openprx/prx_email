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

const ENABLE_REAL_NETWORK_ENV: &str = "PRX_EMAIL_ENABLE_REAL_NETWORK";
const ERR_UNSUPPORTED_TOOL: &str = "EMAIL_UNSUPPORTED_TOOL";
const ERR_VALIDATION: &str = "EMAIL_VALIDATION";
const ERR_REAL_NETWORK_DISABLED: &str = "EMAIL_REAL_NETWORK_DISABLED";
const ERR_NOT_IMPLEMENTED: &str = "EMAIL_NOT_IMPLEMENTED";

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
            name: "email.execute".to_string(),
            description: "PRX email executor. Supported tools: email.sync, email.list, email.get, email.search, email.send, email.reply".to_string(),
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
                return controlled_error(ERR_VALIDATION, format!("invalid json args: {e}"));
            }
        };

        if !is_supported_tool(&args.tool) {
            return controlled_error(
                ERR_UNSUPPORTED_TOOL,
                format!("unsupported tool: {}", args.tool),
            );
        }

        if let Some(msg) = validate_required_fields(&args) {
            return controlled_error(ERR_VALIDATION, msg);
        }

        let allow_network = is_real_network_enabled();
        match args.tool.as_str() {
            "email.sync" => execute_sync(&args, allow_network),
            "email.list" => execute_list(&args),
            "email.get" => execute_get(&args),
            "email.search" => execute_search(&args),
            "email.send" => execute_send(&args, allow_network),
            "email.reply" => execute_reply(&args, allow_network),
            _ => controlled_error(
                ERR_UNSUPPORTED_TOOL,
                format!("unsupported tool: {}", args.tool),
            ),
        }
    }
}

fn controlled_error(code: &str, message: String) -> PluginResult {
    PluginResult {
        success: false,
        output: serde_json::json!({
            "ok": false,
            "error_code": code,
            "message": message,
        })
        .to_string(),
        error: Some(message),
    }
}

fn controlled_success(value: serde_json::Value) -> PluginResult {
    PluginResult {
        success: true,
        output: value.to_string(),
        error: None,
    }
}

fn guard_real_network(tool: &str) -> PluginResult {
    PluginResult {
        success: false,
        output: serde_json::json!({
            "tool": tool,
            "ok": false,
            "error_code": ERR_REAL_NETWORK_DISABLED,
            "guard": "real-network-disabled",
            "hint": format!("Set {ENABLE_REAL_NETWORK_ENV}=1 to enable real IMAP/SMTP execution")
        })
        .to_string(),
        error: Some("real network execution disabled by policy".to_string()),
    }
}

fn execute_sync(args: &DispatchArgs, allow_network: bool) -> PluginResult {
    if !allow_network {
        return guard_real_network(&args.tool);
    }

    controlled_error(
        ERR_NOT_IMPLEMENTED,
        "email.sync is routed but real network execution is intentionally not implemented in wasm-plugin".to_string(),
    )
}

fn execute_list(args: &DispatchArgs) -> PluginResult {
    controlled_success(serde_json::json!({
        "ok": true,
        "tool": args.tool,
        "account_id": args.account_id,
        "items": [],
        "source": "controlled-result"
    }))
}

fn execute_get(args: &DispatchArgs) -> PluginResult {
    controlled_success(serde_json::json!({
        "ok": true,
        "tool": args.tool,
        "account_id": args.account_id,
        "message": {
            "id": args.message_id,
            "subject": null,
            "from": null,
            "snippet": null
        },
        "source": "controlled-result"
    }))
}

fn execute_search(args: &DispatchArgs) -> PluginResult {
    controlled_success(serde_json::json!({
        "ok": true,
        "tool": args.tool,
        "account_id": args.account_id,
        "query": args.query,
        "items": [],
        "source": "controlled-result"
    }))
}

fn execute_send(args: &DispatchArgs, allow_network: bool) -> PluginResult {
    if !allow_network {
        return guard_real_network(&args.tool);
    }

    controlled_error(
        ERR_NOT_IMPLEMENTED,
        format!(
            "{} is routed but send is intentionally blocked in wasm-plugin",
            args.tool
        ),
    )
}

fn execute_reply(args: &DispatchArgs, allow_network: bool) -> PluginResult {
    if !allow_network {
        return guard_real_network(&args.tool);
    }

    controlled_error(
        ERR_NOT_IMPLEMENTED,
        format!(
            "{} is routed but send is intentionally blocked in wasm-plugin",
            args.tool
        ),
    )
}

fn is_real_network_enabled() -> bool {
    match std::env::var(ENABLE_REAL_NETWORK_ENV) {
        Ok(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}

fn is_supported_tool(tool: &str) -> bool {
    matches!(
        tool,
        "email.sync" | "email.list" | "email.get" | "email.search" | "email.send" | "email.reply"
    )
}

fn validate_required_fields(args: &DispatchArgs) -> Option<String> {
    match args.tool.as_str() {
        "email.get" => {
            if args.message_id.as_deref().unwrap_or("").trim().is_empty() {
                return Some("message_id is required for email.get".to_string());
            }
        }
        "email.search" => {
            if args.query.as_deref().unwrap_or("").trim().is_empty() {
                return Some("query is required for email.search".to_string());
            }
        }
        "email.send" => {
            if args.to.as_deref().unwrap_or("").trim().is_empty() {
                return Some("to is required for email.send".to_string());
            }
            if args.subject.as_deref().unwrap_or("").trim().is_empty() {
                return Some("subject is required for email.send".to_string());
            }
            if args.body_text.as_deref().unwrap_or("").trim().is_empty() {
                return Some("body_text is required for email.send".to_string());
            }
        }
        "email.reply" => {
            if args.message_id.as_deref().unwrap_or("").trim().is_empty() {
                return Some("message_id is required for email.reply".to_string());
            }
            if args.body_text.as_deref().unwrap_or("").trim().is_empty() {
                return Some("body_text is required for email.reply".to_string());
            }
        }
        _ => {}
    }

    if let Some(account_id) = args.account_id {
        if account_id <= 0 {
            return Some("account_id must be greater than 0".to_string());
        }
    }

    None
}

#[cfg(target_arch = "wasm32")]
mod bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "tool",
        generate_all,
    });
}

#[cfg(target_arch = "wasm32")]
mod wasm_exports {
    use super::{EmailTool, PluginResult, ToolSpec};
    use crate::bindings::exports::prx::plugin::tool_exports::{
        Guest, PluginResult as WitPluginResult, ToolSpec as WitToolSpec,
    };

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
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn parse_output(json: &str) -> serde_json::Value {
        serde_json::from_str(json).expect("valid json output")
    }

    #[test]
    fn spec_contains_required_tools() {
        let spec = EmailTool::get_spec_impl();
        assert_eq!(spec.name, "email.execute");
        assert!(spec.parameters_schema.contains("email.sync"));
        assert!(spec.parameters_schema.contains("email.list"));
        assert!(spec.parameters_schema.contains("email.get"));
        assert!(spec.parameters_schema.contains("email.search"));
        assert!(spec.parameters_schema.contains("email.send"));
        assert!(spec.parameters_schema.contains("email.reply"));
    }

    #[test]
    fn execute_list_returns_controlled_success() {
        let out = EmailTool::execute_impl(r#"{"tool":"email.list","account_id":1}"#);
        assert!(out.success);
        assert!(out.error.is_none());

        let payload = parse_output(&out.output);
        assert_eq!(payload["ok"], true);
        assert_eq!(payload["tool"], "email.list");
        assert_eq!(payload["source"], "controlled-result");
    }

    #[test]
    fn execute_send_blocked_when_network_disabled() {
        let _guard = env_lock().lock().expect("lock env");
        std::env::remove_var("PRX_EMAIL_ENABLE_REAL_NETWORK");
        let out = EmailTool::execute_impl(
            r#"{"tool":"email.send","account_id":1,"to":"a@b.com","subject":"s","body_text":"b"}"#,
        );

        assert!(!out.success);
        let payload = parse_output(&out.output);
        assert_eq!(payload["error_code"], "EMAIL_REAL_NETWORK_DISABLED");
    }

    #[test]
    fn execute_send_controlled_error_when_network_enabled() {
        let _guard = env_lock().lock().expect("lock env");
        std::env::set_var("PRX_EMAIL_ENABLE_REAL_NETWORK", "1");
        let out = EmailTool::execute_impl(
            r#"{"tool":"email.send","account_id":1,"to":"a@b.com","subject":"s","body_text":"b"}"#,
        );
        std::env::remove_var("PRX_EMAIL_ENABLE_REAL_NETWORK");

        assert!(!out.success);
        let payload = parse_output(&out.output);
        assert_eq!(payload["error_code"], "EMAIL_NOT_IMPLEMENTED");
    }
}
