//! Where machines come from: cloud APIs (Hetzner Cloud, DigitalOcean,
//! Vultr) or Docker on the server's own host. Each only lists what it
//! offers, creates, lists and deletes machines; Heddle decides.
//!
//! Every machine carries a tag with its fleet, so the orchestrator can
//! find machines it no longer remembers (and delete them before they cost
//! more).

use hebra_heddle::{Billing, MachineType};
use serde_json::{Value, json};

use crate::error::{ApiError, ApiResult};

/// A machine the provider has, as it reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub provider_id: String,
    pub name: String,
    pub running: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Created {
    pub provider_id: String,
    pub address: Option<String>,
}

/// The providers Heddle can drive.
pub const KINDS: [&str; 4] = ["hetzner", "digitalocean", "vultr", "docker"];

pub enum Provider {
    Cloud(Cloud),
    Docker(Docker),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Api {
    Hetzner,
    DigitalOcean,
    Vultr,
}

pub struct Cloud {
    api: Api,
    base: String,
    token: String,
    region: String,
    image: String,
    ssh_keys: Vec<Value>,
    billing: Option<Billing>,
    http: reqwest::Client,
    labels: Vec<String>,
}

pub struct Docker {
    image: String,
    types: Vec<MachineType>,
}

fn bad(message: impl Into<String>) -> ApiError {
    ApiError::bad(message)
}

fn provider_error(api: &str, e: impl std::fmt::Display) -> ApiError {
    ApiError::new(
        axum::http::StatusCode::BAD_GATEWAY,
        "provider",
        format!("{api}: {e}"),
    )
}

fn text(config: &Value, key: &str, default: &str) -> String {
    config[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or(default)
        .to_owned()
}

/// Labels every machine of the fleet has (what checks may ask for).
pub fn fleet_labels(config: &Value) -> Vec<String> {
    let labels: Vec<String> = config["labels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| l.as_str().map(str::to_owned))
        .collect();
    if labels.is_empty() {
        vec!["linux".to_owned()]
    } else {
        labels
    }
}

impl Provider {
    /// The provider of a fleet. `token` is its opened API token.
    pub fn new(
        kind: &str,
        config: &Value,
        token: Option<String>,
        http: reqwest::Client,
    ) -> ApiResult<Self> {
        let labels = fleet_labels(config);
        let billing = match config["billing"].as_str() {
            Some("second") => Some(Billing::Second),
            Some("hour") => Some(Billing::Hour),
            _ => None,
        };
        let cloud = |api: Api, base: &str, region: &str, image: &str| -> ApiResult<Self> {
            let token = token
                .clone()
                .filter(|t| !t.is_empty())
                .ok_or_else(|| bad("this provider needs an API token"))?;
            Ok(Self::Cloud(Cloud {
                api,
                base: text(config, "api_url", base)
                    .trim_end_matches('/')
                    .to_owned(),
                token,
                region: text(config, "region", region),
                image: text(config, "image", image),
                ssh_keys: config["ssh_keys"].as_array().cloned().unwrap_or_default(),
                billing,
                http: http.clone(),
                labels: labels.clone(),
            }))
        };
        match kind {
            "hetzner" => cloud(
                Api::Hetzner,
                "https://api.hetzner.cloud/v1",
                "fsn1",
                "ubuntu-24.04",
            ),
            "digitalocean" => cloud(
                Api::DigitalOcean,
                "https://api.digitalocean.com/v2",
                "fra1",
                "ubuntu-24-04-x64",
            ),
            "vultr" => cloud(Api::Vultr, "https://api.vultr.com/v2", "fra", "2284"),
            "docker" => {
                let types = match config["types"].as_array() {
                    Some(list) if !list.is_empty() => list
                        .iter()
                        .map(|t| serde_json::from_value(t.clone()))
                        .collect::<Result<Vec<MachineType>, _>>()
                        .map_err(|e| bad(format!("docker types: {e}")))?,
                    _ => hebra_heddle::Catalog::example().types,
                };
                let types = types
                    .into_iter()
                    .map(|mut t| {
                        for l in &labels {
                            if !t.labels.contains(l) {
                                t.labels.push(l.clone());
                            }
                        }
                        t
                    })
                    .collect();
                Ok(Self::Docker(Docker {
                    image: text(config, "image", "hebra-runner"),
                    types,
                }))
            }
            other => Err(bad(format!(
                "unknown provider `{other}` (use {})",
                KINDS.join(", ")
            ))),
        }
    }

    pub async fn catalog(&self) -> ApiResult<Vec<MachineType>> {
        match self {
            Self::Cloud(c) => c.catalog().await,
            Self::Docker(d) => Ok(d.types.clone()),
        }
    }

    pub async fn create(
        &self,
        kind: &MachineType,
        name: &str,
        tag: &str,
        boot: &Bootstrap<'_>,
    ) -> ApiResult<Created> {
        match self {
            Self::Cloud(c) => c.create(kind, name, tag, &boot.script()).await,
            Self::Docker(d) => d.create(kind, name, tag, boot).await,
        }
    }

    pub async fn delete(&self, provider_id: &str) -> ApiResult<()> {
        match self {
            Self::Cloud(c) => c.delete(provider_id).await,
            Self::Docker(_) => Docker::delete(provider_id).await,
        }
    }

    pub async fn list(&self, tag: &str) -> ApiResult<Vec<Listed>> {
        match self {
            Self::Cloud(c) => c.list(tag).await,
            Self::Docker(_) => Docker::list(tag).await,
        }
    }
}

// ------------------------------------------------------------- bootstrap

/// What a new machine needs to become a runner.
pub struct Bootstrap<'a> {
    /// The server as the machine reaches it.
    pub server: &'a str,
    pub token: &'a str,
    pub name: &'a str,
    pub labels: &'a [String],
    /// A specific release (`v0.3.0`), else the latest.
    pub version: Option<&'a str>,
    /// Also install Docker, for checks with an `image`.
    pub docker: bool,
}

/// Single quotes for a shell, with `'` escaped.
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

impl Bootstrap<'_> {
    /// The first-boot script (cloud-init runs it as root): install `hb`,
    /// register with the one-time token, run as a service.
    #[must_use]
    pub fn script(&self) -> String {
        let mut s =
            String::from("#!/bin/sh\nset -eu\nexport HOME=/root DEBIAN_FRONTEND=noninteractive\n");
        s.push_str("command -v curl >/dev/null || (apt-get update -y && apt-get install -y curl ca-certificates git)\n");
        s.push_str("command -v git >/dev/null || (apt-get update -y && apt-get install -y git)\n");
        if self.docker {
            s.push_str("command -v docker >/dev/null || curl -fsSL https://get.docker.com | sh\n");
        }
        if let Some(v) = self.version {
            s.push_str(&format!("export HEBRA_VERSION={}\n", quote(v)));
        }
        s.push_str(
            "curl -fsSL https://raw.githubusercontent.com/s-pl/hebra/main/scripts/install.sh | HEBRA_INSTALL_DIR=/usr/local/bin sh\n",
        );
        s.push_str("export HB_CONFIG_DIR=/etc/hebra\n");
        let mut register = format!(
            "/usr/local/bin/hb runner register --server {} --token {} --name {} --machine {} --install",
            quote(self.server),
            quote(self.token),
            quote(self.name),
            quote(self.name)
        );
        for l in self.labels {
            register.push_str(&format!(" --label {}", quote(l)));
        }
        s.push_str(&register);
        s.push('\n');
        s
    }
}

// ----------------------------------------------------------------- cloud

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(T[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn number(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

impl Cloud {
    fn name(&self) -> &'static str {
        match self.api {
            Api::Hetzner => "Hetzner Cloud",
            Api::DigitalOcean => "DigitalOcean",
            Api::Vultr => "Vultr",
        }
    }

    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> ApiResult<Value> {
        let mut req = self
            .http
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(&self.token);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| provider_error(self.name(), e))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| provider_error(self.name(), e))?;
        if !status.is_success() {
            let parsed: Value = serde_json::from_str(&text).unwrap_or_default();
            let message = parsed["error"]["message"]
                .as_str()
                .or_else(|| parsed["message"].as_str())
                .or_else(|| parsed["error"].as_str())
                .map_or_else(|| text.chars().take(300).collect(), str::to_owned);
            return Err(provider_error(
                self.name(),
                format!("HTTP {status}: {message}"),
            ));
        }
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    fn billing(&self, default: Billing) -> Billing {
        self.billing.unwrap_or(default)
    }

    async fn catalog(&self) -> ApiResult<Vec<MachineType>> {
        let mut out = Vec::new();
        let kind =
            |name: String, cpus: u32, memory_gb: u32, price: f64, billing: Billing, boot: f64| {
                MachineType {
                    name,
                    cpus,
                    memory_gb,
                    price_per_hour: price,
                    billing,
                    min_billed_secs: 60,
                    boot_secs: boot,
                    labels: self.labels.clone(),
                    max: None,
                }
            };
        match self.api {
            Api::Hetzner => {
                let v = self
                    .call(reqwest::Method::GET, "/server_types?per_page=50", None)
                    .await?;
                for t in v["server_types"].as_array().into_iter().flatten() {
                    if t["deprecated"].as_bool().unwrap_or(false) || t["deprecation"].is_object() {
                        continue;
                    }
                    let Some(price) = t["prices"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .find(|p| p["location"] == self.region.as_str())
                    else {
                        continue;
                    };
                    let Some(hourly) = number(&price["price_hourly"]["gross"]) else {
                        continue;
                    };
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let mut k = kind(
                        t["name"].as_str().unwrap_or_default().to_owned(),
                        u32::try_from(t["cores"].as_u64().unwrap_or(1)).unwrap_or(1),
                        number(&t["memory"]).unwrap_or(1.0).round() as u32,
                        hourly,
                        self.billing(Billing::Hour),
                        45.0,
                    );
                    if t["architecture"] == "arm" {
                        k.labels.push("arm64".to_owned());
                    }
                    out.push(k);
                }
            }
            Api::DigitalOcean => {
                let v = self
                    .call(reqwest::Method::GET, "/sizes?per_page=200", None)
                    .await?;
                for s in v["sizes"].as_array().into_iter().flatten() {
                    let here = s["regions"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|r| r == self.region.as_str());
                    if !s["available"].as_bool().unwrap_or(true) || !here {
                        continue;
                    }
                    let Some(hourly) = number(&s["price_hourly"]) else {
                        continue;
                    };
                    out.push(kind(
                        s["slug"].as_str().unwrap_or_default().to_owned(),
                        u32::try_from(s["vcpus"].as_u64().unwrap_or(1)).unwrap_or(1),
                        u32::try_from(s["memory"].as_u64().unwrap_or(1024) / 1024).unwrap_or(1),
                        hourly,
                        self.billing(Billing::Hour),
                        50.0,
                    ));
                }
            }
            Api::Vultr => {
                let v = self
                    .call(reqwest::Method::GET, "/plans?type=vc2&per_page=500", None)
                    .await?;
                for p in v["plans"].as_array().into_iter().flatten() {
                    let here = p["locations"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|r| r == self.region.as_str());
                    if !here {
                        continue;
                    }
                    let hourly = number(&p["hourly_cost"])
                        .or_else(|| number(&p["monthly_cost"]).map(|m| m / 730.0));
                    let Some(hourly) = hourly else { continue };
                    out.push(kind(
                        p["id"].as_str().unwrap_or_default().to_owned(),
                        u32::try_from(p["vcpu_count"].as_u64().unwrap_or(1)).unwrap_or(1),
                        u32::try_from(p["ram"].as_u64().unwrap_or(1024) / 1024).unwrap_or(1),
                        hourly,
                        self.billing(Billing::Hour),
                        60.0,
                    ));
                }
            }
        }
        if out.is_empty() {
            return Err(provider_error(
                self.name(),
                format!("no machine types in region `{}`", self.region),
            ));
        }
        out.sort_by(|a, b| a.price_per_hour.total_cmp(&b.price_per_hour));
        Ok(out)
    }

    async fn create(
        &self,
        kind: &MachineType,
        name: &str,
        tag: &str,
        script: &str,
    ) -> ApiResult<Created> {
        let tag_label = format!("hebra-fleet-{tag}");
        match self.api {
            Api::Hetzner => {
                let mut body = json!({
                    "name": name,
                    "server_type": kind.name,
                    "image": self.image,
                    "location": self.region,
                    "user_data": script,
                    "labels": { "hebra-fleet": tag },
                    "start_after_create": true,
                });
                if !self.ssh_keys.is_empty() {
                    body["ssh_keys"] = json!(self.ssh_keys);
                }
                let v = self
                    .call(reqwest::Method::POST, "/servers", Some(body))
                    .await?;
                Ok(Created {
                    provider_id: v["server"]["id"].to_string(),
                    address: v["server"]["public_net"]["ipv4"]["ip"]
                        .as_str()
                        .map(str::to_owned),
                })
            }
            Api::DigitalOcean => {
                let mut body = json!({
                    "name": name,
                    "region": self.region,
                    "size": kind.name,
                    "image": self.image,
                    "user_data": script,
                    "tags": [tag_label],
                });
                if !self.ssh_keys.is_empty() {
                    body["ssh_keys"] = json!(self.ssh_keys);
                }
                let v = self
                    .call(reqwest::Method::POST, "/droplets", Some(body))
                    .await?;
                Ok(Created {
                    provider_id: v["droplet"]["id"].to_string(),
                    address: None,
                })
            }
            Api::Vultr => {
                let os: Value = self
                    .image
                    .parse::<u64>()
                    .map_or_else(|_| json!(self.image), |n| json!(n));
                let mut body = json!({
                    "region": self.region,
                    "plan": kind.name,
                    "label": name,
                    "hostname": name,
                    "user_data": base64(script.as_bytes()),
                    "tags": [tag_label],
                });
                if os.is_number() {
                    body["os_id"] = os;
                } else {
                    body["image_id"] = os;
                }
                if !self.ssh_keys.is_empty() {
                    body["sshkey_id"] = json!(self.ssh_keys);
                }
                let v = self
                    .call(reqwest::Method::POST, "/instances", Some(body))
                    .await?;
                Ok(Created {
                    provider_id: v["instance"]["id"].as_str().unwrap_or_default().to_owned(),
                    address: v["instance"]["main_ip"]
                        .as_str()
                        .filter(|ip| *ip != "0.0.0.0")
                        .map(str::to_owned),
                })
            }
        }
    }

    async fn delete(&self, provider_id: &str) -> ApiResult<()> {
        let path = match self.api {
            Api::Hetzner => format!("/servers/{provider_id}"),
            Api::DigitalOcean => format!("/droplets/{provider_id}"),
            Api::Vultr => format!("/instances/{provider_id}"),
        };
        match self.call(reqwest::Method::DELETE, &path, None).await {
            // Already gone is what we wanted.
            Err(e) if e.message().contains("404") => Ok(()),
            other => other.map(|_| ()),
        }
    }

    async fn list(&self, tag: &str) -> ApiResult<Vec<Listed>> {
        let tag_label = format!("hebra-fleet-{tag}");
        let (path, key) = match self.api {
            Api::Hetzner => (
                format!("/servers?label_selector=hebra-fleet%3D{tag}&per_page=50"),
                "servers",
            ),
            Api::DigitalOcean => (
                format!("/droplets?tag_name={tag_label}&per_page=200"),
                "droplets",
            ),
            Api::Vultr => (
                format!("/instances?tag={tag_label}&per_page=500"),
                "instances",
            ),
        };
        let v = self.call(reqwest::Method::GET, &path, None).await?;
        Ok(v[key]
            .as_array()
            .into_iter()
            .flatten()
            .map(|m| {
                let id = m["id"]
                    .as_str()
                    .map_or_else(|| m["id"].to_string(), str::to_owned);
                let name = m["name"]
                    .as_str()
                    .or_else(|| m["label"].as_str())
                    .unwrap_or_default()
                    .to_owned();
                let running = match self.api {
                    Api::Hetzner => m["status"] == "running",
                    Api::DigitalOcean => m["status"] == "active",
                    Api::Vultr => m["status"] == "active" && m["power_status"] == "running",
                };
                Listed {
                    provider_id: id,
                    name,
                    running,
                }
            })
            .collect())
    }
}

// ---------------------------------------------------------------- docker

async fn docker(args: &[&str]) -> ApiResult<String> {
    let out = tokio::process::Command::new("docker")
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| provider_error("Docker", format!("cannot run docker: {e}")))?;
    if !out.status.success() {
        return Err(provider_error(
            "Docker",
            String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The server as a container on the same host reaches it.
#[must_use]
pub fn from_container(server: &str) -> String {
    server
        .replace("://localhost", "://host.docker.internal")
        .replace("://127.0.0.1", "://host.docker.internal")
        .replace("://0.0.0.0", "://host.docker.internal")
}

impl Docker {
    async fn create(
        &self,
        kind: &MachineType,
        name: &str,
        tag: &str,
        boot: &Bootstrap<'_>,
    ) -> ApiResult<Created> {
        let info = docker(&["info", "--format", "{{.NCPU}}"])
            .await
            .unwrap_or_default();
        let host_cpus: u32 = info.trim().parse().unwrap_or(2);
        let cpus = kind.cpus.clamp(1, host_cpus.max(1)).to_string();
        let memory = format!("{}g", kind.memory_gb.clamp(1, 16));
        let fleet = format!("hebra.fleet={tag}");
        let kind_label = format!("hebra.kind={}", kind.name);
        let labels = boot.labels.join(" ");
        let env = [
            format!("HEBRA_SERVER={}", from_container(boot.server)),
            format!("HB_RUNNER_TOKEN={}", boot.token),
            format!("HEBRA_RUNNER_NAME={name}"),
            format!("HEBRA_MACHINE={name}"),
            format!("HEBRA_LABELS={labels}"),
            format!("HEBRA_BOOT_DELAY={:.0}", kind.boot_secs.min(120.0)),
        ];
        let mut args = vec![
            "run",
            "-d",
            "--name",
            name,
            "--label",
            &fleet,
            "--label",
            &kind_label,
            "--cpus",
            &cpus,
            "--memory",
            &memory,
            "--add-host",
            "host.docker.internal:host-gateway",
        ];
        for e in &env {
            args.push("-e");
            args.push(e);
        }
        args.push(&self.image);
        let id = docker(&args).await?;
        Ok(Created {
            provider_id: id.trim().chars().take(12).collect(),
            address: None,
        })
    }

    async fn delete(provider_id: &str) -> ApiResult<()> {
        match docker(&["rm", "-f", provider_id]).await {
            Err(e) if e.message().contains("No such container") => Ok(()),
            other => other.map(|_| ()),
        }
    }

    async fn list(tag: &str) -> ApiResult<Vec<Listed>> {
        let filter = format!("label=hebra.fleet={tag}");
        let out = docker(&[
            "ps",
            "-a",
            "--filter",
            &filter,
            "--format",
            "{{.ID}} {{.Names}} {{.State}}",
        ])
        .await?;
        Ok(out
            .lines()
            .filter_map(|l| {
                let mut p = l.split_whitespace();
                Some(Listed {
                    provider_id: p.next()?.to_owned(),
                    name: p.next()?.to_owned(),
                    running: p.next() == Some("running"),
                })
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_scripts_quote_and_register() {
        let b = Bootstrap {
            server: "https://hebra.example.com",
            token: "hbr_x'y",
            name: "heddle-cx22-1",
            labels: &["linux".to_owned()],
            version: Some("v0.3.0"),
            docker: true,
        };
        let s = b.script();
        assert!(s.starts_with("#!/bin/sh"));
        assert!(s.contains("--token 'hbr_x'\\''y'"));
        assert!(s.contains("--install"));
        assert!(s.contains("get.docker.com"));
        assert!(s.contains("HEBRA_VERSION='v0.3.0'"));
    }

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn unknown_providers_and_missing_tokens_are_refused() {
        let http = reqwest::Client::new();
        assert!(Provider::new("aws", &json!({}), None, http.clone()).is_err());
        assert!(Provider::new("hetzner", &json!({}), None, http.clone()).is_err());
        assert!(Provider::new("docker", &json!({}), None, http).is_ok());
    }
}
