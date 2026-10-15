//! Working with a server's repository: tasks, checks, and review comments
//! and approvals.

use std::fmt::Write as _;
use std::time::Duration;

use anyhow::Result;
use hebra_core::Workspace;
use serde_json::{Value, json};

use crate::commands::Output;
use crate::remote::Remote;

fn remote(ws: &Workspace, name: &str) -> Result<Remote> {
    Remote::load(ws, name, Duration::from_secs(60))
}

// ------------------------------------------------------------------ tasks

fn task_line(t: &Value) -> String {
    format!(
        "#{:<4} {:<9} {}{}{}",
        t["id"].as_i64().unwrap_or_default(),
        if t["state"] == "open" && t["ready"] == false {
            "waiting"
        } else {
            t["state"].as_str().unwrap_or("?")
        },
        t["title"].as_str().unwrap_or(""),
        t["claimed_by"]
            .as_str()
            .map(|c| format!("  [{c}]"))
            .unwrap_or_default(),
        t["labels"]
            .as_array()
            .filter(|l| !l.is_empty())
            .map(|l| format!(
                "  ({})",
                l.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
            .unwrap_or_default(),
    )
}

fn task_text(t: &Value) -> String {
    let mut text = format!("{}\n", task_line(t));
    if let Some(body) = t["body"].as_str().filter(|b| !b.is_empty()) {
        let _ = writeln!(text, "\n{body}");
    }
    if let Some(acceptance) = t["acceptance"].as_array().filter(|a| !a.is_empty()) {
        text.push_str("\nDone when:\n");
        for a in acceptance {
            let _ = writeln!(text, "  - {}", a.as_str().unwrap_or(""));
        }
    }
    if let Some(deps) = t["depends_on"].as_array().filter(|d| !d.is_empty()) {
        let _ = writeln!(
            text,
            "\nAfter: {}",
            deps.iter()
                .map(|d| format!("#{d}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if let Some(lease) = t["lease_until"].as_str() {
        let _ = writeln!(text, "Claimed until {lease}");
    }
    text
}

pub fn task_list(ws: &Workspace, name: &str, state: &str) -> Result<Output> {
    let data = remote(ws, name)?.get_json(&format!("/tasks?state={state}"))?;
    let mut text = String::new();
    for t in data["tasks"].as_array().cloned().unwrap_or_default() {
        let _ = writeln!(text, "{}", task_line(&t));
    }
    if text.is_empty() {
        let _ = writeln!(text, "No {state} tasks.");
    }
    Ok(Output::new(data, text))
}

pub struct NewTask {
    pub title: String,
    pub body: Option<String>,
    pub acceptance: Vec<String>,
    pub labels: Vec<String>,
    pub parent: Option<i64>,
    pub after: Vec<i64>,
}

pub fn task_create(ws: &Workspace, name: &str, t: NewTask) -> Result<Output> {
    let task = remote(ws, name)?.post_json(
        "/tasks",
        &json!({
            "title": t.title,
            "body": t.body.unwrap_or_default(),
            "acceptance": t.acceptance,
            "labels": t.labels,
            "parent": t.parent,
            "depends_on": t.after,
        }),
    )?;
    let text = format!("Created task #{}.\n", task["id"]);
    Ok(Output::new(task, text))
}

pub fn task_show(ws: &Workspace, name: &str, id: i64) -> Result<Output> {
    let task = remote(ws, name)?.get_json(&format!("/tasks/{id}"))?;
    let text = task_text(&task);
    Ok(Output::new(task, text))
}

/// `claim`, `release`, `close` (done/cancelled), `reopen`.
pub fn task_act(ws: &Workspace, name: &str, id: i64, action: &str, body: Value) -> Result<Output> {
    let task = remote(ws, name)?.post_json(&format!("/tasks/{id}/{action}"), &body)?;
    let text = match action {
        "claim" => format!(
            "Claimed #{id} until {}. Commit with HB_TASK=#{id} so the review closes it.\n",
            task["lease_until"].as_str().unwrap_or("?")
        ),
        "release" => format!("Released #{id}.\n"),
        "reopen" => format!("Reopened #{id}.\n"),
        _ => format!(
            "Task #{id} is {}.\n",
            task["state"].as_str().unwrap_or("closed")
        ),
    };
    Ok(Output::new(task, text))
}

// ----------------------------------------------------------------- checks

fn check_line(c: &Value) -> String {
    format!(
        "{:<9} {:<20} {}  {}{}",
        c["state"].as_str().unwrap_or("?"),
        c["name"].as_str().unwrap_or("?"),
        c["id"].as_str().unwrap_or(""),
        c["summary"].as_str().unwrap_or(""),
        if c["credits"].as_i64().unwrap_or(0) > 0 {
            format!(" ({} credits)", c["credits"])
        } else {
            String::new()
        },
    )
}

pub fn checks_list(ws: &Workspace, name: &str, rev: Option<&str>) -> Result<Output> {
    let path = match rev {
        Some(r) => format!("/checks?commit={r}"),
        None => "/checks".to_owned(),
    };
    let data = remote(ws, name)?.get_json(&path)?;
    let mut text = String::new();
    for c in data["checks"].as_array().cloned().unwrap_or_default() {
        let _ = writeln!(text, "{}", check_line(&c));
    }
    if text.is_empty() {
        text.push_str("No checks. Declare them in .hb/checks.toml.\n");
    }
    Ok(Output::new(data, text))
}

pub fn checks_log(ws: &Workspace, name: &str, id: &str) -> Result<Output> {
    let data = remote(ws, name)?.get_json(&format!("/checks/{id}/log"))?;
    let text: String = data["chunks"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|c| c["text"].as_str().map(str::to_owned))
        .collect();
    Ok(Output::new(data, text))
}

pub fn checks_act(
    ws: &Workspace,
    name: &str,
    action: &str,
    rev: &str,
    check: Option<String>,
) -> Result<Output> {
    let r = remote(ws, name)?;
    let info = r.get_json("")?;
    let commit = info["bookmarks"][rev]
        .as_str()
        .map_or_else(|| rev.to_owned(), str::to_owned);
    let reply = r.post_json(
        &format!("/checks/{action}"),
        &json!({ "commit": commit, "name": check }),
    )?;
    let text = match action {
        "approve" => format!("Approved {} waiting run(s).\n", reply["approved"]),
        _ => format!(
            "Queued {} run(s).\n",
            reply["queued"].as_array().map_or(0, Vec::len)
        ),
    };
    Ok(Output::new(reply, text))
}

// ---------------------------------------------------------------- reviews

pub fn review_show(ws: &Workspace, name: &str, id: i64) -> Result<Output> {
    let review = remote(ws, name)?.get_json(&format!("/reviews/{id}"))?;
    let mut text = format!(
        "#{} {}  {} -> {}  ({})\n",
        review["id"],
        review["state"].as_str().unwrap_or("?"),
        review["source"].as_str().unwrap_or("?"),
        review["target"].as_str().unwrap_or("?"),
        review["author"].as_str().unwrap_or("?"),
    );
    let _ = writeln!(text, "{}", review["title"].as_str().unwrap_or(""));
    if let Some(body) = review["body"].as_str().filter(|b| !b.is_empty()) {
        let _ = writeln!(text, "\n{body}");
    }
    let req = &review["requirements"];
    if req.is_object() {
        let _ = writeln!(
            text,
            "\nApprovals: {} of {}{}",
            req["approvals"],
            req["approvals_needed"],
            if req["ready"] == true {
                "  — ready to merge"
            } else {
                ""
            }
        );
        for c in req["checks"].as_array().cloned().unwrap_or_default() {
            let _ = writeln!(text, "  {}", check_line(&c));
        }
    }
    for c in review["comments"].as_array().cloned().unwrap_or_default() {
        let place = match (c["path"].as_str(), c["line"].as_i64()) {
            (Some(p), Some(l)) => format!(" on {p}:{l}"),
            (Some(p), None) => format!(" on {p}"),
            _ => String::new(),
        };
        let _ = writeln!(
            text,
            "\n{}{place}{}:\n  {}",
            c["author"].as_str().unwrap_or("?"),
            if c["resolved"] == true {
                " (resolved)"
            } else {
                ""
            },
            c["body"].as_str().unwrap_or("").replace('\n', "\n  ")
        );
    }
    Ok(Output::new(review, text))
}

pub fn review_comment(
    ws: &Workspace,
    name: &str,
    id: i64,
    body: &str,
    path: Option<String>,
    line: Option<i64>,
) -> Result<Output> {
    let reply = remote(ws, name)?.post_json(
        &format!("/reviews/{id}/comments"),
        &json!({ "body": body, "path": path, "line": line, "side": line.map(|_| "new") }),
    )?;
    Ok(Output::new(reply, format!("Commented on review #{id}.\n")))
}

pub fn review_approve(
    ws: &Workspace,
    name: &str,
    id: i64,
    changes: bool,
    note: Option<String>,
) -> Result<Output> {
    let state = if changes {
        "changes_requested"
    } else {
        "approved"
    };
    let review = remote(ws, name)?.post_json(
        &format!("/reviews/{id}/approve"),
        &json!({ "state": state, "body": note }),
    )?;
    let text = if changes {
        format!("Requested changes on review #{id}.\n")
    } else {
        format!(
            "Approved review #{id}{}.\n",
            if review["requirements"]["ready"] == true {
                "; it can be merged"
            } else {
                ""
            }
        )
    };
    Ok(Output::new(review, text))
}
