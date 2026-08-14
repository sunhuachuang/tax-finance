//! Line-delimited JSON-RPC plumbing for the MCP stdio transport.

use serde_json::{Value, json};

use crate::tools::{Context, dispatch, tool_definitions};

pub struct Server {
    ctx: Context,
}

impl Server {
    pub fn new(ctx: Context) -> Self {
        Server { ctx }
    }

    /// One request line in, at most one response line out. Notifications and
    /// unparseable lines produce nothing — a broken peer cannot make this
    /// server write a broken frame.
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        let message: Value = serde_json::from_str(line).ok()?;
        self.handle(&message).map(|v| v.to_string())
    }

    pub fn handle(&mut self, message: &Value) -> Option<Value> {
        let id = message.get("id")?.clone();
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let params = message.get("params").cloned().unwrap_or_else(|| json!({}));

        let result = match method {
            "initialize" => Ok(json!({
                "protocolVersion": params
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or("2025-06-18"),
                "capabilities": { "tools": {} },
                "serverInfo": {
                    "name": "taxmcp",
                    "version": env!("CARGO_PKG_VERSION"),
                },
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tool_definitions() })),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
                let (text, is_error) = match dispatch(&mut self.ctx, name, &arguments) {
                    Ok(value) => (
                        serde_json::to_string_pretty(&value).unwrap_or_else(|e| e.to_string()),
                        false,
                    ),
                    Err(message) => (message, true),
                };
                Ok(json!({
                    "content": [{ "type": "text", "text": text }],
                    "isError": is_error,
                }))
            }
            other => Err((-32601, format!("method not found: {other}"))),
        };

        Some(match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, message)) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": code, "message": message },
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn server() -> (Server, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Context {
            store: taxstore::Store::open_in_memory().unwrap(),
            data_dir: dir.path().to_path_buf(),
            rules_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../rules"),
        };
        (Server::new(ctx), dir)
    }

    fn call(server: &mut Server, msg: Value) -> Value {
        server.handle(&msg).expect("a request with an id gets a response")
    }

    #[test]
    fn the_handshake_works_and_notifications_are_silent() {
        let (mut server, _dir) = server();

        let response = call(
            &mut server,
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}),
        );
        assert_eq!(response["result"]["serverInfo"]["name"], "taxmcp");
        assert_eq!(response["result"]["protocolVersion"], "2025-06-18");

        // notifications/initialized has no id: no response at all.
        assert!(
            server
                .handle(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
                .is_none()
        );

        let response = call(&mut server, json!({"jsonrpc":"2.0","id":2,"method":"nope"}));
        assert_eq!(response["error"]["code"], -32601);
    }

    #[test]
    fn confirmation_is_not_reachable_over_mcp() {
        let (mut server, _dir) = server();
        let response = call(&mut server, json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}));
        let tools: Vec<String> = response["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();

        for read in ["review_queue", "gst_return", "ir3_summary", "list_documents"] {
            assert!(tools.contains(&read.to_string()), "missing {read}");
        }
        // The writes that exist only create pending records; the confirming
        // verbs must not exist at all.
        for forbidden in ["approve", "reject", "post", "reverse", "void", "set_status"] {
            assert!(
                !tools.iter().any(|t| t.contains(forbidden)),
                "tool list leaks a confirming verb: {forbidden}"
            );
        }

        // Calling one anyway is an error, not a silent success.
        let response = call(
            &mut server,
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
                   "params":{"name":"approve_draft","arguments":{}}}),
        );
        assert_eq!(response["result"]["isError"], true);
    }
}
