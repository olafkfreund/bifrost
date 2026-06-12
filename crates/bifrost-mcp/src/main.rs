//! Bifrost MCP server (#241) — exposes **read-only** grounded migration context
//! over the Model Context Protocol (stdio, newline-delimited JSON-RPC 2.0), so an
//! agent (the Copilot coding agent, Claude, an IDE) can resolve conversion gaps
//! grounded in the real portfolio, forecast, coverage, assessment, readiness, and
//! report — instead of guessing.
//!
//! It is deliberately **read-only**: it only surfaces context. Every mutation
//! (convert, approve, commit) stays behind the control-plane API's human-approval
//! and audit-logged path, per Bifrost's review-first and attestable rules. The
//! server proxies the control plane at `BIFROST_API_URL` (default
//! `http://127.0.0.1:8080`), so it inherits the API's auth and air-gap posture.
//!
//! Protocol routing is a pure function ([`route`]) so it is fully unit-tested
//! without a live API; `main` does the I/O.

use std::io::{BufRead, Write};

use serde_json::{json, Value};

const PROTOCOL_VERSION: &str = "2025-06-18";

/// A read-only tool backed by a control-plane API `GET`.
struct ApiTool {
    name: &'static str,
    description: &'static str,
    path: &'static str,
    /// Whether the tool accepts an optional `project` argument (a query param).
    project_arg: bool,
}

const API_TOOLS: &[ApiTool] = &[
    ApiTool {
        name: "bifrost_portfolio",
        description: "The migration portfolio: every pipeline with its project, classification (YAML/classic), risk band + score, converted ratio, and proposal status.",
        path: "/api/portfolio",
        project_arg: false,
    },
    ApiTool {
        name: "bifrost_forecast",
        description: "Deterministic GitHub Actions cost + capacity forecast for the target org: monthly/annual cost, runner-minutes, and per-project breakdown.",
        path: "/api/forecast",
        project_arg: false,
    },
    ApiTool {
        name: "bifrost_coverage",
        description: "The completeness matrix: every Azure DevOps moving-part category mapped to its GitHub equivalent and a status (auto/review/manual/notInventoried).",
        path: "/api/completeness",
        project_arg: false,
    },
    ApiTool {
        name: "bifrost_assessment",
        description: "Source (Azure DevOps) assessment: pipeline mix, risk distribution, and inventory density (service connections, variable groups, secrets, runners, custom tasks).",
        path: "/api/source-stats",
        project_arg: false,
    },
    ApiTool {
        name: "bifrost_readiness",
        description: "Target GitHub pre-flight readiness checklist: runners, Actions policy, OIDC federation, secrets, rulesets, ownership, rollback — each with a status.",
        path: "/api/readiness",
        project_arg: false,
    },
    ApiTool {
        name: "bifrost_report",
        description: "The pre-migration status report (Markdown), optionally scoped to one project via the `project` argument.",
        path: "/api/report",
        project_arg: true,
    },
];

/// Tool definitions (JSON Schema) advertised to the client.
fn tools_list() -> Value {
    let empty_schema =
        || json!({ "type": "object", "properties": {}, "additionalProperties": false });
    let mut tools: Vec<Value> = API_TOOLS
        .iter()
        .map(|t| {
            let schema = if t.project_arg {
                json!({
                    "type": "object",
                    "properties": { "project": { "type": "string", "description": "Scope to a single project (optional)." } },
                    "additionalProperties": false
                })
            } else {
                empty_schema()
            };
            json!({ "name": t.name, "description": t.description, "inputSchema": schema })
        })
        .collect();
    tools.push(json!({
        "name": "validate_workflow",
        "description": "Basic structural validation of a GitHub Actions workflow YAML (checks for top-level `on:` and `jobs:`). A quick sanity check, not a full run.",
        "inputSchema": {
            "type": "object",
            "properties": { "yaml": { "type": "string", "description": "The workflow YAML to check." } },
            "required": ["yaml"],
            "additionalProperties": false
        }
    }));
    json!(tools)
}

