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

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
#[derive(Debug, Clone)]
struct HostResult {
    success: bool,
    output: String,
    error: Option<String>,
}

pub struct EmailTool;

const ENABLE_REAL_NETWORK_ENV: &str = "PRX_EMAIL_ENABLE_REAL_NETWORK";

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

trait HostExecutor {
    fn execute_tool(&self, tool: &str, args_json: &str, allow_network: bool) -> HostResult;
}

#[cfg(target_arch = "wasm32")]
struct WasmHostExecutor;

#[cfg(target_arch = "wasm32")]
impl HostExecutor for WasmHostExecutor {
    fn execute_tool(&self, tool: &str, args_json: &str, allow_network: bool) -> HostResult {
        let out = crate::bindings::prx::plugin::host_calls::execute_tool(tool, args_json, allow_network);
        HostResult {
            success: out.success,
            output: out.output,
            error: out.error,
        }
    }
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
        #[cfg(target_arch = "wasm32")]
        {
            return Self::execute_with(args_json, &WasmHostExecutor);
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = args_json;
            PluginResult {
                success: false,
                output: String::new(),
                error: Some("execute() is only available in wasm runtime".to_string()),
            }
        }
    }

    fn execute_with(args_json: &str, host: &dyn HostExecutor) -> PluginResult {
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

        if !is_supported_tool(&args.tool) {
            return PluginResult {
                success: false,
                output: String::new(),
                error: Some(format!("unsupported tool: {}", args.tool)),
            };
        }

        if let Some(msg) = validate_required_fields(&args) {
            return PluginResult {
                success: false,
                output: String::new(),
                error: Some(msg),
            };
        }

        let allow_network = is_real_network_enabled();
        if !allow_network && requires_network(&args.tool) {
            return PluginResult {
                success: false,
                output: serde_json::json!({
                    "tool": args.tool,
                    "ok": false,
                    "guard": "real-network-disabled",
                    "hint": format!("Set {ENABLE_REAL_NETWORK_ENV}=1 to enable real IMAP/SMTP execution")
                })
                .to_string(),
                error: Some("real network execution disabled by policy".to_string()),
            };
        }

        let host_out = host.execute_tool(&args.tool, args_json, allow_network);
        PluginResult {
            success: host_out.success,
            output: host_out.output,
            error: host_out.error,
        }
    }
}

fn is_real_network_enabled() -> bool {
    match std::env::var(ENABLE_REAL_NETWORK_ENV) {
        Ok(v) => matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"),
        Err(_) => false,
    }
}

fn is_supported_tool(tool: &str) -> bool {
    matches!(
        tool,
        "email.sync" | "email.list" | "email.get" | "email.search" | "email.send" | "email.reply"
    )
}

fn requires_network(tool: &str) -> bool {
    matches!(tool, "email.sync" | "email.send" | "email.reply")
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
    use super::{EmailTool, HostExecutor, HostResult};
    use std::sync::{Arc, Mutex, OnceLock};

    struct MockHost {
        calls: Arc<Mutex<Vec<(String, bool)>>>,
        response: HostResult,
    }

    impl HostExecutor for MockHost {
        fn execute_tool(&self, tool: &str, _args_json: &str, allow_network: bool) -> HostResult {
            self.calls
                .lock()
                .expect("lock")
                .push((tool.to_string(), allow_network));
            self.response.clone()
        }
    }

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn with_mock(response: HostResult) -> (MockHost, Arc<Mutex<Vec<(String, bool)>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        (
            MockHost {
                calls: calls.clone(),
                response,
            },
            calls,
        )
    }

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
    fn execute_sync_blocked_when_network_disabled() {
        let _guard = env_lock().lock().expect("lock env");
        std::env::remove_var("PRX_EMAIL_ENABLE_REAL_NETWORK");
        let (host, calls) = with_mock(HostResult {
            success: true,
            output: "{}".to_string(),
            error: None,
        });

        let out = EmailTool::execute_with(r#"{"tool":"email.sync","account_id":1}"#, &host);
        assert!(!out.success);
        assert!(out.error.unwrap_or_default().contains("disabled"));
        assert!(calls.lock().expect("lock").is_empty());
    }

    #[test]
    fn execute_send_calls_host_when_network_enabled() {
        let _guard = env_lock().lock().expect("lock env");
        std::env::set_var("PRX_EMAIL_ENABLE_REAL_NETWORK", "1");
        let (host, calls) = with_mock(HostResult {
            success: true,
            output: r#"{"ok":true,"tool":"email.send"}"#.to_string(),
            error: None,
        });

        let out = EmailTool::execute_with(
            r#"{"tool":"email.send","account_id":1,"to":"a@b.com","subject":"s","body_text":"b"}"#,
            &host,
        );

        assert!(out.success);
        let calls = calls.lock().expect("lock");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "email.send");
        assert!(calls[0].1);
        std::env::remove_var("PRX_EMAIL_ENABLE_REAL_NETWORK");
    }

    #[test]
    fn execute_reply_calls_host_when_network_enabled() {
        let _guard = env_lock().lock().expect("lock env");
        std::env::set_var("PRX_EMAIL_ENABLE_REAL_NETWORK", "true");
        let (host, calls) = with_mock(HostResult {
            success: true,
            output: r#"{"ok":true,"tool":"email.reply"}"#.to_string(),
            error: None,
        });

        let out = EmailTool::execute_with(
            r#"{"tool":"email.reply","account_id":1,"message_id":"m1","body_text":"reply"}"#,
            &host,
        );

        assert!(out.success);
        let calls = calls.lock().expect("lock");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "email.reply");
        assert!(calls[0].1);
        std::env::remove_var("PRX_EMAIL_ENABLE_REAL_NETWORK");
    }
}
