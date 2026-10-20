//! Talking to a Hebra server: remotes, push, pull and presence.

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use hebra_core::model::{Actor, Provenance};
use hebra_core::{ObjectId, Workspace, transport};
use serde_json::{Value, json};

use crate::commands::{Output, own_changes};

/// A remote: server base URL, repository name and token.
pub struct Remote {
    pub name: String,
    pub base: String,
    pub repo: String,
    token: Option<String>,
    agent: ureq::Agent,
}

/// `https://host[:port]/owner/repo` -> (`https://host[:port]`, `owner/repo`).
pub(crate) fn split_url(url: &str) -> Result<(String, String)> {
    const SHAPE: &str = "remote URL must look like https://host/<owner>/<repo>";
    let trimmed = url.trim().trim_end_matches('/');
    let scheme_end = trimmed.find("://").ok_or_else(|| anyhow!(SHAPE))?;
    let scheme = &trimmed[..scheme_end];
    if scheme != "http" && scheme != "https" {
        bail!(SHAPE);
    }
    let rest = &trimmed[scheme_end + 3..];
    let (host, path) = rest.split_once('/').ok_or_else(|| anyhow!(SHAPE))?;
    let parts: Vec<&str> = path.split('/').collect();
    if host.is_empty() || parts.len() != 2 || parts.iter().any(|p| p.is_empty()) {
        bail!(SHAPE);
    }
    Ok((format!("{scheme}://{host}"), path.to_owned()))
}

fn agent(timeout: Duration) -> ureq::Agent {
    crate::client::http_agent(Some(timeout))
}