/// What `main` should do with a parsed request. Keeps [`route`] pure.
#[derive(Debug, PartialEq)]
enum Action {
    /// A notification — send nothing back.
    Notify,
    /// A complete JSON-RPC response to write.
    Respond(Value),
    /// A `tools/call` that needs an API GET; `main` fetches and wraps the body.
    Fetch {
        id: Value,
        path: String,
        query: Vec<(String, String)>,
    },
}

fn ok(id: Option<Value>, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "result": result })
}

fn err(id: Option<Value>, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "error": { "code": code, "message": message } })
}

fn text_result(text: String, is_error: bool) -> Value {
    json!({ "content": [ { "type": "text", "text": text } ], "isError": is_error })
}

/// Route a parsed JSON-RPC request to an [`Action`]. Pure — no I/O.
fn route(req: &Value) -> Action {
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    let id = req.get("id").cloned();
    match method {
        "initialize" => Action::Respond(ok(
            id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "bifrost-mcp", "version": env!("CARGO_PKG_VERSION") }
            }),
        )),
        "notifications/initialized" => Action::Notify,
        "ping" => Action::Respond(ok(id, json!({}))),
        "tools/list" => Action::Respond(ok(id, json!({ "tools": tools_list() }))),
        "tools/call" => route_tool_call(id, req.get("params")),
        _ => {
            // No id => notification we don't handle; with id => method not found.
            if id.is_none() {
                Action::Notify
            } else {
                Action::Respond(err(id, -32601, "method not found"))
            }
        }
    }
}

fn route_tool_call(id: Option<Value>, params: Option<&Value>) -> Action {
    let params = params.cloned().unwrap_or_else(|| json!({}));
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    if name == "validate_workflow" {
        let yaml = args.get("yaml").and_then(Value::as_str).unwrap_or("");
        return Action::Respond(ok(id, text_result(validate_workflow(yaml), false)));
    }
    if let Some(tool) = API_TOOLS.iter().find(|t| t.name == name) {
        let mut query = Vec::new();
        if tool.project_arg {
            if let Some(p) = args.get("project").and_then(Value::as_str) {
                if !p.is_empty() {
                    query.push(("project".to_string(), p.to_string()));
                }
            }
        }
        return Action::Fetch {
            id: id.unwrap_or(Value::Null),
            path: tool.path.to_string(),
            query,
        };
    }
    Action::Respond(ok(id, text_result(format!("unknown tool: {name}"), true)))
}

/// A quick structural sanity check on a converted workflow. Heuristic by design —
/// the real validation is running it (see the sandbox parity path).
fn validate_workflow(yaml: &str) -> String {
    if yaml.trim().is_empty() {
        return "Invalid: the workflow is empty.".to_string();
    }
    let has = |key: &str| {
        yaml.lines()
            .any(|l| l.trim_start() == key || l.trim_start().starts_with(&format!("{} ", key)))
    };
    let mut issues = Vec::new();
    if !has("on:") {
        issues.push("no top-level `on:` trigger");
    }
    if !has("jobs:") {
        issues.push("no top-level `jobs:` mapping");
    }
    if issues.is_empty() {
        "Structurally plausible: has `on:` and `jobs:`. This is a basic check — run the workflow in a sandbox to validate fully.".to_string()
    } else {
        format!(
            "Possible issues: {}. (Basic structural check only.)",
            issues.join("; ")
        )
    }
}

/// Build the JSON-RPC response for a completed API fetch.
fn fetch_response(id: Value, path: &str, body: Result<String, String>) -> Value {
    let id = Some(id);
    match body {
        Ok(text) => ok(id, text_result(text, false)),
        Err(e) => ok(id, text_result(format!("error fetching {path}: {e}"), true)),
    }
}

async fn fetch(
    client: &reqwest::Client,
    base: &str,
    path: &str,
    query: &[(String, String)],
) -> Result<String, String> {
    let url = format!("{base}{path}");
    let mut rb = client.get(&url);
    if !query.is_empty() {
        rb = rb.query(query);
    }
    let resp = rb.send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if status.is_success() {
        Ok(text)
    } else {
        Err(format!("{status}: {text}"))
    }
}

