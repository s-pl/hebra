//! Collaboration on a server: reviews, deleting remote bookmarks, and
//! `hb watch` (live presence and events).

use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Write as _};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use hebra_core::Workspace;
use hebra_core::model::{Actor, Provenance};
use serde_json::{Value, json};

use crate::commands::Output;
use crate::remote::{Remote, exchange};

fn review_line(r: &Value) -> String {
    format!(
        "#{:<4} {:<7} {} -> {}  {}  ({})",
        r["id"].as_i64().unwrap_or_default(),
        r["state"].as_str().unwrap_or("?"),
        r["source"].as_str().unwrap_or("?"),
        r["target"].as_str().unwrap_or("?"),
        r["title"].as_str().unwrap_or(""),
        r["author"].as_str().unwrap_or("?"),
    )
}

pub fn review_open(
    ws: &Workspace,
    remote: &str,
    source: &str,
    target: &str,
    title: &str,
    body: Option<String>,
) -> Result<Output> {
    let r = Remote::load(ws, remote, Duration::from_secs(60))?;
    let review = r.post_json(
        "/reviews",
        &json!({ "source": source, "target": target, "title": title, "body": body.unwrap_or_default() }),
    )?;
    let text = format!(
        "Review #{} open: {source} -> {target}\nA person can merge it from the hub or with `hb review merge {}`.\n",
        review["id"], review["id"]
    );
    Ok(Output::new(review, text))
}

pub fn review_list(ws: &Workspace, remote: &str, all: bool) -> Result<Output> {
    let r = Remote::load(ws, remote, Duration::from_secs(60))?;
    let data = r.get_json(if all {
        "/reviews?state=all"
    } else {
        "/reviews"
    })?;
    let mut text = String::new();
    let reviews = data["reviews"].as_array().cloned().unwrap_or_default();
    if reviews.is_empty() {
        text.push_str("No open reviews.\n");
    }
    for review in &reviews {
        let _ = writeln!(text, "{}", review_line(review));
    }
    Ok(Output::new(data, text))
}

pub fn review_act(ws: &Workspace, remote: &str, id: i64, action: &str) -> Result<Output> {
    let r = Remote::load(ws, remote, Duration::from_secs(120))?;
    let review = r.post_json(&format!("/reviews/{id}/{action}"), &json!({}))?;
    let text = match action {
        "merge" => format!(
            "Merged review #{id}: {} -> {} ({})\nRun `hb pull` to bring it here.\n",
            review["source"].as_str().unwrap_or("?"),
            review["target"].as_str().unwrap_or("?"),
            review["merged_commit"]
                .as_str()
                .map_or("?", |c| c.get(..12).unwrap_or(c)),
        ),
        _ => format!("Closed review #{id}.\n"),
    };
    Ok(Output::new(review, text))
}

pub fn delete_bookmark(
    ws: &mut Workspace,
    actor: &Actor,
    prov: &Provenance,
    remote: &str,
    name: &str,
) -> Result<Output> {
    let r = Remote::load(ws, remote, Duration::from_secs(60))?;
    let info = r.get_json("")?;
    let Some(current) = info["bookmarks"][name].as_str() else {
        bail!("`{name}` does not exist on {remote}");
    };
    r.post_json(
        "/bookmarks",
        &json!({ "name": name, "old": current, "new": null }),
    )?;
    let (remote_owned, name_owned) = (remote.to_owned(), name.to_owned());
    ws.run(actor, prov, "push --delete", |tx| {
        if let Some(tracking) = tx.view.remotes.get_mut(&remote_owned) {
            tracking.remove(&name_owned);
        }
        Ok(())
    })?;
    Ok(Output::new(
        json!({ "deleted": name, "remote": remote }),
        format!("Deleted {name} on {remote}\n"),
    ))
}

fn describe_event(kind: &str, data: &Value) -> Option<String> {
    let actor = data["actor"].as_str().unwrap_or("someone");
    match kind {
        "bookmark" => Some(match (data["old"].is_null(), data["new"].is_null()) {
            (_, true) => format!("{actor} deleted {}", data["name"].as_str().unwrap_or("?")),
            (true, false) => format!("{actor} created {}", data["name"].as_str().unwrap_or("?")),
            _ => format!("{actor} moved {}", data["name"].as_str().unwrap_or("?")),
        }),
        "review" => Some(format!(
            "review #{} is {}",
            data["id"].as_i64().unwrap_or_default(),
            data["state"].as_str().unwrap_or("?")
        )),
        "presence" => Some(format!("{actor} updated what they are working on")),
        _ => None,
    }
}

/// Keeps presence up to date and prints what others do, as it happens.
pub fn watch(
    ws: &mut Workspace,
    actor: &Actor,
    prov: &Provenance,
    remote: &str,
    interval: u64,
    json_out: bool,
) -> Result<Output> {
    let r = Remote::load(ws, remote, Duration::from_secs(30))?;
    let url = r.stream_url();
    let auth = r.auth_header();
    let me = actor.display();

    // Live events on a background thread.
    std::thread::spawn(move || {
        let agent = crate::client::http_agent(None);
        loop {
            let mut req = agent.get(&url);
            if let Some(a) = &auth {
                req = req.header("Authorization", a);
            }
            if let Ok(mut resp) = req.call() {
                let reader = BufReader::new(resp.body_mut().as_reader());
                let mut kind = String::new();
                for line in reader.lines().map_while(Result::ok) {
                    if let Some(k) = line.strip_prefix("event: ") {
                        kind = k.to_owned();
                    } else if let Some(d) = line.strip_prefix("data: ") {
                        let data: Value = serde_json::from_str(d).unwrap_or_default();
                        if data["actor"].as_str() == Some(me.as_str()) {
                            continue;
                        }
                        if json_out {
                            println!("{}", json!({ "event": kind, "data": data }));
                        } else if let Some(text) = describe_event(&kind, &data) {
                            println!("  · {text}");
                        }
                        let _ = std::io::stdout().flush();
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(3));
        }
    });

    if !json_out {
        println!(
            "Watching {remote}: sharing your presence every {interval}s and showing others' activity. Ctrl+C to stop."
        );
    }
    let mut last = String::new();
    let mut last_sent = Instant::now() - Duration::from_secs(3600);
    loop {
        match exchange(ws, actor, prov, remote, Duration::from_secs(10)) {
            Ok(others) => {
                let summary = serde_json::to_string(&others).unwrap_or_default();
                if summary != last || last_sent.elapsed() > Duration::from_secs(60) {
                    if summary != last && !json_out {
                        for p in &others {
                            for w in p["workspaces"].as_array().into_iter().flatten() {
                                let files = w["changes"].as_array().map_or(0, Vec::len);
                                println!(
                                    "  ● {} on {}: {} file(s){}",
                                    w["actor"].as_str().unwrap_or("?"),
                                    p["machine"].as_str().unwrap_or("?"),
                                    files,
                                    w["task"]
                                        .as_str()
                                        .map(|t| format!(" · {t}"))
                                        .unwrap_or_default()
                                );
                            }
                        }
                    }
                    last = summary;
                    last_sent = Instant::now();
                }
            }
            Err(e) => eprintln!("  (presence: {e})"),
        }
        std::thread::sleep(Duration::from_secs(interval.max(2)));
    }
}
