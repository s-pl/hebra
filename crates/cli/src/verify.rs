//! The verifier agent's loop, run by a runner for `kind = "agent"` checks.
//!
//! The runner holds the code; the server holds the model. Each turn sends
//! the conversation to the server's gateway, executes the tools the model
//! asks for inside the checkout, and continues until the model calls
//! `verdict`. Tool output is data for the model, never instructions.

use std::fmt::Write as _;
use std::path::Path;
use std::time::Duration;

use anyhow::{Result, anyhow};
use hebra_core::ObjectId;
use hebra_core::diff;
use hebra_core::model::{Commit, Tree};
use hebra_core::store::{COMMIT, Store, TREE};
use serde_json::{Value, json};

use crate::client::Client;
use crate::runner::{Sink, max_turns, tool_run, truncate};

/// Longest a single command the verifier runs may take.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const MAX_READ: u64 = 200 * 1024;

fn tree_of(store: &Store, commit: &Commit) -> Result<Tree> {
    Ok(store.read_json(TREE, &commit.tree)?)
}

/// The change under review as a unified diff (against the review's target,
/// or the commit's first parent).
fn change(store: &Store, commit: &Commit, base: Option<&str>) -> Result<String> {
    let from = match base
        .map(ObjectId::parse)
        .transpose()?
        .or_else(|| commit.parents.first().copied())
    {
        Some(id) => tree_of(store, &store.read_json::<Commit>(COMMIT, &id)?)?,
        None => Tree::default(),
    };
    let to = tree_of(store, commit)?;
    let files = diff::diff_trees(store, &from, &to, true)?;
    let mut out = String::new();
    for f in &files {
        let _ = writeln!(out, "### {} (+{} -{})", f.path, f.additions, f.deletions);
        if let Some(patch) = &f.patch {
            out.push_str(patch);
            out.push('\n');
        }
    }
    Ok(if out.is_empty() {
        "(no file changes)".to_owned()
    } else {
        truncate(&out)
    })
}

fn inside(work: &Path, path: &str) -> Option<std::path::PathBuf> {
    let clean = path.trim().trim_start_matches("./");
    if clean.split(['/', '\\']).any(|p| p == "..")
        || Path::new(clean).is_absolute()
        || clean.contains(':')
    {
        return None;
    }
    Some(work.join(clean))
}

fn list_files(work: &Path, dir: &str) -> String {
    let Some(root) = inside(work, dir) else {
        return "path outside the repository".to_owned();
    };
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(rel) = p.strip_prefix(work) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
            if out.len() >= 2000 {
                return out.join("\n") + "\n… (more files)";
            }
        }
    }
    out.sort();
    out.join("\n")
}

fn search(work: &Path, query: &str) -> String {
    if query.is_empty() {
        return "empty query".to_owned();
    }
    let mut hits = Vec::new();
    for file in list_files(work, "").lines() {
        let Ok(text) = std::fs::read_to_string(work.join(file)) else {
            continue;
        };
        for (n, line) in text.lines().enumerate() {
            if line.contains(query) {
                hits.push(format!("{file}:{}: {}", n + 1, line.trim()));
                if hits.len() >= 300 {
                    return hits.join("\n") + "\n… (more matches)";
                }
            }
        }
    }
    if hits.is_empty() {
        "no matches".to_owned()
    } else {
        hits.join("\n")
    }
}

fn read_file(work: &Path, path: &str) -> String {
    let Some(p) = inside(work, path) else {
        return "path outside the repository".to_owned();
    };
    match std::fs::metadata(&p) {
        Ok(m) if m.len() > MAX_READ => {
            format!("file is {} bytes; read a smaller one or search it", m.len())
        }
        Ok(_) => std::fs::read_to_string(&p).unwrap_or_else(|_| "not a text file".to_owned()),
        Err(_) => "no such file".to_owned(),
    }
}