fn main() -> anyhow::Result<()> {
    let base =
        std::env::var("BIFROST_API_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());
    eprintln!("bifrost-mcp: read-only migration context over MCP/stdio; proxying {base}");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let client = reqwest::Client::new();

    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let mut out = std::io::stdout();
    let mut line = String::new();

    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break; // EOF
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue, // ignore malformed lines rather than crash the stream
        };

        let response = match route(&req) {
            Action::Notify => None,
            Action::Respond(v) => Some(v),
            Action::Fetch { id, path, query } => {
                let body = rt.block_on(fetch(&client, &base, &path, &query));
                Some(fetch_response(id, &path, body))
            }
        };

        if let Some(response) = response {
            writeln!(out, "{}", serde_json::to_string(&response)?)?;
            out.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_advertises_tools_capability() {
        let req = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} });
        match route(&req) {
            Action::Respond(v) => {
                assert_eq!(v["result"]["protocolVersion"], PROTOCOL_VERSION);
                assert!(v["result"]["capabilities"]["tools"].is_object());
                assert_eq!(v["result"]["serverInfo"]["name"], "bifrost-mcp");
                assert_eq!(v["id"], 1);
            }
            other => panic!("expected Respond, got {other:?}"),
        }
    }

    #[test]
    fn tools_list_includes_every_api_tool_plus_validate() {
        let req = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
        let Action::Respond(v) = route(&req) else {
            panic!("expected Respond")
        };
        let names: Vec<&str> = v["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        for t in API_TOOLS {
            assert!(names.contains(&t.name), "missing tool {}", t.name);
        }
        assert!(names.contains(&"validate_workflow"));
    }

    #[test]
    fn tool_call_routes_to_the_api_path() {
        let req = json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "bifrost_forecast", "arguments": {} }
        });
        match route(&req) {
            Action::Fetch { id, path, query } => {
                assert_eq!(id, json!(3));
                assert_eq!(path, "/api/forecast");
                assert!(query.is_empty());
            }
            other => panic!("expected Fetch, got {other:?}"),
        }
    }

    #[test]
    fn report_tool_passes_the_project_query() {
        let req = json!({
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": { "name": "bifrost_report", "arguments": { "project": "Payments" } }
        });
        match route(&req) {
            Action::Fetch { path, query, .. } => {
                assert_eq!(path, "/api/report");
                assert_eq!(query, vec![("project".to_string(), "Payments".to_string())]);
            }
            other => panic!("expected Fetch, got {other:?}"),
        }
    }

    #[test]
    fn validate_workflow_is_local_and_flags_missing_keys() {
        let req = json!({
            "jsonrpc": "2.0", "id": 5, "method": "tools/call",
            "params": { "name": "validate_workflow", "arguments": { "yaml": "name: x\n" } }
        });
        let Action::Respond(v) = route(&req) else {
            panic!("expected Respond")
        };
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("on:"), "should flag missing on:");
        assert!(text.contains("jobs:"), "should flag missing jobs:");

        let good = validate_workflow("on:\n  push:\njobs:\n  build:\n    runs-on: ubuntu-latest\n");
        assert!(good.contains("plausible"));
    }

    #[test]
    fn notifications_produce_no_response() {
        let req = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert_eq!(route(&req), Action::Notify);
    }

    #[test]
    fn unknown_method_with_id_is_an_error() {
        let req = json!({ "jsonrpc": "2.0", "id": 9, "method": "no/such/method" });
        let Action::Respond(v) = route(&req) else {
            panic!("expected Respond")
        };
        assert_eq!(v["error"]["code"], -32601);
    }

    #[test]
    fn unknown_tool_returns_an_is_error_result() {
        let req = json!({
            "jsonrpc": "2.0", "id": 10, "method": "tools/call",
            "params": { "name": "bogus", "arguments": {} }
        });
        let Action::Respond(v) = route(&req) else {
            panic!("expected Respond")
        };
        assert_eq!(v["result"]["isError"], true);
    }
}
