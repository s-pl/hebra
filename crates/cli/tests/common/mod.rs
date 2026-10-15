//! A real hebra-server for end-to-end tests, and helpers to run `hb`.
//!
//! Like production: objects encrypted, and, when these variables are set (CI
//! does), metadata in PostgreSQL, objects in S3 and live events over NATS:
//! HEBRA_TEST_DATABASE_URL, HEBRA_TEST_S3_BUCKET, HEBRA_TEST_NATS_URL.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;

use hebra_core::store::Key;
use hebra_mandate::{Delegation, Restrictions, op};
use hebra_server::db::AccountKind;
use hebra_server::{Config, Edition, NatsConfig, ObjectsConfig, Server};
use serde_json::{Value, json};

pub struct Env {
    _dir: tempfile::TempDir,
    pub root: PathBuf,
    /// `http://127.0.0.1:port`
    pub server: String,
    /// `owner/name`
    pub repo: String,
    /// `http://127.0.0.1:port/owner/name`
    pub url: String,
    /// Ana's own mandate (a person, server admin, owner of the repository).
    pub human: String,
    /// Ana's mandate narrowed for the agent `claude-code`.
    pub agent: String,
    /// A web session for Ana (approving device sign-ins).
    pub session: String,
    /// Bob: another person on the server, no access yet.
    pub bob: String,
    pub bob_handle: String,
    /// Eve: a person outside everything.
    pub eve: String,
    pub eve_handle: String,
    pub config_dir: PathBuf,
}

fn config(data: &Path, edition: Edition) -> Config {
    let mut config = Config::local(data);
    config.edition = edition;
    config.encryption_keys = vec![Key::from_hex(&Key::generate_hex()).unwrap()];
    config.database_url = std::env::var("HEBRA_TEST_DATABASE_URL").ok();
    if config.database_url.is_some() {
        config.mandate_key = Some(hebra_mandate::Authority::generate().to_hex());
    }
    if let Ok(bucket) = std::env::var("HEBRA_TEST_S3_BUCKET") {
        config.objects = ObjectsConfig::S3 { bucket };
    }
    config.nats = std::env::var("HEBRA_TEST_NATS_URL")
        .ok()
        .map(|url| NatsConfig {
            url,
            ..NatsConfig::default()
        });
    config
}

/// A unique suffix from the random temp dir, so tests can share a database.
fn unique(dir: &Path) -> String {
    dir.file_name()
        .unwrap()
        .to_string_lossy()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_lowercase()
}

pub fn start_server() -> Env {
    start(Edition::Community)
}

pub fn start(edition: Edition) -> Env {
    start_with(edition, None)
}

