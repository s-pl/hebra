//! `hebra-load`: how much Hebra can take.
//!
//! - `micro`: the hot paths in-process on every core (object ids, encrypted
//!   object store, rate limiter, permission globs), no network.
//! - `http`: a real server under a mixed workload of readers and writers
//!   (bookmark compare-and-swap, presence) while live subscribers check that
//!   every event reaches them.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use hebra_core::ObjectId;
use hebra_core::model::{Actor, ActorKind, Commit, FileEntry, Provenance, Tree};
use hebra_core::store::{BLOB, Backend, COMMIT, Key, Store, TREE};
use serde_json::{Value, json};

#[derive(Parser)]
#[command(
    name = "hebra-load",
    about = "Load tests and micro-benchmarks for Hebra"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// In-process benchmarks of the hot paths, on every core.
    Micro {
        #[arg(long, default_value_t = 5)]
        secs: u64,
        /// Threads (default: all logical cores).
        #[arg(long)]
        threads: Option<usize>,
    },
    /// Mixed read/write load against a running server.
    Http {
        /// Server base URL(s), comma separated (requests rotate between them).
        #[arg(long, value_delimiter = ',', default_value = "http://127.0.0.1:4600")]
        url: Vec<String>,
        /// Admin token (creates the test repository).
        #[arg(long, env = "HB_TOKEN")]
        token: String,
        #[arg(long, default_value_t = 30)]
        secs: u64,
        /// Concurrent clients.
        #[arg(long, default_value_t = 256)]
        concurrency: usize,
        /// Live (SSE) subscribers that must receive every bookmark event.
        #[arg(long, default_value_t = 50)]
        subscribers: usize,
        /// Only reads (no writes).
        #[arg(long)]
        read_only: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Micro { secs, threads } => micro(secs, threads),
        Command::Http {
            url,
            token,
            secs,
            concurrency,
            subscribers,
            read_only,
        } => http(url, token, secs, concurrency, subscribers, read_only).await,
    }
}

// ------------------------------------------------------------------ micro

/// An in-memory backend, so the store benchmark measures hashing and
/// encryption rather than the disk.
#[derive(Debug, Default)]
struct MemBackend {
    shards: Vec<std::sync::Mutex<HashMap<String, Vec<u8>>>>,
}

impl MemBackend {
    fn new() -> Self {
        Self {
            shards: (0..256)
                .map(|_| std::sync::Mutex::new(HashMap::new()))
                .collect(),
        }
    }
    fn shard(&self, key: &str) -> std::sync::MutexGuard<'_, HashMap<String, Vec<u8>>> {
        let i = usize::from_str_radix(&key[..2], 16).unwrap_or(0) % self.shards.len();
        self.shards[i]
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Backend for MemBackend {
    fn get(&self, key: &str) -> std::io::Result<Option<Vec<u8>>> {
        Ok(self.shard(key).get(key).cloned())
    }
    fn put(&self, key: &str, bytes: &[u8]) -> std::io::Result<()> {
        self.shard(key).insert(key.to_owned(), bytes.to_vec());
        Ok(())
    }
    fn exists(&self, key: &str) -> std::io::Result<bool> {
        Ok(self.shard(key).contains_key(key))
    }
}

/// Runs `op(thread, i)` on `threads` threads for `secs`; returns ops/second.
fn run(threads: usize, secs: u64, op: impl Fn(usize, u64) + Sync) -> f64 {
    let stop = AtomicBool::new(false);
    let total = AtomicU64::new(0);
    let start = Instant::now();
    std::thread::scope(|s| {
        for t in 0..threads {
            let (stop, total, op) = (&stop, &total, &op);
            s.spawn(move || {
                let mut i = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    for _ in 0..256 {
                        op(t, i);
                        i += 1;
                    }
                }
                total.fetch_add(i, Ordering::Relaxed);
            });
        }
        std::thread::sleep(Duration::from_secs(secs));
        stop.store(true, Ordering::Relaxed);
    });
    #[allow(clippy::cast_precision_loss)]
    let ops = total.load(Ordering::Relaxed) as f64;
    ops / start.elapsed().as_secs_f64()
}

fn report(name: &str, per_sec: f64) {
    println!("  {name:<52} {:>14} ops/s", group(per_sec));
}

