//! A small JSON client for server-wide API calls (sign-in, mandates,
//! organizations, runners), which are not tied to one repository.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;

pub struct Client {
    pub base: String,
    token: Option<String>,
    agent: ureq::Agent,
}

impl Client {
    pub fn new(base: &str, token: Option<String>) -> Self {
        Self {
            base: base.trim_end_matches('/').to_owned(),
            token,
            agent: ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(120)))
                .http_status_as_error(false)
                .build()
                .into(),
        }
    }

    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    fn url(&self, path: &str) -> String {
        format!("{}/api/v1{path}", self.base)
    }

    fn finish(&self, mut resp: ureq::http::Response<ureq::Body>) -> Result<Value> {
        let status = resp.status();
        let body = resp
            .body_mut()
            .with_config()
            .limit(64 * 1024 * 1024)
            .read_to_vec()
            .context("reading the server response")?;
        if status.is_success() {
            return Ok(serde_json::from_slice(&body).unwrap_or(Value::Null));
        }
        let message = serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
            .unwrap_or_else(|| format!("HTTP {status}"));
        bail!("{} ({}): {message}", self.base, status.as_u16())
    }

    pub fn get(&self, path: &str) -> Result<Value> {
        let mut req = self.agent.get(self.url(path));
        if let Some(t) = &self.token {
            req = req.header("Authorization", format!("Bearer {t}"));
        }
        let resp = req
            .call()
            .with_context(|| format!("connecting to {}", self.base))?;
        self.finish(resp)
    }

    pub fn send(&self, method: &str, path: &str, body: &Value) -> Result<Value> {
        let url = self.url(path);
        let mut req = match method {
            "POST" => self.agent.post(&url),
            "PUT" => self.agent.put(&url),
            "PATCH" => self.agent.patch(&url),
            "DELETE" => {
                let mut del = self.agent.delete(&url);
                if let Some(t) = &self.token {
                    del = del.header("Authorization", format!("Bearer {t}"));
                }
                let resp = del
                    .call()
                    .with_context(|| format!("connecting to {}", self.base))?;
                return self.finish(resp);
            }
            other => bail!("unsupported method {other}"),
        }
        .header("Content-Type", "application/json");
        if let Some(t) = &self.token {
            req = req.header("Authorization", format!("Bearer {t}"));
        }
        let resp = req
            .send(body.to_string().as_bytes())
            .with_context(|| format!("connecting to {}", self.base))?;
        self.finish(resp)
    }

    /// POSTs JSON and returns the raw body (packs).
    pub fn post_bytes(&self, path: &str, body: &Value) -> Result<Vec<u8>> {
        let mut req = self
            .agent
            .post(self.url(path))
            .header("Content-Type", "application/json");
        if let Some(t) = &self.token {
            req = req.header("Authorization", format!("Bearer {t}"));
        }
        let mut resp = req
            .send(body.to_string().as_bytes())
            .with_context(|| format!("connecting to {}", self.base))?;
        if !resp.status().is_success() {
            return self.finish(resp).map(|_| Vec::new());
        }
        Ok(resp
            .body_mut()
            .with_config()
            .limit(4 * 1024 * 1024 * 1024)
            .read_to_vec()?)
    }

    pub fn post(&self, path: &str, body: &Value) -> Result<Value> {
        self.send("POST", path, body)
    }
}
