//! Signing in and credentials: `hb login` (device flow), `hb logout`,
//! `hb whoami`, and mandates (`hb mandate grant | list | revoke`).
//!
//! Mandates from `hb login` are kept per server in
//! `<config dir>/hebra/credentials.json` (only readable by you on Unix).
//! `HB_TOKEN` and a remote's own token take precedence.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use hebra_mandate::{Delegation, Restrictions, op};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::client::Client;
use crate::commands::Output;

#[derive(Debug, Default, Serialize, Deserialize)]
struct Credentials {
    #[serde(default)]
    servers: BTreeMap<String, Stored>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Stored {
    mandate: String,
    handle: String,
}

/// Where `hb` keeps its own settings (`HB_CONFIG_DIR` overrides).
fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("HB_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    base.map(|b| b.join("hebra"))
}

fn credentials_path() -> Result<PathBuf> {
    config_dir()
        .map(|d| d.join("credentials.json"))
        .ok_or_else(|| anyhow!("cannot find a configuration directory; set HB_CONFIG_DIR"))
}

fn load() -> Credentials {
    credentials_path()
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save(creds: &Credentials) -> Result<()> {
    let path = credentials_path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(creds)?;
    let tmp = path.with_extension("tmp");
    {
        use std::io::Write as _;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options
            .open(&tmp)
            .with_context(|| format!("writing {}", tmp.display()))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

fn normalize(server: &str) -> String {
    server.trim().trim_end_matches('/').to_owned()
}

/// The mandate `hb login` stored for `server`, if any.
pub fn stored_credential(server: &str) -> Option<String> {
    load()
        .servers
        .get(&normalize(server))
        .map(|s| s.mandate.clone())
}

/// The token of the current repository's remote, for its server. Set once
/// per invocation by server-wide commands run inside a repository.
static REMOTE_TOKEN: std::sync::OnceLock<(String, String)> = std::sync::OnceLock::new();

/// Lets server-wide commands use the token of this repository's remote
/// (`hb remote add … --token`) when they talk to the same server.
pub fn use_remote_token(server: &str, token: String) {
    let _ = REMOTE_TOKEN.set((normalize(server), token));
}

/// The credential for a server: HB_TOKEN, then the current repository's
/// remote token for that server, then what `hb login` stored.
pub fn credential(server: &str) -> Option<String> {
    std::env::var("HB_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
        .or_else(|| {
            REMOTE_TOKEN
                .get()
                .filter(|(s, _)| *s == normalize(server))
                .map(|(_, t)| t.clone())
        })
        .or_else(|| stored_credential(server))
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "this machine".to_owned())
}

/// `hb login`: shows a code, waits for the person to approve it in the
/// browser, and stores the mandate the server issues.
pub fn login(server: &str, json_mode: bool) -> Result<Output> {
    let server = normalize(server);
    let client = Client::new(&server, None);
    let start = client.post(
        "/auth/device",
        &json!({ "client_name": format!("hb on {}", hostname()) }),
    )?;
    let device_code = start["device_code"]
        .as_str()
        .ok_or_else(|| anyhow!("unexpected server reply"))?;
    let user_code = start["user_code"].as_str().unwrap_or("?");
    let uri = start["verification_uri_complete"]
        .as_str()
        .or_else(|| start["verification_uri"].as_str())
        .unwrap_or("");
    let interval = Duration::from_secs(start["interval"].as_u64().unwrap_or(3).max(1));
    let deadline =
        Instant::now() + Duration::from_secs(start["expires_in"].as_u64().unwrap_or(900));
    if json_mode {
        eprintln!(
            "{}",
            json!({ "user_code": user_code, "verification_uri": uri })
        );
    } else {
        eprintln!("Open {uri}\nand approve the code {user_code}. Waiting…");
    }
    loop {
        if Instant::now() > deadline {
            bail!("the code expired; run `hb login` again");
        }
        std::thread::sleep(interval);
        let reply = client.post("/auth/device/token", &json!({ "device_code": device_code }))?;
        match reply["status"].as_str() {
            Some("approved") => {
                let mandate = reply["mandate"]
                    .as_str()
                    .ok_or_else(|| anyhow!("no mandate in reply"))?;
                let handle = reply["account"]["handle"]
                    .as_str()
                    .unwrap_or("?")
                    .to_owned();
                let mut creds = load();
                creds.servers.insert(
                    server.clone(),
                    Stored {
                        mandate: mandate.to_owned(),
                        handle: handle.clone(),
                    },
                );
                save(&creds)?;
                return Ok(Output::new(
                    json!({ "server": server, "account": reply["account"] }),
                    format!("Signed in to {server} as {handle}.\n"),
                ));
            }
            Some("denied") => bail!("the sign-in was denied"),
            Some("used") => bail!("this code was already used; run `hb login` again"),
            _ => {}
        }
    }
}

/// `hb logout`: revokes the stored mandate on the server and forgets it.
pub fn logout(server: &str) -> Result<Output> {
    let server = normalize(server);
    let mut creds = load();
    let Some(stored) = creds.servers.remove(&server) else {
        bail!("not signed in to {server}");
    };
    let revoked = hebra_mandate::root_id(&stored.mandate)
        .ok()
        .map(|id| {
            Client::new(&server, Some(stored.mandate.clone())).send(
                "DELETE",
                &format!("/me/mandates/{id}"),
                &json!({}),
            )
        })
        .is_some_and(|r| r.is_ok());
    save(&creds)?;
    Ok(Output::new(
        json!({ "server": server, "revoked": revoked }),
        format!(
            "Signed out of {server}{}.\n",
            if revoked {
                " (mandate revoked)"
            } else {
                " (could not reach the server to revoke the mandate)"
            }
        ),
    ))
}

pub fn whoami(server: &str) -> Result<Output> {
    let server = normalize(server);
    let token = credential(&server)
        .ok_or_else(|| anyhow!("not signed in to {server}; run `hb login {server}`"))?;
    let me = Client::new(&server, Some(token)).get("/me")?;
    let mut text = format!(
        "{} on {server}{}\n",
        me["display"].as_str().unwrap_or("?"),
        if me["account"]["site_admin"] == true {
            " (server admin)"
        } else {
            ""
        }
    );
    if let Some(orgs) = me["orgs"].as_array().filter(|o| !o.is_empty()) {
        let list: Vec<String> = orgs
            .iter()
            .map(|o| {
                format!(
                    "{} ({})",
                    o["account"]["handle"].as_str().unwrap_or("?"),
                    o["role"].as_str().unwrap_or("?")
                )
            })
            .collect();
        let _ = writeln!(text, "Organizations: {}", list.join(", "));
    }
    Ok(Output::new(me, text))
}

/// What `hb mandate grant` hands to an agent.
pub struct Grant {
    pub agent: String,
    pub repos: Vec<String>,
    pub operations: Vec<String>,
    pub bookmarks: Vec<String>,
    pub ttl_secs: i64,
    pub purpose: Option<String>,
    pub session: Option<String>,
}

/// Narrows your mandate for an agent, offline. The result can do at most
/// what yours can, limited to `grant`.
pub fn grant(server: &str, g: &Grant) -> Result<Output> {
    let server = normalize(server);
    let token = credential(&server)
        .ok_or_else(|| anyhow!("not signed in to {server}; run `hb login {server}`"))?;
    let operations = if g.operations.is_empty() {
        vec![
            op::READ.to_owned(),
            op::PUSH.to_owned(),
            op::REVIEW.to_owned(),
            op::TASK.to_owned(),
        ]
    } else {
        g.operations.clone()
    };
    let bookmarks = if g.bookmarks.is_empty() {
        vec![format!("agents/{}/*", g.agent)]
    } else {
        g.bookmarks.clone()
    };
    let expires_at = jiff::Timestamp::now().as_second() + g.ttl_secs;
    let restrictions = Restrictions {
        repos: g.repos.clone(),
        operations: operations.clone(),
        bookmarks: bookmarks.clone(),
        expires_at: Some(expires_at),
        delegation: Delegation {
            agent: Some(g.agent.clone()),
            purpose: g.purpose.clone(),
            session: g.session.clone(),
        },
    };
    let mandate = hebra_mandate::attenuate(&token, &restrictions)?;
    let until = jiff::Timestamp::from_second(expires_at)
        .map(|t| t.to_string())
        .unwrap_or_default();
    let text = format!(
        "{mandate}\n\nFor {} until {until}: {} on {}, bookmarks {}.\nGive it to the agent as HB_TOKEN.\n",
        g.agent,
        operations.join(", "),
        if g.repos.is_empty() {
            "every repository you can access".to_owned()
        } else {
            g.repos.join(", ")
        },
        bookmarks.join(", "),
    );
    Ok(Output::new(
        json!({ "mandate": mandate, "agent": g.agent, "repos": g.repos, "operations": operations, "bookmarks": bookmarks, "expires_at": until }),
        text,
    ))
}

pub fn list_mandates(server: &str) -> Result<Output> {
    let server = normalize(server);
    let token = credential(&server).ok_or_else(|| anyhow!("not signed in to {server}"))?;
    let data = Client::new(&server, Some(token)).get("/me/mandates")?;
    let mut text = String::new();
    for m in data["mandates"].as_array().cloned().unwrap_or_default() {
        let _ = writeln!(
            text,
            "{}  {:<24} {}{}",
            m["id"].as_str().map_or("?", |i| i.get(..16).unwrap_or(i)),
            m["label"].as_str().unwrap_or(""),
            if m["revoked"] == true {
                "revoked"
            } else {
                "active"
            },
            m["expires_at"]
                .as_str()
                .map(|e| format!(", expires {e}"))
                .unwrap_or_default(),
        );
    }
    if text.is_empty() {
        text.push_str("No mandates.\n");
    }
    Ok(Output::new(data, text))
}

pub fn revoke_mandate(server: &str, id: &str) -> Result<Output> {
    let server = normalize(server);
    let token = credential(&server).ok_or_else(|| anyhow!("not signed in to {server}"))?;
    let reply = Client::new(&server, Some(token)).send(
        "DELETE",
        &format!("/me/mandates/{id}"),
        &Value::Null,
    )?;
    Ok(Output::new(
        reply,
        format!("Revoked {id} and every mandate derived from it.\n"),
    ))
}
