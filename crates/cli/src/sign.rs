//! Signing changes and tags with the tools people already have: GnuPG, or an
//! SSH key (`gpg.format = ssh`), the same settings Git reads.

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow, bail};
use hebra_core::Workspace;
use serde_json::json;

use crate::cli::SignArgs;
use crate::commands::Output;

const NAMESPACE: &str = "git";

/// How to produce signatures here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signer {
    ssh: bool,
    key: Option<String>,
}

fn setting(ws: &Workspace, key: &str) -> Option<String> {
    ws.config_get(key)
        .ok()
        .flatten()
        .or_else(|| crate::settings::get(key))
}

/// The signer to use, if `--gpg-sign` or `commit.gpgsign` asks for one.
pub fn wanted(ws: &Workspace, args: &SignArgs) -> Option<Signer> {
    if args.no_gpg_sign {
        return None;
    }
    let Some(key) = &args.gpg_sign else {
        return (setting(ws, "commit.gpgsign").as_deref() == Some("true")).then(|| signer(ws));
    };
    let mut signer = signer(ws);
    if key.is_some() {
        signer.key.clone_from(key);
    }
    Some(signer)
}

fn signer(ws: &Workspace) -> Signer {
    Signer {
        ssh: setting(ws, "gpg.format").as_deref() == Some("ssh"),
        key: setting(ws, "user.signingkey"),
    }
}

fn pipe(mut command: Command, input: &[u8]) -> Result<std::process::Output> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting {:?}", command.get_program()))?;
    child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("no stdin"))?
        .write_all(input)?;
    Ok(child.wait_with_output()?)
}

impl Signer {
    pub fn sign(&self, payload: &[u8]) -> Result<String> {
        let out = if self.ssh {
            let key = self
                .key
                .as_deref()
                .ok_or_else(|| anyhow!("set user.signingkey to your SSH key"))?;
            let mut command = Command::new("ssh-keygen");
            command.args(["-Y", "sign", "-n", NAMESPACE, "-f", key]);
            pipe(command, payload)?
        } else {
            let mut command = Command::new("gpg");
            command.args(["--batch", "--armor", "--detach-sign"]);
            if let Some(key) = &self.key {
                command.args(["--local-user", key]);
            }
            pipe(command, payload)?
        };
        if !out.status.success() {
            bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    }
}

pub fn signer_for_tags(ws: &Workspace, requested: bool) -> Option<Signer> {
    requested.then(|| signer(ws))
}

/// Checks a detached signature with `gpg --verify`, or `ssh-keygen -Y check-novalidate`
/// / `verify` when `gpg.ssh.allowedSignersFile` is set.
pub fn verify(ws: &Workspace, payload: &[u8], signature: &str) -> Result<Output> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("signature");
    std::fs::write(&file, signature)?;
    let out = if signature.contains("BEGIN SSH SIGNATURE") {
        let mut command = Command::new("ssh-keygen");
        match setting(ws, "gpg.ssh.allowedsignersfile") {
            Some(allowed) => {
                let identity = setting(ws, "user.email").unwrap_or_default();
                command.args([
                    "-Y", "verify", "-n", NAMESPACE, "-f", &allowed, "-I", &identity,
                ]);
            }
            None => {
                command.args(["-Y", "check-novalidate", "-n", NAMESPACE]);
            }
        }
        command.arg("-s").arg(&file);
        pipe(command, payload)?
    } else {
        let data = dir.path().join("payload");
        std::fs::write(&data, payload)?;
        let mut command = Command::new("gpg");
        command.args(["--batch", "--verify"]).arg(&file).arg(&data);
        command.stdin(Stdio::null());
        command.output().context("starting gpg")?
    };
    let detail = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(Output::with_exit(
        json!({ "valid": out.status.success(), "detail": detail.trim() }),
        detail,
        u8::from(!out.status.success()),
    ))
}