/// Runs the verification; returns (state, exit code, summary).
#[allow(clippy::too_many_arguments)]
pub fn run(
    client: &Client,
    log: &Sink<'_>,
    run_id: &str,
    work: &Path,
    store: &Store,
    commit: &Commit,
    base: Option<&str>,
    spec: &Value,
) -> Result<(String, Option<i64>, String)> {
    let contract_path = spec["contract"].as_str().unwrap_or(".hb/contract.md");
    let contract = match inside(work, contract_path).and_then(|p| std::fs::read_to_string(p).ok()) {
        Some(c) => c,
        None => {
            return Ok((
                "error".to_owned(),
                None,
                format!(
                    "no contract at `{contract_path}`; add one to describe how changes are verified"
                ),
            ));
        }
    };
    let diff = change(store, commit, base)?;
    log(&format!(
        "Verifying against {contract_path} with Hebra's verifier.\n"
    ));
    let mut messages = vec![json!({
        "role": "user",
        "content": format!(
            "<contract path=\"{contract_path}\">\n{contract}\n</contract>\n\n<change description=\"{}\">\n{diff}\n</change>\n\nVerify the change against the contract.",
            commit.description.replace('"', "'").lines().next().unwrap_or("")
        ),
    })];
    for turn in 0..max_turns() {
        let reply = client.post(
            &format!("/runner/runs/{run_id}/agent"),
            &json!({ "messages": messages }),
        )?;
        let content = reply["content"].clone();
        let blocks = content.as_array().cloned().unwrap_or_default();
        messages.push(json!({ "role": "assistant", "content": content }));
        let mut results = Vec::new();
        for block in &blocks {
            match block["type"].as_str() {
                Some("text") => {
                    if let Some(t) = block["text"].as_str() {
                        log(&format!("{t}\n"));
                    }
                }
                Some("tool_use") => {
                    let input = &block["input"];
                    let name = block["name"].as_str().unwrap_or_default();
                    if name == "verdict" {
                        let pass = input["verdict"] == "pass";
                        let summary = input["summary"].as_str().unwrap_or("").to_owned();
                        let mut report = format!(
                            "\nVerdict: {}\n{summary}\n",
                            if pass { "PASS" } else { "FAIL" }
                        );
                        for f in input["findings"].as_array().cloned().unwrap_or_default() {
                            let _ = writeln!(
                                report,
                                "- [{}] {}{}: {}",
                                f["severity"].as_str().unwrap_or("error"),
                                f["file"].as_str().unwrap_or(""),
                                f["line"]
                                    .as_i64()
                                    .map(|l| format!(":{l}"))
                                    .unwrap_or_default(),
                                f["message"].as_str().unwrap_or("")
                            );
                        }
                        log(&report);
                        log(&format!("({} credits)\n", reply["run_credits"]));
                        return Ok((
                            if pass { "success" } else { "failure" }.to_owned(),
                            None,
                            summary.chars().take(2000).collect(),
                        ));
                    }
                    let output = match name {
                        "read_file" => read_file(work, input["path"].as_str().unwrap_or("")),
                        "list_files" => list_files(work, input["path"].as_str().unwrap_or("")),
                        "search" => search(work, input["query"].as_str().unwrap_or("")),
                        "run" => {
                            let command = input["command"].as_str().unwrap_or("");
                            log(&format!("$ {command}\n"));
                            let out = tool_run(command, work, COMMAND_TIMEOUT)
                                .unwrap_or_else(|e| format!("could not run: {e:#}"));
                            log(&format!("{}\n", truncate(&out)));
                            out
                        }
                        other => format!("unknown tool `{other}`"),
                    };
                    results.push(json!({
                        "type": "tool_result",
                        "tool_use_id": block["id"],
                        "content": truncate(&output),
                    }));
                }
                _ => {}
            }
        }
        if results.is_empty() {
            messages.push(json!({ "role": "user", "content": "Call `verdict` to finish." }));
        } else {
            messages.push(json!({ "role": "user", "content": results }));
        }
        if turn + 1 == max_turns() {
            break;
        }
    }
    Err(anyhow!(
        "the verifier did not reach a verdict within {} turns",
        max_turns()
    ))
}