/// A server with a model gateway (for agent verification tests).
pub fn start_with(edition: Edition, gateway: Option<hebra_server::ModelGateway>) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("server");
    let suffix = unique(dir.path());
    let ana = format!("ana-{}", &suffix[suffix.len().saturating_sub(10)..]);
    let repo = format!("{ana}/acme");
    let (tx, rx) = std::sync::mpsc::channel();
    let (handle, full_name) = (ana.clone(), repo.clone());
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let _ = rustls::crypto::ring::default_provider().install_default();
            let mut cfg = config(&data, edition);
            cfg.gateway = gateway;
            let server = Server::open(cfg).await.unwrap();
            let db = server.db();
            let person = db
                .create_account(
                    &handle,
                    AccountKind::Human,
                    "Ana",
                    Some("ana@example.com"),
                    None,
                )
                .await
                .unwrap();
            db.set_site_admin(&person.id, true).await.unwrap();
            let created = db
                .create_repo(
                    &handle,
                    &hebra_server::db::NewRepo {
                        owner_id: &person.id,
                        name: "acme",
                        public: false,
                        description: "",
                        protect: &["main".to_owned()],
                        forked_from: None,
                    },
                )
                .await
                .unwrap();
            server.create_objects(&created.id).unwrap();
            let (human, root) = server.authority().issue(&person.id, None).unwrap();
            db.record_mandate(&root, &person.id, "tests", None, None)
                .await
                .unwrap();
            let agent = hebra_mandate::attenuate(
                &human,
                &Restrictions {
                    repos: vec![full_name.clone()],
                    operations: vec![
                        op::READ.into(),
                        op::PUSH.into(),
                        op::REVIEW.into(),
                        op::TASK.into(),
                        op::MERGE.into(),
                    ],
                    bookmarks: vec!["agents/claude-code/*".into(), "main".into()],
                    expires_at: Some(jiff::Timestamp::now().as_second() + 3600),
                    delegation: Delegation {
                        agent: Some("claude-code".into()),
                        purpose: Some("tests".into()),
                        session: None,
                    },
                },
            )
            .unwrap();
            let session = hebra_server::identity::secret::token(
                hebra_server::identity::secret::SESSION_PREFIX,
            );
            db.create_session(
                &hebra_server::identity::secret::hash(&session),
                &person.id,
                3_600_000,
                None,
            )
            .await
            .unwrap();
            let mut people = Vec::new();
            for who in ["bob", "eve"] {
                let handle = format!("{who}-{}", &handle[4..]);
                let account = db
                    .create_account(&handle, AccountKind::Human, who, None, None)
                    .await
                    .unwrap();
                let (mandate, root) = server.authority().issue(&account.id, None).unwrap();
                db.record_mandate(&root, &account.id, "tests", None, None)
                    .await
                    .unwrap();
                people.push((handle, mandate));
            }
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            tx.send((
                listener.local_addr().unwrap(),
                human,
                agent,
                session,
                people,
            ))
            .unwrap();
            let server = Arc::new(server);
            server.spawn_maintenance();
            hebra_server::serve(server, listener).await.unwrap();
        });
    });
    let (addr, human, agent, session, mut people) = rx.recv().unwrap();
    let (eve_handle, eve) = people.pop().unwrap();
    let (bob_handle, bob) = people.pop().unwrap();
    let root = dir.path().to_path_buf();
    let config_dir = root.join("hb-config");
    let server = format!("http://{addr}");
    Env {
        url: format!("{server}/{repo}"),
        server,
        repo,
        human,
        agent,
        session,
        bob,
        bob_handle,
        eve,
        eve_handle,
        config_dir,
        root,
        _dir: dir,
    }
}

impl Env {
    /// `hb` with this environment's config directory (credentials, runners).
    pub fn hb(&self, cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_hb"));
        cmd.current_dir(cwd)
            .args(args)
            .env_remove("CLAUDECODE")
            .env_remove("HB_TOKEN")
            .env_remove("HB_ACTOR")
            .env_remove("HB_SERVER")
            .env("HB_CONFIG_DIR", &self.config_dir)
            .env("USER", "tester")
            .env("USERNAME", "tester");
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.output().unwrap()
    }

    pub fn ok(&self, cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> Value {
        let mut full = vec!["--json"];
        full.extend_from_slice(args);
        let out = self.hb(cwd, &full, env);
        assert!(
            out.status.success(),
            "hb {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }

    pub fn fail(&self, cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> String {
        let mut full = vec!["--json"];
        full.extend_from_slice(args);
        let out = self.hb(cwd, &full, env);
        assert!(!out.status.success(), "hb {args:?} should fail");
        String::from_utf8_lossy(&out.stderr).into_owned()
    }

    /// A fresh clone directory with `origin` pointing at the repository.
    pub fn clone_dir(&self, name: &str, token: &str) -> PathBuf {
        let dir = self.root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        self.ok(&dir, &["init"], &[]);
        self.ok(&dir, &["config", "set", "user.name", name], &[]);
        self.ok(
            &dir,
            &["remote", "add", "origin", &self.url, "--token", token],
            &[],
        );
        dir
    }

    /// Calls the API directly with a credential.
    pub fn api(&self, method: &str, path: &str, token: &str, body: Option<Value>) -> (u16, Value) {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into();
        let url = format!("{}/api/v1{path}", self.server);
        let auth = format!("Bearer {token}");
        let mut resp = match method {
            "GET" if token.is_empty() => agent.get(&url).call().unwrap(),
            "GET" => agent
                .get(&url)
                .header("Authorization", &auth)
                .call()
                .unwrap(),
            "DELETE" => agent
                .delete(&url)
                .header("Authorization", &auth)
                .call()
                .unwrap(),
            m => {
                let req = match m {
                    "POST" => agent.post(&url),
                    "PUT" => agent.put(&url),
                    "PATCH" => agent.patch(&url),
                    other => panic!("method {other}"),
                };
                req.header("Authorization", &auth)
                    .header("Content-Type", "application/json")
                    .send(body.unwrap_or_else(|| json!({})).to_string().as_bytes())
                    .unwrap()
            }
        };
        let status = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// Ana's handle.
    pub fn ana(&self) -> &str {
        self.repo.split('/').next().unwrap()
    }
}