fn group(n: f64) -> String {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let s = (n.round() as u64).to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn micro(secs: u64, threads: Option<usize>) -> anyhow::Result<()> {
    let threads = threads
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(4, std::num::NonZero::get));
    println!("micro-benchmarks: {threads} threads, {secs}s each\n");

    let small = vec![7u8; 1024];
    report(
        "object id: BLAKE3 of a 1 KiB blob",
        run(threads, secs, |t, i| {
            let mut d = small.clone();
            d[..8].copy_from_slice(&i.to_le_bytes());
            d[8] = t as u8;
            std::hint::black_box(ObjectId::hash(BLOB, &d));
        }),
    );

    let key = Key::from_hex(&Key::generate_hex())?;
    let store = Store::with_backend(Arc::new(MemBackend::new()), vec![key.clone()]);
    report(
        "store write, encrypted (hash + XChaCha20-Poly1305), 1 KiB",
        run(threads, secs, |t, i| {
            let mut d = small.clone();
            d[..8].copy_from_slice(&i.to_le_bytes());
            d[8] = t as u8;
            std::hint::black_box(store.write(BLOB, &d).ok());
        }),
    );

    let hot: Vec<ObjectId> = (0..4096u32)
        .map(|i| store.write(BLOB, &i.to_le_bytes()))
        .collect::<Result<_, _>>()?;
    report(
        "store read, decrypt + verify hash (small objects)",
        run(threads, secs, |_, i| {
            let id = &hot[(i % 4096) as usize];
            std::hint::black_box(store.read(BLOB, id).ok());
        }),
    );

    let limiter = hebra_server::RateLimiter::new(
        hebra_server::Limits {
            token_rps: 1e12,
            token_burst: 1e12,
            ip_rps: 1e12,
            ip_burst: 1e12,
        },
        false,
    );
    let peers: Vec<std::net::SocketAddr> = (0..threads)
        .map(|t| format!("10.0.{}.{}:1", t / 250, t % 250 + 1).parse())
        .collect::<Result<_, _>>()?;
    let headers: Vec<axum_headers::HeaderMap> = (0..threads)
        .map(|t| {
            let mut h = axum_headers::HeaderMap::new();
            if let Ok(v) = axum_headers::HeaderValue::from_str(&format!("Bearer hbt_{t:048}")) {
                h.insert(axum_headers::AUTHORIZATION, v);
            }
            h
        })
        .collect();
    report(
        "rate limiter: IP + token buckets per request",
        run(threads, secs, |t, _| {
            std::hint::black_box(limiter.check(&headers[t], Some(peers[t])).is_ok());
        }),
    );

    let patterns = ["agents/claude-code/*", "release-*-final", "main"];
    report(
        "permission check: bookmark glob patterns",
        run(threads, secs, |_, i| {
            let name = if i % 2 == 0 {
                "agents/claude-code/google-sign-in"
            } else {
                "release-2.3-final"
            };
            std::hint::black_box(patterns.iter().any(|p| hebra_server::glob_match(p, name)));
        }),
    );

    let pack_objects: Vec<(ObjectId, Vec<u8>)> = hot
        .iter()
        .take(64)
        .map(|id| Ok((*id, store.read_raw(id)?)))
        .collect::<hebra_core::Result<_>>()?;
    let pack = hebra_core::transport::encode(&pack_objects);
    report(
        "pack decode + verify, 64 objects (objects/s)",
        run(threads, secs, |_, _| {
            std::hint::black_box(hebra_core::transport::decode(&pack).ok());
        }) * 64.0,
    );
    Ok(())
}

/// The `http` crate's header types, as re-exported by axum through the server.
mod axum_headers {
    pub use hebra_server::http::header::AUTHORIZATION;
    pub use hebra_server::http::{HeaderMap, HeaderValue};
}

// ------------------------------------------------------------------- http

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Op {
    RepoInfo,
    Log,
    File,
    PresenceGet,
    PresencePost,
    BookmarkCas,
}

