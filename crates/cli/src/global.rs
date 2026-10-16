//! Commands that talk to a server without needing a local repository:
//! signing in, mandates, runners, organizations, agents, repositories.

use std::path::Path;

use anyhow::{Result, anyhow};
use hebra_core::Workspace;
use serde_json::json;

use crate::account;
use crate::cli::{AgentCommand, Command, MandateCommand, OrgCommand, RepoCommand, RunnerCommand};
use crate::client::Client;
use crate::commands::Output;
use crate::remote::split_url;

/// The server to use: given, `HB_SERVER`, or this repository's `origin`.
fn server(given: Option<String>, cwd: &Path) -> Result<String> {
    if let Some(s) = given.or_else(|| std::env::var("HB_SERVER").ok().filter(|s| !s.is_empty())) {
        return Ok(s.trim_end_matches('/').to_owned());
    }
    let url = Workspace::open(cwd)
        .ok()
        .and_then(|ws| ws.config_get("remote.origin.url").ok().flatten())
        .ok_or_else(|| anyhow!("name a server (e.g. https://hebra.dev), set HB_SERVER, or run inside a repository with an `origin`"))?;
    Ok(split_url(&url)?.0)
}

/// `owner/name` of this repository's `origin`, if any.
fn origin_repo(cwd: &Path) -> Option<String> {
    let ws = Workspace::open(cwd).ok()?;
    let url = ws.config_get("remote.origin.url").ok().flatten()?;
    split_url(&url).ok().map(|(_, repo)| repo)
}

fn signed_in(server: &str) -> Result<Client> {
    let token =
        account::credential(server).ok_or_else(|| anyhow!("run `hb login {server}` first"))?;
    Ok(Client::new(server, Some(token)))
}

/// `8h`, `30m`, `2d`, `90s` → seconds.
fn seconds(text: &str) -> Result<i64> {
    let (n, unit) = text.split_at(text.len().saturating_sub(1));
    let n: i64 = n
        .parse()
        .map_err(|_| anyhow!("invalid duration `{text}` (like 8h, 30m, 2d)"))?;
    Ok(match unit {
        "d" => n * 86_400,
        "h" => n * 3_600,
        "m" => n * 60,
        "s" => n,
        _ => return Err(anyhow!("invalid duration `{text}` (like 8h, 30m, 2d)")),
    })
}

/// Makes the token of this repository's `origin` (if it has one) available
/// to server-wide commands that talk to the same server.
fn adopt_origin_token(cwd: &Path) {
    let Ok(ws) = Workspace::open(cwd) else { return };
    let url = ws.config_get("remote.origin.url").ok().flatten();
    let token = ws.config_get("remote.origin.token").ok().flatten();
    if let (Some(url), Some(token)) = (url, token)
        && let Ok((base, _)) = split_url(&url)
    {
        account::use_remote_token(&base, token);
    }
}

/// Runs `command` if it is a server-wide one.
pub fn execute(
    command: Command,
    cwd: &Path,
    json_mode: bool,
) -> std::result::Result<Result<Output>, Box<Command>> {
    if !matches!(command, Command::Login { .. } | Command::Logout { .. }) {
        adopt_origin_token(cwd);
    }
    Ok(match command {
        Command::Login { server: s } => server(s, cwd).and_then(|s| account::login(&s, json_mode)),
        Command::Logout { server: s } => server(s, cwd).and_then(|s| account::logout(&s)),
        Command::Whoami { server: s } => server(s, cwd).and_then(|s| account::whoami(&s)),
        Command::Mandate(MandateCommand::Grant {
            agent,
            repos,
            operations,
            bookmarks,
            ttl,
            purpose,
            session,
            server: s,
        }) => (|| {
            let s = server(s, cwd)?;
            let repos = if repos.is_empty() {
                origin_repo(cwd).into_iter().collect()
            } else {
                repos
            };
            account::grant(
                &s,
                &account::Grant {
                    agent,
                    repos,
                    operations,
                    bookmarks,
                    ttl_secs: seconds(&ttl)?,
                    purpose,
                    session,
                },
            )
        })(),
        Command::Mandate(MandateCommand::List { server: s }) => {
            server(s, cwd).and_then(|s| account::list_mandates(&s))
        }
        Command::Mandate(MandateCommand::Revoke { id, server: s }) => {
            server(s, cwd).and_then(|s| account::revoke_mandate(&s, &id))
        }
        Command::Runner(RunnerCommand::Register {
            owner,
            name,
            labels,
            server: s,
        }) => server(s, cwd).and_then(|s| crate::runner::register(&s, &owner, &name, &labels)),
        Command::Runner(RunnerCommand::Start { name, once }) => crate::runner::start(&name, once),
        Command::Org(OrgCommand::Create {
            handle,
            name,
            server: s,
        }) => (|| {
            let s = server(s, cwd)?;
            let reply = signed_in(&s)?.post(
                "/orgs",
                &json!({ "handle": handle, "name": name.unwrap_or_default() }),
            )?;
            Ok(Output::new(
                reply,
                format!("Created organization {handle}; you own it.\n"),
            ))
        })(),
        Command::Org(OrgCommand::Add {
            org,
            person,
            role,
            server: s,
        }) => (|| {
            let s = server(s, cwd)?;
            let reply = signed_in(&s)?.send(
                "PUT",
                &format!("/accounts/{org}/members/{person}"),
                &json!({ "role": role }),
            )?;
            Ok(Output::new(
                reply,
                format!("{person} is {role} of {org}.\n"),
            ))
        })(),
        Command::Agent(AgentCommand::Create {
            handle,
            owner,
            server: s,
        }) => (|| {
            let s = server(s, cwd)?;
            let client = signed_in(&s)?;
            let owner = match owner {
                Some(o) => o,
                None => client.get("/me")?["account"]["handle"]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| anyhow!("could not tell who you are"))?,
            };
            let reply = client.post(
                &format!("/accounts/{owner}/agents"),
                &json!({ "handle": handle }),
            )?;
            let mandate = reply["mandate"].as_str().unwrap_or("").to_owned();
            Ok(Output::new(
                reply,
                format!(
                    "{mandate}\n\nCreated agent {handle} for {owner}. Give it this mandate as HB_TOKEN; it is not shown again.\n"
                ),
            ))
        })(),
        Command::Repo(RepoCommand::Create {
            full_name,
            public,
            description,
            server: s,
        }) => (|| {
            let s = server(s, cwd)?;
            let (owner, name) = full_name
                .split_once('/')
                .ok_or_else(|| anyhow!("use owner/name"))?;
            let reply = signed_in(&s)?.post(
                "/repos",
                &json!({ "owner": owner, "name": name, "public": public, "description": description.unwrap_or_default() }),
            )?;
            Ok(Output::new(
                reply,
                format!(
                    "Created {full_name}. Add it here with `hb remote add origin {s}/{full_name}`.\n"
                ),
            ))
        })(),
        other => return Err(Box::new(other)),
    })
}
