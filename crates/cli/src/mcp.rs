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

fn int(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_i64)
        .unwrap_or_default()
        .to_string()
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
            properties: json!({
                "cwd": cwd,
                "message": { "type": "string" },
                "task": { "type": "string", "description": "The task this work is for (e.g. `#12`); merging the review then closes it." },
            }),
            required: &["message"],
            args: |v| {
                let mut a = vec![
                    "commit".into(),
                    "--message".into(),
                    s(v, "message").unwrap_or_default(),
                ];
                opt(&mut a, "--task", s(v, "task"));
                a
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
            name: "hb_tasks",
            description: "Tasks on the server. state: `ready` (open, dependencies done: pick from these), `open`, `claimed`, `done`, `all`.",
            properties: json!({ "cwd": cwd, "state": { "type": "string" } }),
            required: &[],
            args: |v| {
                let mut a = vec!["task".into(), "list".into()];
                opt(&mut a, "--state", s(v, "state"));
                a
            },
        },
        Tool {
            name: "hb_task_show",
            description: "A task with its acceptance criteria and dependencies.",
            properties: json!({ "cwd": cwd, "id": { "type": "integer" } }),
            required: &["id"],
            args: |v| vec!["task".into(), "show".into(), int(v, "id")],
        },
        Tool {
            name: "hb_task_claim",
            description: "Claim a task so nobody else takes it while you work (a lease; claim again to renew). Then commit with `task` set to its number so merging your review closes it.",
            properties: json!({ "cwd": cwd, "id": { "type": "integer" }, "minutes": { "type": "integer" } }),
            required: &["id"],
            args: |v| {
                let mut a = vec!["task".into(), "claim".into(), int(v, "id")];
                if v.get("minutes").is_some() {
                    a.push("--for".into());
                    a.push(int(v, "minutes"));
                }
                a
            },
        },
        Tool {
            name: "hb_task_release",
            description: "Give a claimed task back (you cannot finish it).",
            properties: json!({ "cwd": cwd, "id": { "type": "integer" } }),
            required: &["id"],
            args: |v| vec!["task".into(), "release".into(), int(v, "id")],
        },
        Tool {
            name: "hb_task_create",
            description: "Create a task (for example, to split your work into sub-tasks with `parent`, or to record follow-up work).",
            properties: json!({
                "cwd": cwd,
                "title": { "type": "string" },
                "body": { "type": "string" },
                "acceptance": { "type": "array", "items": { "type": "string" } },
                "parent": { "type": "integer" },
                "after": { "type": "array", "items": { "type": "integer" } },
            }),
            required: &["title"],
            args: |v| {
                let mut a = vec![
                    "task".into(),
                    "create".into(),
                    s(v, "title").unwrap_or_default(),
                ];
                opt(&mut a, "--body", s(v, "body"));
                for criterion in list(v, "acceptance") {
                    a.push("--accept".into());
                    a.push(criterion);
                }
                if v.get("parent").is_some() {
                    a.push("--parent".into());
                    a.push(int(v, "parent"));
                }
                for dep in v
                    .get("after")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                {
                    a.push("--after".into());
                    a.push(dep.as_i64().unwrap_or_default().to_string());
                }
                a
            },
        },
        Tool {
            name: "hb_review_show",
            description: "A review: what it changes, its approvals, its checks (with states), and every comment, including line comments to address.",
            properties: json!({ "cwd": cwd, "id": { "type": "integer" } }),
            required: &["id"],
            args: |v| vec!["review".into(), "show".into(), int(v, "id")],
        },
        Tool {
            name: "hb_review_comment",
            description: "Comment on a review, optionally on a file line (answer reviewers, explain a change).",
            properties: json!({
                "cwd": cwd,
                "id": { "type": "integer" },
                "body": { "type": "string" },
                "path": { "type": "string" },
                "line": { "type": "integer" },
            }),
            required: &["id", "body"],
            args: |v| {
                let mut a = vec![
                    "review".into(),
                    "comment".into(),
                    int(v, "id"),
                    s(v, "body").unwrap_or_default(),
                ];
                opt(&mut a, "--path", s(v, "path"));
                if v.get("line").is_some() {
                    a.push("--line".into());
                    a.push(int(v, "line"));
                }
                a
            },
        },
        Tool {
            name: "hb_checks",
            description: "Checks of a commit or bookmark (or recent runs): state, summary, credits. Use hb_check_log to read a failure.",
            properties: json!({ "cwd": cwd, "revision": { "type": "string" } }),
            required: &[],
            args: |v| {
                let mut a = vec!["checks".into(), "list".into()];
                a.extend(s(v, "revision"));
                a
            },
        },
        Tool {
            name: "hb_check_log",
            description: "The full log of a check run (to see why it failed).",
            properties: json!({ "cwd": cwd, "id": { "type": "string" } }),
            required: &["id"],
            args: |v| {
                vec![
                    "checks".into(),
                    "log".into(),
                    s(v, "id").unwrap_or_default(),
                ]
            },
        },
        Tool {
            name: "hb_verify_log",
            description: "Verify the server's transparency log (hash chain, signed head, bookmarks).",
            properties: json!({ "cwd": cwd }),
            required: &[],
            args: |_| vec!["verify-log".into()],
        },
        Tool {
            name: "hb_whoami",
            description: "Who you are on the server, and who you act for.",
            properties: json!({ "cwd": cwd }),
            required: &[],
            args: |_| vec!["whoami".into()],
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
    let result = match crate::global::execute(cli.command, &cwd, true) {
        Ok(result) => result,
        Err(command) => execute(*command, &cwd),
    };
    match result {
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
                "instructions": "Hebra is a version control system for many agents working at once. Pick work with hb_tasks (state `ready`) and claim it with hb_task_claim. Start your own workspace with hb_start before editing, claim the paths you will touch with hb_claim, check hb_status for warnings, and finish with hb_commit (pass `task`). Publish with hb_push to agents/<you>/<topic> and ask for review with hb_review_open; read feedback with hb_review_show and failing checks with hb_check_log. Files are recorded automatically; conflicts never block.",
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