impl Op {
    const ALL: [Op; 6] = [
        Op::RepoInfo,
        Op::Log,
        Op::File,
        Op::PresenceGet,
        Op::PresencePost,
        Op::BookmarkCas,
    ];
    fn name(self) -> &'static str {
        match self {
            Op::RepoInfo => "GET  repo (bookmarks, permissions)",
            Op::Log => "GET  log (walk commits)",
            Op::File => "GET  file at main (decrypt blob)",
            Op::PresenceGet => "GET  presence",
            Op::PresencePost => "POST presence (upsert + live event)",
            Op::BookmarkCas => "POST bookmark CAS (txn + event + fan-out)",
        }
    }
    /// Weights out of 100: mostly reads, like a busy team.
    fn pick(n: u64, read_only: bool) -> Op {
        let n = n % 100;
        match (n, read_only) {
            (0..=34, _) => Op::RepoInfo,
            (35..=49, _) => Op::Log,
            (50..=64, _) => Op::File,
            (65..=74, _) => Op::PresenceGet,
            (_, true) => Op::RepoInfo,
            (75..=89, false) => Op::PresencePost,
            _ => Op::BookmarkCas,
        }
    }
}

#[derive(Default)]
struct Stats {
    latencies: HashMap<Op, Vec<u32>>,
    errors: HashMap<String, u64>,
}

fn object(store: &Store, id: &ObjectId) -> anyhow::Result<(ObjectId, Vec<u8>)> {
    Ok((*id, store.read_raw(id)?))
}

/// Two commits (A, and B on top of A) to move bookmarks between.
fn fixture() -> anyhow::Result<(ObjectId, ObjectId, Vec<u8>)> {
    let store = Store::with_backend(Arc::new(MemBackend::new()), Vec::new());
    let author = Actor {
        name: "hebra-load".into(),
        kind: ActorKind::Agent,
        on_behalf_of: None,
    };
    let mut commits = Vec::new();
    let mut parent = None;
    let mut objects = Vec::new();
    for (n, body) in ["fn main() {}\n", "fn main() { println!(\"load\"); }\n"]
        .iter()
        .enumerate()
    {
        let blob = store.write(BLOB, body.repeat(40).as_bytes())?;
        let mut tree = Tree::default();
        tree.files.insert(
            "src/main.rs".into(),
            FileEntry {
                blob,
                executable: false,
            },
        );
        let tree_id = store.write_json(TREE, &tree)?;
        let commit = Commit {
            change_id: hebra_core::ChangeId::random(),
            parents: parent.into_iter().collect(),
            predecessors: Vec::new(),
            tree: tree_id,
            description: format!("load test commit {n}"),
            author: author.clone(),
            timestamp: jiff::Timestamp::now().to_string(),
            provenance: Provenance::default(),
            conflicts: Vec::new(),
        };
        let id = store.write_json(COMMIT, &commit)?;
        objects.extend([
            object(&store, &blob)?,
            object(&store, &tree_id)?,
            object(&store, &id)?,
        ]);
        commits.push(id);
        parent = Some(id);
    }
    Ok((
        commits[0],
        commits[1],
        hebra_core::transport::encode(&objects),
    ))
}

async fn check(resp: reqwest::Response) -> anyhow::Result<Value> {
    let status = resp.status();
    let body = resp.bytes().await?;
    if !status.is_success() {
        bail!("{status}: {}", String::from_utf8_lossy(&body));
    }
    Ok(serde_json::from_slice(&body).unwrap_or(Value::Null))
}