impl Remote {
    pub fn load(ws: &Workspace, name: &str, timeout: Duration) -> Result<Self> {
        let url = ws
            .config_get(&format!("remote.{name}.url"))?
            .ok_or_else(|| {
                anyhow!("no remote named `{name}` (add one with `hb remote add {name} <url>`)")
            })?;
        let (base, repo) = split_url(&url)?;
        let token = std::env::var("HB_TOKEN")
            .ok()
            .filter(|t| !t.is_empty())
            .or(ws.config_get(&format!("remote.{name}.token"))?)
            .or_else(|| crate::account::stored_credential(&base));
        Ok(Self {
            name: name.to_owned(),
            base,
            repo,
            token,
            agent: agent(timeout),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}/api/v1/repos/{}{path}", self.base, self.repo)
    }

    fn auth(&self) -> Option<String> {
        self.token.as_ref().map(|t| format!("Bearer {t}"))
    }

    fn read(&self, mut resp: ureq::http::Response<ureq::Body>) -> Result<Vec<u8>> {
        let status = resp.status();
        let body = resp
            .body_mut()
            .with_config()
            .limit(2 * 1024 * 1024 * 1024)
            .read_to_vec()
            .context("reading the server response")?;
        if status.is_success() {
            return Ok(body);
        }
        let message = serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
            .unwrap_or_else(|| format!("HTTP {status}"));
        bail!("{} ({}): {message}", self.name, status.as_u16())
    }

    /// URL of the repository's live event stream.
    pub fn stream_url(&self) -> String {
        self.url("/stream")
    }

    pub fn auth_header(&self) -> Option<String> {
        self.auth()
    }

    pub fn get_json(&self, path: &str) -> Result<Value> {
        let mut req = self.agent.get(self.url(path));
        if let Some(a) = self.auth() {
            req = req.header("Authorization", a);
        }
        let resp = req
            .call()
            .with_context(|| format!("connecting to {}", self.base))?;
        Ok(serde_json::from_slice(&self.read(resp)?)?)
    }

    fn post(&self, path: &str, content_type: &str, body: &[u8]) -> Result<Vec<u8>> {
        let mut req = self
            .agent
            .post(self.url(path))
            .header("Content-Type", content_type);
        if let Some(a) = self.auth() {
            req = req.header("Authorization", a);
        }
        let resp = req
            .send(body)
            .with_context(|| format!("connecting to {}", self.base))?;
        self.read(resp)
    }

    pub fn post_json(&self, path: &str, body: &Value) -> Result<Value> {
        let bytes = self.post(path, "application/json", body.to_string().as_bytes())?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}

// ------------------------------------------------------------ remote add

pub fn add(ws: &Workspace, name: &str, url: &str, token: Option<String>) -> Result<Output> {
    split_url(url)?;
    ws.config_set(&format!("remote.{name}.url"), url)?;
    if let Some(t) = &token {
        ws.config_set(&format!("remote.{name}.token"), t)?;
    }
    Ok(Output::new(
        json!({ "remote": name, "url": url, "token": token.is_some() }),
        format!("Added remote {name} -> {url}\n"),
    ))
}

pub fn list(ws: &Workspace) -> Result<Output> {
    let config = ws.config_list()?;
    let mut remotes = Vec::new();
    let mut text = String::new();
    for (key, url) in &config {
        if let Some(name) = key
            .strip_prefix("remote.")
            .and_then(|k| k.strip_suffix(".url"))
        {
            let has_token = config.contains_key(&format!("remote.{name}.token"));
            let _ = writeln!(
                text,
                "{name}\t{url}{}",
                if has_token { "\t(token set)" } else { "" }
            );
            remotes.push(json!({ "name": name, "url": url, "token": has_token }));
        }
    }
    if remotes.is_empty() {
        text.push_str("No remotes.\n");
    }
    Ok(Output::new(json!({ "remotes": remotes }), text))
}

pub fn remove(ws: &Workspace, name: &str) -> Result<Output> {
    let removed = ws.config_unset(&format!("remote.{name}.url"))?;
    ws.config_unset(&format!("remote.{name}.token"))?;
    if !removed {
        bail!("no remote named `{name}`");
    }
    Ok(Output::new(
        json!({ "removed": name }),
        format!("Removed remote {name}\n"),
    ))
}

// ------------------------------------------------------------------ push

struct PushPlan {
    bookmark: String,
    local: ObjectId,
    tracked: Option<ObjectId>,
    objects: Vec<(ObjectId, Vec<u8>)>,
}

pub fn push(
    ws: &mut Workspace,
    actor: &Actor,
    prov: &Provenance,
    remote_name: &str,
    bookmark: Option<String>,
    revision: Option<String>,
) -> Result<Output> {
    let remote = Remote::load(ws, remote_name, Duration::from_secs(600))?;
    let store = ws.store.clone();
    let remote_for_tx = remote_name.to_owned();

    // 1. Decide what to send (and optionally point the bookmark at a revision).
    let plans: Vec<PushPlan> = ws.run(actor, prov, "push", |tx| {
        if let (Some(name), Some(rev)) = (&bookmark, &revision) {
            let id = tx.resolve(rev)?;
            tx.view.bookmarks.insert(name.clone(), id);
        }
        let tracking = tx
            .view
            .remotes
            .get(&remote_for_tx)
            .cloned()
            .unwrap_or_default();
        let haves: HashSet<ObjectId> = tracking.values().copied().collect();
        let names: Vec<String> = match &bookmark {
            Some(b) => vec![b.clone()],
            None => tx
                .view
                .bookmarks
                .iter()
                .filter(|(n, id)| tracking.get(*n) != Some(id))
                .map(|(n, _)| n.clone())
                .collect(),
        };
        let mut plans = Vec::new();
        for name in names {
            let local = *tx
                .view
                .bookmarks
                .get(&name)
                .ok_or_else(|| hebra_core::Error::RevisionNotFound(name.clone()))?;
            let ids = transport::closure(&store, &[local], &haves)?;
            plans.push(PushPlan {
                bookmark: name.clone(),
                local,
                tracked: tracking.get(&name).copied(),
                objects: transport::collect(&store, &ids)?,
            });
        }
        Ok(plans)
    })?;

    if plans.is_empty() {
        return Ok(Output::new(
            json!({ "pushed": [] }),
            "Nothing to push: every bookmark matches the remote. Use `hb push --bookmark <name> -r <rev>` to publish a change.\n",
        ));
    }

    // 2. Send objects, then move each bookmark with compare-and-swap.
    let mut pushed = Vec::new();
    let mut text = String::new();
    for plan in &plans {
        if !plan.objects.is_empty() {
            let pack = transport::encode(&plan.objects);
            remote.post("/objects", "application/x-hebra-pack", &pack)?;
        }
        if plan.tracked == Some(plan.local) {
            continue;
        }
        let body = json!({
            "name": plan.bookmark,
            "old": plan.tracked.map(|o| o.hex()),
            "new": plan.local.hex(),
        });
        remote.post_json("/bookmarks", &body).map_err(|e| {
            let msg = e.to_string();
            if msg.contains("(409)") {
                anyhow!("{msg}\nRun `hb pull {remote_name}` and merge `{}@{remote_name}` before pushing again.", plan.bookmark)
            } else {
                e
            }
        })?;
        let _ = writeln!(
            text,
            "{} -> {} on {remote_name} ({} object(s) sent)",
            plan.bookmark,
            plan.local.short(),
            plan.objects.len()
        );
        pushed.push(json!({ "bookmark": plan.bookmark, "commit": plan.local.hex(), "objects": plan.objects.len() }));
    }

    // 3. Remember what the remote now has.
    let remote_for_tx = remote_name.to_owned();
    ws.run(actor, prov, "push", |tx| {
        let tracking = tx.view.remotes.entry(remote_for_tx).or_default();
        for plan in &plans {
            tracking.insert(plan.bookmark.clone(), plan.local);
        }
        Ok(())
    })?;
    Ok(Output::new(
        json!({ "remote": remote_name, "pushed": pushed }),
        text,
    ))
}

// ------------------------------------------------------------------ pull

pub fn pull(
    ws: &mut Workspace,
    actor: &Actor,
    prov: &Provenance,
    remote_name: &str,
) -> Result<Output> {
    let remote = Remote::load(ws, remote_name, Duration::from_secs(600))?;
    let info = remote.get_json("")?;
    let mut server: BTreeMap<String, ObjectId> = BTreeMap::new();
    if let Some(map) = info["bookmarks"].as_object() {
        for (name, target) in map {
            server.insert(
                name.clone(),
                ObjectId::parse(target.as_str().unwrap_or_default())?,
            );
        }
    }
    let store = ws.store.clone();
    let tracking: BTreeMap<String, ObjectId> = ws.run(actor, prov, "pull", |tx| {
        Ok(tx
            .view
            .remotes
            .get(remote_name)
            .cloned()
            .unwrap_or_default())
    })?;

    let want: Vec<String> = server
        .values()
        .filter(|id| !store.has(id))
        .map(ObjectId::hex)
        .collect();
    let mut received = 0;
    if !want.is_empty() {
        let have: Vec<String> = tracking
            .values()
            .filter(|id| store.has(id))
            .map(ObjectId::hex)
            .collect();
        let body = json!({ "want": want, "have": have });
        let pack = remote.post("/pack", "application/json", body.to_string().as_bytes())?;
        let objects = transport::decode(&pack)?;
        received = store.batch(|| transport::store_all(&store, &objects))?;
    }

    let remote_for_tx = remote_name.to_owned();
    let (updated, diverged) = ws.run(actor, prov, "pull", |tx| {
        let old_tracking = tx
            .view
            .remotes
            .get(&remote_for_tx)
            .cloned()
            .unwrap_or_default();
        let mut updated = Vec::new();
        let mut diverged = Vec::new();
        for (name, new) in &server {
            match tx.view.bookmarks.get(name).copied() {
                None => {
                    tx.view.bookmarks.insert(name.clone(), *new);
                    updated.push(name.clone());
                }
                Some(local) if local == *new => {}
                Some(local) => {
                    let unchanged_locally = old_tracking.get(name) == Some(&local);
                    if unchanged_locally || tx.is_ancestor(local, *new)? {
                        tx.view.bookmarks.insert(name.clone(), *new);
                        updated.push(name.clone());
                    } else if !tx.is_ancestor(*new, local)? {
                        diverged.push(name.clone());
                    }
                }
            }
        }
        tx.view
            .remotes
            .insert(remote_for_tx.clone(), server.clone());
        Ok((updated, diverged))
    })?;

    let mut text = format!(
        "Pulled from {remote_name}: {} bookmark(s), {received} new object(s).\n",
        server.len()
    );
    for name in &updated {
        let _ = writeln!(text, "  {name} -> {}", server[name].short());
    }
    for name in &diverged {
        let _ = writeln!(
            text,
            "  {name} has diverged: merge it with `hb merge {name}@{remote_name}`"
        );
    }
    Ok(Output::new(
        json!({ "remote": remote_name, "bookmarks": server.keys().collect::<Vec<_>>(), "updated": updated, "diverged": diverged, "objects": received }),
        text,
    ))
}

// -------------------------------------------------------------- presence

fn client_id(ws: &Workspace) -> Result<String> {
    if let Some(id) = ws.config_get("client.id")? {
        return Ok(id);
    }
    let mut bytes = [0u8; 8];
    rand::fill(&mut bytes);
    let id = hex::encode(bytes);
    ws.config_set("client.id", &id)?;
    Ok(id)
}

fn machine() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown machine".to_owned())
}

/// Builds this machine's presence: every workspace with its changed paths
/// and claims.
fn local_presence(ws: &mut Workspace, actor: &Actor, prov: &Provenance) -> Result<Value> {
    ws.snapshot_others(prov)?;
    let workspaces = ws.run(actor, prov, "presence", |tx| {
        let claims = tx.claims()?;
        let entries: Vec<(String, hebra_core::model::WorkspaceInfo)> = tx
            .view
            .workspaces
            .iter()
            .map(|(n, w)| (n.clone(), w.clone()))
            .collect();
        let mut out = Vec::new();
        for (name, info) in entries {
            let changes: Vec<String> = own_changes(tx, &info.wc)?
                .into_iter()
                .map(|(p, _)| p)
                .collect();
            let ws_claims: Vec<&String> = claims
                .iter()
                .filter(|c| c.workspace == name)
                .map(|c| &c.path)
                .collect();
            out.push(json!({
                "name": name,
                "actor": info.actor.as_ref().map_or_else(|| actor.display(), Actor::display),
                "task": info.task,
                "changes": changes,
                "claims": ws_claims,
            }));
        }
        Ok(out)
    })?;
    Ok(json!({ "machine": machine(), "actor": actor.display(), "workspaces": workspaces }))
}

/// Publishes our presence and returns everyone else's.
pub fn exchange(
    ws: &mut Workspace,
    actor: &Actor,
    prov: &Provenance,
    remote_name: &str,
    timeout: Duration,
) -> Result<Vec<Value>> {
    let remote = Remote::load(ws, remote_name, timeout)?;
    let me = client_id(ws)?;
    let mut payload = local_presence(ws, actor, prov)?;
    payload["client"] = json!(me);
    remote.post_json("/presence", &payload)?;
    let all = remote.get_json("/presence")?;
    Ok(all["presence"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p["client"].as_str() != Some(me.as_str()))
        .collect())
}

pub fn presence(
    ws: &mut Workspace,
    actor: &Actor,
    prov: &Provenance,
    remote_name: &str,
) -> Result<Output> {
    let others = exchange(ws, actor, prov, remote_name, Duration::from_secs(10))?;
    let mut text = String::new();
    if others.is_empty() {
        let _ = writeln!(text, "Nobody else is working on {remote_name} right now.");
    }
    for p in &others {
        let _ = writeln!(
            text,
            "{} on {}",
            p["actor"].as_str().unwrap_or("?"),
            p["machine"].as_str().unwrap_or("?")
        );
        for w in p["workspaces"].as_array().into_iter().flatten() {
            let changes = w["changes"].as_array().map_or(0, Vec::len);
            let _ = writeln!(
                text,
                "  {}  {}  {} file(s)",
                w["name"].as_str().unwrap_or("?"),
                w["actor"].as_str().unwrap_or("?"),
                changes
            );
            for c in w["changes"].as_array().into_iter().flatten() {
                let _ = writeln!(text, "      {}", c.as_str().unwrap_or("?"));
            }
        }
    }
    Ok(Output::new(
        json!({ "remote": remote_name, "presence": others }),
        text,
    ))
}

/// Warnings for files we changed that someone on another machine is also
/// changing or has claimed.
pub fn remote_warnings(others: &[Value], changed: &[String]) -> Vec<Value> {
    let mut out = Vec::new();
    for p in others {
        for w in p["workspaces"].as_array().into_iter().flatten() {
            let theirs: HashSet<&str> = w["changes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let claims: Vec<&str> = w["claims"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            for path in changed {
                let claimed = claims.iter().any(|c| {
                    path == c || path.strip_prefix(*c).is_some_and(|r| r.starts_with('/'))
                });
                if theirs.contains(path.as_str()) || claimed {
                    out.push(json!({
                        "path": path,
                        "machine": p["machine"],
                        "workspace": w["name"],
                        "actor": w["actor"],
                        "claimed": claimed,
                    }));
                }
            }
        }
    }
    out
}
