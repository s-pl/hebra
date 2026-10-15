//! Model Context Protocol server over stdio (newline-delimited JSON-RPC 2.0).
//!
//! Every tool maps to an `hb` command and returns the same JSON the CLI
//! prints with `--json`.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Parser;
use serde_json::{Value, json};

use crate::cli::Cli;
use crate::commands::execute;

const PROTOCOL_VERSION: &str = "2025-11-25";

struct Tool {
    name: &'static str,
    description: &'static str,
    properties: Value,
    required: &'static [&'static str],
    args: fn(&Value) -> Vec<String>,
}

fn s(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn list(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn opt(args: &mut Vec<String>, flag: &str, value: Option<String>) {
    if let Some(value) = value {
        args.push(flag.to_owned());
        args.push(value);
    }
}

fn tools() -> Vec<Tool> {
    let cwd = json!({ "type": "string", "description": "Workspace directory to run in (defaults to the server's directory)." });
    let rev = json!({ "type": "string", "description": "Revision: @, @-, <workspace>@, a bookmark, or a change/commit id prefix." });
    vec![
        Tool {
            name: "hb_status",
            description: "Working-copy change, changed files, conflicts, and coordination warnings (paths claimed or also changed by other agents).",
            properties: json!({ "cwd": cwd }),
            required: &[],
            args: |_| vec!["status".into()],
        },
        Tool {
            name: "hb_diff",
            description: "Unified diff of a change (default: the working copy).",
            properties: json!({ "cwd": cwd, "revision": rev, "stat": { "type": "boolean" } }),
            required: &[],
            args: |v| {
                let mut a = vec!["diff".into()];
                opt(&mut a, "--revision", s(v, "revision"));
                if v.get("stat").and_then(Value::as_bool).unwrap_or(false) {
                    a.push("--stat".into());
                }
                a
            },
        },
        Tool {
            name: "hb_log",
            description: "History of visible changes, newest first, with authors, agents and tasks.",
            properties: json!({ "cwd": cwd, "limit": { "type": "integer" } }),
            required: &[],
            args: |v| {
                let mut a = vec!["log".into()];
                opt(
                    &mut a,
                    "--limit",
                    v.get("limit")
                        .and_then(Value::as_u64)
                        .map(|n| n.to_string()),
                );
                a
            },
        },
        Tool {
            name: "hb_show",
            description: "One change in detail: metadata, provenance and full diff.",
            properties: json!({ "cwd": cwd, "revision": rev }),
            required: &[],
            args: |v| {
                let mut a = vec!["show".into()];
                a.extend(s(v, "revision"));
                a
            },
        },
        Tool {
            name: "hb_commit",
            description: "Describe the working-copy change and start a new empty one on top. Files are recorded automatically; there is no staging.",
            properties: json!({ "cwd": cwd, "message": { "type": "string" } }),
            required: &["message"],
            args: |v| {
                vec![
                    "commit".into(),
                    "--message".into(),
                    s(v, "message").unwrap_or_default(),
                ]
            },
        },
        Tool {
            name: "hb_describe",
            description: "Set the description of a change without starting a new one.",
            properties: json!({ "cwd": cwd, "message": { "type": "string" }, "revision": rev }),
            required: &["message"],
            args: |v| {
                let mut a = vec![
                    "describe".into(),
                    "--message".into(),
                    s(v, "message").unwrap_or_default(),
                ];
                opt(&mut a, "--revision", s(v, "revision"));
                a
            },
        },
        Tool {
            name: "hb_start",
            description: "Create an isolated workspace (own directory and change) for a task. Use one per agent to avoid stepping on each other. Returns the path to work in.",
            properties: json!({
                "cwd": cwd,
                "task": { "type": "string" },
                "agent": { "type": "string", "description": "Agent name recorded on its changes." },
                "name": { "type": "string" },
                "from": rev,
            }),
            required: &["task"],
            args: |v| {
                let mut a = vec!["start".into(), s(v, "task").unwrap_or_default()];
                opt(&mut a, "--agent", s(v, "agent"));
                opt(&mut a, "--name", s(v, "name"));
                opt(&mut a, "--from", s(v, "from"));
                a
            },
        },
        Tool {
            name: "hb_workspaces",
            description: "Who is working where: every workspace with its agent, task, changed files, claims and last activity.",
            properties: json!({ "cwd": cwd }),
            required: &[],
            args: |_| vec!["workspaces".into()],
        },
        Tool {
            name: "hb_merge",
            description: "Merge other changes (e.g. `agent-x@`) into a new change. Conflicts are recorded, never blocking.",
            properties: json!({ "cwd": cwd, "revisions": { "type": "array", "items": { "type": "string" } }, "message": { "type": "string" } }),
            required: &["revisions"],
            args: |v| {
                let mut a = vec!["merge".into()];
                a.extend(list(v, "revisions"));
                opt(&mut a, "--message", s(v, "message"));
                a
            },
        },
        Tool {
            name: "hb_resolve",
            description: "Mark conflicts as resolved (content conflicts also clear automatically once markers are removed).",
            properties: json!({ "cwd": cwd, "paths": { "type": "array", "items": { "type": "string" } } }),
            required: &[],
            args: |v| {
                let mut a = vec!["resolve".into()];
                a.extend(list(v, "paths"));
                a
            },
        },
        Tool {
            name: "hb_claim",
            description: "Reserve files or directories you are about to edit so other agents see it before conflicts happen.",
            properties: json!({
                "cwd": cwd,
                "paths": { "type": "array", "items": { "type": "string" } },
                "ttl": { "type": "string", "description": "e.g. 30m, 2h" },
                "note": { "type": "string" },
            }),
            required: &["paths"],
            args: |v| {
                let mut a = vec!["claim".into()];
                a.extend(list(v, "paths"));
                opt(&mut a, "--ttl", s(v, "ttl"));
                opt(&mut a, "--note", s(v, "note"));
                a
            },
        },
        Tool {
            name: "hb_release",
            description: "Release claims of this workspace (all if no paths).",
            properties: json!({ "cwd": cwd, "paths": { "type": "array", "items": { "type": "string" } } }),
            required: &[],
            args: |v| {
                let mut a = vec!["release".into()];
                a.extend(list(v, "paths"));
                a
            },
        },
        Tool {
            name: "hb_claims",
            description: "Active claims across all workspaces.",
            properties: json!({ "cwd": cwd }),
            required: &[],
            args: |_| vec!["claims".into()],
        },
        Tool {
            name: "hb_git_export",
            description: "Export history to a Git repository (a bare repo is created if missing) so it can be pushed to GitHub. Deterministic; provenance is kept as Hebra-* trailers.",
            properties: json!({
                "cwd": cwd,
                "dir": { "type": "string", "description": "Path of the Git repository to write to." },
                "branch": { "type": "string" },
                "revision": rev,
            }),
            required: &["dir"],
            args: |v| {
                let mut a = vec![
                    "git".into(),
                    "export".into(),
                    s(v, "dir").unwrap_or_default(),
                ];
                opt(&mut a, "--branch", s(v, "branch"));
                opt(&mut a, "--revision", s(v, "revision"));
                a
            },
        },
        Tool {
            name: "hb_git_import",
            description: "Import the history of a Git branch as a bookmark. Deterministic; commits exported by Hebra keep their change ids and provenance.",
            properties: json!({
                "cwd": cwd,
                "dir": { "type": "string", "description": "Path of the Git repository to read from." },
                "branch": { "type": "string" },
                "bookmark": { "type": "string" },
            }),
            required: &["dir"],
            args: |v| {
                let mut a = vec![
                    "git".into(),
                    "import".into(),
                    s(v, "dir").unwrap_or_default(),
                ];
                opt(&mut a, "--branch", s(v, "branch"));
                opt(&mut a, "--bookmark", s(v, "bookmark"));
                a
            },
        },
        Tool {
            name: "hb_push",
            description: "Publish work to the server. Agents push their own bookmark, e.g. bookmark agents/<name>/<task> with revision @-.",
            properties: json!({ "cwd": cwd, "bookmark": { "type": "string" }, "revision": rev, "remote": { "type": "string" } }),
            required: &["bookmark"],
            args: |v| {
                let mut a = vec![
                    "push".into(),
                    s(v, "remote").unwrap_or_else(|| "origin".into()),
                ];
                opt(&mut a, "--bookmark", s(v, "bookmark"));
                opt(&mut a, "--revision", s(v, "revision"));
                a
            },
        },
        Tool {
            name: "hb_pull",
            description: "Fetch bookmarks and changes from the server; local bookmarks fast-forward, divergence is reported.",
            properties: json!({ "cwd": cwd, "remote": { "type": "string" } }),
            required: &[],
            args: |v| {
                vec![
                    "pull".into(),
                    s(v, "remote").unwrap_or_else(|| "origin".into()),
                ]
            },
        },
        Tool {
            name: "hb_presence",
            description: "Who is working on the shared repository right now, on every machine, and which files they touch.",
            properties: json!({ "cwd": cwd, "remote": { "type": "string" } }),
            required: &[],
            args: |v| {
                vec![
                    "presence".into(),
                    s(v, "remote").unwrap_or_else(|| "origin".into()),
                ]
            },
        },
        Tool {
            name: "hb_review_open",
            description: "Ask a person to review and merge your pushed bookmark into main (or another target).",
            properties: json!({
                "cwd": cwd,
                "bookmark": { "type": "string" },
                "title": { "type": "string" },
                "body": { "type": "string" },
                "target": { "type": "string" },
            }),
            required: &["bookmark", "title"],
            args: |v| {
                let mut a = vec![
                    "review".into(),
                    "open".into(),
                    s(v, "bookmark").unwrap_or_default(),
                ];
                opt(&mut a, "--title", s(v, "title"));
                opt(&mut a, "--body", s(v, "body"));
                opt(&mut a, "--target", s(v, "target"));
                a
            },
        },
        Tool {
            name: "hb_reviews",
            description: "List open reviews on the server.",
            properties: json!({ "cwd": cwd }),
            required: &[],
            args: |_| vec!["review".into(), "list".into()],
        },
        Tool {
            name: "hb_undo",
            description: "Undo the last operation. Everything is in the operation log, so nothing is lost.",
            properties: json!({ "cwd": cwd }),
            required: &[],
            args: |_| vec!["undo".into()],
        },
    ]
}

fn call_tool(base: &Path, params: &Value) -> Value {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let Some(tool) = tools().into_iter().find(|t| t.name == name) else {
        return tool_error(&format!("unknown tool `{name}`"));
    };
    let cwd = s(&arguments, "cwd").map_or_else(|| base.to_path_buf(), |c| base.join(c));
    let mut argv = vec!["hb".to_owned()];
    argv.extend((tool.args)(&arguments));
    let cli = match Cli::try_parse_from(&argv) {
        Ok(cli) => cli,
        Err(e) => return tool_error(&e.to_string()),
    };
    match execute(cli.command, &cwd) {
        Ok(out) => json!({
            "content": [{ "type": "text", "text": serde_json::to_string_pretty(&out.json).unwrap_or_default() }],
            "structuredContent": out.json,
            "isError": false,
        }),
        Err(e) => tool_error(&format!("{e:#}")),
    }
}

fn tool_error(message: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": message }], "isError": true })
}

fn handle(base: &Path, method: &str, params: &Value) -> Result<Value, (i64, String)> {
    match method {
        "initialize" => {
            let version = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL_VERSION);
            Ok(json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "hebra", "title": "Hebra", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "Hebra is a version control system for many agents working at once. Start your own workspace with hb_start before editing, claim the paths you will touch with hb_claim, check hb_status for warnings, and finish with hb_commit. Files are recorded automatically; conflicts never block.",
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({
            "tools": tools().iter().map(|t| json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": { "type": "object", "properties": t.properties, "required": t.required },
            })).collect::<Vec<_>>()
        })),
        "tools/call" => Ok(call_tool(base, params)),
        _ => Err((-32601, format!("method not found: {method}"))),
    }
}

/// Serves requests from stdin until it closes.
pub fn serve(base: Option<PathBuf>) -> Result<()> {
    let base = match base {
        Some(b) => b,
        None => std::env::current_dir()?,
    };
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Err(e) => Some(
                json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": e.to_string() } }),
            ),
            Ok(msg) => {
                let id = msg.get("id").cloned();
                let method = msg
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
                // Notifications (no id) get no response.
                id.map(|id| match handle(&base, method, &params) {
                    Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                    Err((code, message)) => {
                        json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
                    }
                })
            }
        };
        if let Some(response) = response {
            writeln!(stdout, "{response}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}