#[allow(clippy::cast_precision_loss, clippy::too_many_lines)]
async fn http(
    urls: Vec<String>,
    token: String,
    secs: u64,
    concurrency: usize,
    subscribers: usize,
    read_only: bool,
) -> anyhow::Result<()> {
    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(concurrency)
        .tcp_nodelay(true)
        .build()?;
    let base = urls[0].trim_end_matches('/').to_owned();
    let auth = format!("Bearer {token}");
    let repo = format!("load-{}", jiff::Timestamp::now().as_second());

    // Repository with two commits; `main` at A.
    check(
        client
            .post(format!("{base}/api/v1/repos"))
            .header("authorization", &auth)
            .json_body(&json!({ "name": repo, "protect": [] }))
            .await?,
    )
    .await
    .context("creating the test repository (is the token admin?)")?;
    let (a, b, pack) = fixture()?;
    check(
        client
            .post(format!("{base}/api/v1/repos/{repo}/objects"))
            .header("authorization", &auth)
            .body(pack)
            .send()
            .await?,
    )
    .await?;
    check(
        client
            .post(format!("{base}/api/v1/repos/{repo}/bookmarks"))
            .header("authorization", &auth)
            .json_body(&json!({ "name": "main", "old": null, "new": a.hex() }))
            .await?,
    )
    .await?;

    // Live subscribers count bookmark events.
    let received = Arc::new(AtomicU64::new(0));
    let resyncs = Arc::new(AtomicU64::new(0));
    let ready = Arc::new(AtomicU64::new(0));
    for i in 0..subscribers {
        let url = format!(
            "{}/api/v1/repos/{repo}/stream",
            urls[i % urls.len()].trim_end_matches('/')
        );
        let (client, received, resyncs, ready) = (
            client.clone(),
            Arc::clone(&received),
            Arc::clone(&resyncs),
            Arc::clone(&ready),
        );
        tokio::spawn(async move {
            use futures_util::StreamExt;
            let Ok(resp) = client.get(&url).send().await else {
                return;
            };
            let mut body = resp.bytes_stream();
            // Count event lines straight from the bytes, keeping only the
            // unfinished last line between chunks.
            let mut carry: Vec<u8> = Vec::new();
            while let Some(Ok(chunk)) = body.next().await {
                carry.extend_from_slice(&chunk);
                let Some(last) = carry.iter().rposition(|&b| b == b'\n') else {
                    continue;
                };
                for line in carry[..last].split(|&b| b == b'\n') {
                    match line {
                        b"event: bookmark" => {
                            received.fetch_add(1, Ordering::Relaxed);
                        }
                        b"event: ready" => {
                            ready.fetch_add(1, Ordering::Relaxed);
                        }
                        b"event: resync" => {
                            resyncs.fetch_add(1, Ordering::Relaxed);
                        }
                        _ => {}
                    }
                }
                carry.drain(..=last);
            }
        });
    }
    let wait = Instant::now();
    while ready.load(Ordering::Relaxed) < subscribers as u64
        && wait.elapsed() < Duration::from_secs(20)
    {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    println!(
        "http load: {} for {secs}s, {concurrency} clients, {} live subscribers, servers: {}\n",
        if read_only {
            "reads only"
        } else {
            "75% reads / 25% writes"
        },
        ready.load(Ordering::Relaxed),
        urls.join(", ")
    );

    let deadline = Instant::now() + Duration::from_secs(secs);
    let started = Instant::now();
    let mut tasks = Vec::new();
    for w in 0..concurrency {
        let client = client.clone();
        let base = urls[w % urls.len()].trim_end_matches('/').to_owned();
        let (auth, repo) = (auth.clone(), repo.clone());
        tasks.push(tokio::spawn(async move {
            let mut stats = Stats::default();
            let bookmark = format!("load/w{w}");
            let mut current: Option<ObjectId> = None;
            let mut n: u64 = (w as u64).wrapping_mul(2_654_435_761);
            while Instant::now() < deadline {
                n = n.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
                let op = Op::pick(n >> 33, read_only);
                let t0 = Instant::now();
                let req = match op {
                    Op::RepoInfo => client.get(format!("{base}/api/v1/repos/{repo}")),
                    Op::Log => client.get(format!("{base}/api/v1/repos/{repo}/log?limit=20")),
                    Op::File => client.get(format!(
                        "{base}/api/v1/repos/{repo}/tree/main?path=src/main.rs"
                    )),
                    Op::PresenceGet => client.get(format!("{base}/api/v1/repos/{repo}/presence")),
                    Op::PresencePost => client
                        .post(format!("{base}/api/v1/repos/{repo}/presence"))
                        .header("content-type", "application/json")
                        .body(
                            json!({ "client": format!("load-{w}"), "machine": "load", "workspaces": [] })
                                .to_string(),
                        ),
                    Op::BookmarkCas => {
                        let next = if current == Some(a) { b } else { a };
                        let body = json!({
                            "name": bookmark,
                            "old": current.map(|c| c.hex()),
                            "new": next.hex(),
                        });
                        current = Some(next);
                        client
                            .post(format!("{base}/api/v1/repos/{repo}/bookmarks"))
                            .header("content-type", "application/json")
                            .body(body.to_string())
                    }
                };
                let result = req.header("authorization", &auth).send().await;
                match result {
                    Ok(resp) if resp.status().is_success() => {
                        let _ = resp.bytes().await;
                        #[allow(clippy::cast_possible_truncation)]
                        let micros = t0.elapsed().as_micros() as u32;
                        stats.latencies.entry(op).or_default().push(micros);
                    }
                    Ok(resp) => {
                        *stats.errors.entry(format!("{} {}", op.name(), resp.status())).or_default() += 1;
                        if op == Op::BookmarkCas {
                            current = None;
                            // Re-sync our view of the bookmark.
                            let info = client
                                .get(format!("{base}/api/v1/repos/{repo}"))
                                .header("authorization", &auth)
                                .send()
                                .await;
                            if let Ok(info) = info
                                && let Ok(v) = check(info).await
                            {
                                current = v["bookmarks"][&bookmark]
                                    .as_str()
                                    .and_then(|s| ObjectId::parse(s).ok());
                            }
                        }
                        let _ = resp.bytes().await;
                    }
                    Err(e) => {
                        let kind = if e.is_timeout() { "timeout" } else if e.is_connect() { "connect" } else { "transport" };
                        *stats.errors.entry(format!("{} {kind}", op.name())).or_default() += 1;
                    }
                }
            }
            stats
        }));
    }

    let mut all = Stats::default();
    for t in tasks {
        let s = t.await?;
        for (op, mut l) in s.latencies {
            all.latencies.entry(op).or_default().append(&mut l);
        }
        for (k, v) in s.errors {
            *all.errors.entry(k).or_default() += v;
        }
    }
    let elapsed = started.elapsed().as_secs_f64();
    // Let the last live events arrive.
    tokio::time::sleep(Duration::from_secs(2)).await;

    println!(
        "  {:<44} {:>10} {:>9} {:>8} {:>8} {:>8} {:>8}",
        "operation", "ok", "ops/s", "p50 ms", "p90 ms", "p99 ms", "p99.9"
    );
    let mut total = 0usize;
    let mut everything: Vec<u32> = Vec::new();
    for op in Op::ALL {
        let Some(l) = all.latencies.get_mut(&op) else {
            continue;
        };
        l.sort_unstable();
        total += l.len();
        everything.extend_from_slice(l);
        #[allow(clippy::cast_precision_loss)]
        let rate = l.len() as f64 / elapsed;
        println!(
            "  {:<44} {:>10} {:>9} {:>8.2} {:>8.2} {:>8.2} {:>8.2}",
            op.name(),
            l.len(),
            group(rate),
            pct(l, 50.0),
            pct(l, 90.0),
            pct(l, 99.0),
            pct(l, 99.9)
        );
    }
    everything.sort_unstable();
    #[allow(clippy::cast_precision_loss)]
    let rate = total as f64 / elapsed;
    println!(
        "  {:<44} {:>10} {:>9} {:>8.2} {:>8.2} {:>8.2} {:>8.2}",
        "ALL",
        total,
        group(rate),
        pct(&everything, 50.0),
        pct(&everything, 90.0),
        pct(&everything, 99.0),
        pct(&everything, 99.9)
    );
    let errors: u64 = all.errors.values().sum();
    println!("\n  errors: {errors}");
    let mut errs: Vec<_> = all.errors.into_iter().collect();
    errs.sort_by_key(|e| std::cmp::Reverse(e.1));
    for (k, v) in errs.iter().take(8) {
        println!("    {v:>8}  {k}");
    }
    let moves = all.latencies.get(&Op::BookmarkCas).map_or(0, Vec::len) as u64;
    if subscribers > 0 && moves > 0 {
        let expected = moves * ready.load(Ordering::Relaxed);
        let got = received.load(Ordering::Relaxed);
        #[allow(clippy::cast_precision_loss)]
        let ratio = got as f64 / expected as f64 * 100.0;
        println!(
            "\n  live fan-out: {} of {} bookmark events delivered ({ratio:.2}%), {} resyncs, {} events/s pushed to subscribers",
            group(got as f64),
            group(expected as f64),
            resyncs.load(Ordering::Relaxed),
            group(got as f64 / elapsed)
        );
    }
    Ok(())
}

fn pct(sorted: &[u32], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let i = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    f64::from(sorted[i]) / 1000.0
}

/// Small helper: JSON body without reqwest's `json` feature.
trait JsonBody {
    fn json_body(
        self,
        v: &Value,
    ) -> impl std::future::Future<Output = reqwest::Result<reqwest::Response>>;
}

impl JsonBody for reqwest::RequestBuilder {
    fn json_body(
        self,
        v: &Value,
    ) -> impl std::future::Future<Output = reqwest::Result<reqwest::Response>> {
        self.header("content-type", "application/json")
            .body(v.to_string())
            .send()
    }
}
