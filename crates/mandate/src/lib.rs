//! Mandates: Hebra's credentials for people and agents.
//!
//! A mandate is a [Biscuit](https://www.biscuitsec.org) token. The server
//! signs a *root mandate* for an account (`account("acct_…")`); whoever holds
//! it can append blocks that only take rights away — limit it to some
//! repositories, operations, bookmarks, or a deadline — and hand the result to
//! an agent, offline. Nobody can add rights: every block's checks must pass.
//!
//! On each request the server states what is happening as facts
//! (`repo`, `operation`, `bookmark`, `time`) and authorizes the token
//! against them; see [`authorize`].
//!
//! The text form is `hbm_` followed by the token in URL-safe base64.

use std::collections::{BTreeSet, HashMap};

use biscuit_auth::builder::Term;
use biscuit_auth::{
    AuthorizerBuilder, Biscuit, BlockBuilder, KeyPair, PrivateKey, PublicKey, UnverifiedBiscuit,
};
use serde::{Deserialize, Serialize};

pub use biscuit_auth::Algorithm;

/// Prefix of a mandate in text form.
pub const PREFIX: &str = "hbm_";

/// Operations a mandate can be limited to.
pub mod op {
    /// Read code, history, reviews, tasks.
    pub const READ: &str = "read";
    /// Send objects and move bookmarks.
    pub const PUSH: &str = "push";
    /// Open reviews, comment on them.
    pub const REVIEW: &str = "review";
    /// Approve and merge reviews.
    pub const MERGE: &str = "merge";
    /// Create, claim and close tasks.
    pub const TASK: &str = "task";
    /// Repository settings, access, runners, check approval.
    pub const ADMIN: &str = "admin";
    /// Run checks (runners).
    pub const RUN: &str = "run";

    pub const ALL: [&str; 7] = [READ, PUSH, REVIEW, MERGE, TASK, ADMIN, RUN];
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("this is not a Hebra mandate")]
    NotAMandate,
    #[error("invalid mandate: {0}")]
    Invalid(String),
    #[error("{0}")]
    Restriction(String),
}

impl From<biscuit_auth::error::Token> for Error {
    fn from(e: biscuit_auth::error::Token) -> Self {
        Self::Invalid(e.to_string())
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Who a delegated mandate was handed to and why. Informational: it never
/// grants anything, it labels the work for provenance and audit.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Delegation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

/// What an attenuation block allows. Empty lists mean "no limit on this".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Restrictions {
    /// `owner/name`.
    pub repos: Vec<String>,
    /// From [`op`].
    pub operations: Vec<String>,
    /// Bookmark patterns: exact names, or a prefix ending in `*`.
    pub bookmarks: Vec<String>,
    /// Unix seconds.
    pub expires_at: Option<i64>,
    pub delegation: Delegation,
}

/// The server's signing key.
pub struct Authority {
    keys: KeyPair,
}

impl Authority {
    /// A new random key (64 hex characters with [`Authority::to_hex`]).
    #[must_use]
    pub fn generate() -> Self {
        Self {
            keys: KeyPair::new(),
        }
    }

    pub fn from_hex(hex: &str) -> Result<Self> {
        let private = PrivateKey::from_bytes_hex(hex.trim(), Algorithm::Ed25519)
            .map_err(|e| Error::Invalid(format!("signing key: {e}")))?;
        Ok(Self {
            keys: KeyPair::from(&private),
        })
    }

    #[must_use]
    pub fn to_hex(&self) -> String {
        self.keys.private().to_bytes_hex()
    }

    #[must_use]
    pub fn public(&self) -> PublicKey {
        self.keys.public()
    }

    /// Signs a root mandate for `account_id`, valid until `expires_at` (unix
    /// seconds) if given. Returns the token and its root id (for revocation).
    pub fn issue(&self, account_id: &str, expires_at: Option<i64>) -> Result<(String, String)> {
        let mut params = HashMap::new();
        params.insert("account".to_owned(), Term::Str(account_id.to_owned()));
        let mut code = String::from("account({account});");
        if let Some(exp) = expires_at {
            params.insert("exp".to_owned(), date(exp)?);
            code.push_str(" check if time($t), $t < {exp};");
        }
        let token = Biscuit::builder()
            .code_with_params(code, params, HashMap::new())?
            .build(&self.keys)?;
        let root = revocation_ids(&token)
            .into_iter()
            .next()
            .ok_or_else(|| Error::Invalid("token without blocks".to_owned()))?;
        Ok((encode(&token)?, root))
    }

    /// Checks the signature chain and returns the token.
    pub fn verify(&self, text: &str) -> Result<Verified> {
        let raw = text.strip_prefix(PREFIX).ok_or(Error::NotAMandate)?;
        let token = Biscuit::from_base64(raw, self.public())?;
        let mut authorizer = AuthorizerBuilder::new().build(&token)?;
        let accounts: Vec<(String,)> = authorizer.query("data($id) <- account($id)")?;
        let [(account_id,)] = accounts.as_slice() else {
            return Err(Error::Invalid(
                "a mandate names exactly one account".to_owned(),
            ));
        };
        let delegation = token
            .context()
            .iter()
            .flatten()
            .filter_map(|c| serde_json::from_str::<Delegation>(c).ok())
            .reduce(|outer, inner| Delegation {
                agent: inner.agent.or(outer.agent),
                purpose: inner.purpose.or(outer.purpose),
                session: inner.session.or(outer.session),
            });
        Ok(Verified {
            account_id: account_id.clone(),
            revocation_ids: revocation_ids(&token),
            delegated: token.block_count() > 1,
            delegation: delegation.unwrap_or_default(),
            token,
        })
    }
}

/// A mandate whose signatures are valid. Whether it allows a given request
/// is [`authorize`]'s job.
pub struct Verified {
    pub account_id: String,
    /// Hex ids of every block, root first; revoking any of them revokes this
    /// mandate.
    pub revocation_ids: Vec<String>,
    /// True if someone attenuated it (the holder is acting for someone else).
    pub delegated: bool,
    pub delegation: Delegation,
    token: Biscuit,
}

/// What is being done, as the server states it.
#[derive(Debug, Clone)]
pub struct Request<'a> {
    pub repo: Option<&'a str>,
    pub operation: &'a str,
    pub bookmark: Option<&'a str>,
    /// Unix seconds.
    pub now: i64,
}

/// Runs every check of every block against the request.
pub fn authorize(mandate: &Verified, request: &Request<'_>) -> Result<()> {
    let mut params = HashMap::new();
    params.insert("op".to_owned(), Term::Str(request.operation.to_owned()));
    params.insert("now".to_owned(), date(request.now)?);
    let mut code = String::from("operation({op}); time({now});");
    match request.repo {
        Some(repo) => {
            params.insert("repo".to_owned(), Term::Str(repo.to_owned()));
            code.push_str(" repo({repo});");
        }
        None => code.push_str(" no_repo(true);"),
    }
    match request.bookmark {
        Some(b) => {
            params.insert("bookmark".to_owned(), Term::Str(b.to_owned()));
            code.push_str(" bookmark({bookmark});");
        }
        None => code.push_str(" no_bookmark(true);"),
    }
    code.push_str(" allow if true;");
    let mut authorizer = AuthorizerBuilder::new()
        .code_with_params(code, params, HashMap::new())?
        .build(&mandate.token)?;
    authorizer
        .authorize()
        .map(|_| ())
        .map_err(|e| Error::Restriction(format!("the mandate does not allow this ({e})")))
}

/// The block that applies `r` to a mandate.
fn block(r: &Restrictions) -> Result<BlockBuilder> {
    let mut params = HashMap::new();
    let mut code = String::new();
    if !r.repos.is_empty() {
        params.insert("repos".to_owned(), set(&r.repos));
        code.push_str("check if no_repo(true) or repo($r), {repos}.contains($r);\n");
    }
    if !r.operations.is_empty() {
        for o in &r.operations {
            if !op::ALL.contains(&o.as_str()) {
                return Err(Error::Restriction(format!(
                    "unknown operation `{o}` (use {})",
                    op::ALL.join(", ")
                )));
            }
        }
        params.insert("ops".to_owned(), set(&r.operations));
        code.push_str("check if operation($op), {ops}.contains($op);\n");
    }
    if !r.bookmarks.is_empty() {
        let mut alternatives = vec!["no_bookmark(true)".to_owned()];
        for (i, pattern) in r.bookmarks.iter().enumerate() {
            let key = format!("b{i}");
            if let Some(prefix) = pattern.strip_suffix('*') {
                params.insert(key.clone(), Term::Str(prefix.to_owned()));
                alternatives.push(format!("bookmark($b), $b.starts_with({{{key}}})"));
            } else {
                params.insert(key.clone(), Term::Str(pattern.clone()));
                alternatives.push(format!("bookmark($b), $b == {{{key}}}"));
            }
        }
        code.push_str(&format!("check if {};\n", alternatives.join(" or ")));
    }
    if let Some(exp) = r.expires_at {
        params.insert("exp".to_owned(), date(exp)?);
        code.push_str("check if time($t), $t < {exp};\n");
    }
    let mut builder = BlockBuilder::new().code_with_params(code, params, HashMap::new())?;
    if r.delegation != Delegation::default() {
        builder = builder.context(
            serde_json::to_string(&r.delegation).map_err(|e| Error::Invalid(e.to_string()))?,
        );
    }
    Ok(builder)
}

/// Narrows a mandate, offline: the result can do at most what both the
/// original and `r` allow. Used by `hb mandate grant` to hand work to agents.
pub fn attenuate(text: &str, r: &Restrictions) -> Result<String> {
    let raw = text.strip_prefix(PREFIX).ok_or(Error::NotAMandate)?;
    let token = UnverifiedBiscuit::from_base64(raw)?.append(block(r)?)?;
    Ok(format!("{PREFIX}{}", token.to_base64()?))
}

/// The root id of a mandate (to revoke it), read without verifying it.
pub fn root_id(text: &str) -> Result<String> {
    let raw = text.strip_prefix(PREFIX).ok_or(Error::NotAMandate)?;
    let token = UnverifiedBiscuit::from_base64(raw)?;
    token
        .revocation_identifiers()
        .first()
        .map(hex::encode)
        .ok_or_else(|| Error::Invalid("token without blocks".to_owned()))
}

fn encode(token: &Biscuit) -> Result<String> {
    Ok(format!("{PREFIX}{}", token.to_base64()?))
}

fn revocation_ids(token: &Biscuit) -> Vec<String> {
    token
        .revocation_identifiers()
        .iter()
        .map(hex::encode)
        .collect()
}

fn set(items: &[String]) -> Term {
    Term::Set(
        items
            .iter()
            .cloned()
            .map(Term::Str)
            .collect::<BTreeSet<_>>(),
    )
}

fn date(unix: i64) -> Result<Term> {
    u64::try_from(unix)
        .map(Term::Date)
        .map_err(|_| Error::Invalid("time before 1970".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn req<'a>(repo: &'a str, operation: &'a str, bookmark: Option<&'a str>) -> Request<'a> {
        Request {
            repo: Some(repo),
            operation,
            bookmark,
            now: NOW,
        }
    }

    #[test]
    fn root_mandate_allows_everything_until_it_expires() {
        let authority = Authority::generate();
        let (token, root) = authority.issue("acct_ana", Some(NOW + 60)).unwrap();
        let v = authority.verify(&token).unwrap();
        assert_eq!(v.account_id, "acct_ana");
        assert!(!v.delegated);
        assert_eq!(v.revocation_ids[0], root);
        assert!(authorize(&v, &req("acme/app", op::PUSH, Some("main"))).is_ok());
        let late = Request {
            now: NOW + 61,
            ..req("acme/app", op::READ, None)
        };
        assert!(authorize(&v, &late).is_err());
    }

    #[test]
    fn attenuation_only_narrows() {
        let authority = Authority::generate();
        let (token, _) = authority.issue("acct_ana", None).unwrap();
        let agent = attenuate(
            &token,
            &Restrictions {
                repos: vec!["acme/app".into()],
                operations: vec![op::READ.into(), op::PUSH.into(), op::REVIEW.into()],
                bookmarks: vec!["agents/claude/*".into()],
                expires_at: Some(NOW + 3600),
                delegation: Delegation {
                    agent: Some("claude-code".into()),
                    purpose: Some("Add Google sign-in".into()),
                    session: None,
                },
            },
        )
        .unwrap();
        let v = authority.verify(&agent).unwrap();
        assert!(v.delegated);
        assert_eq!(v.account_id, "acct_ana");
        assert_eq!(v.delegation.agent.as_deref(), Some("claude-code"));
        assert!(authorize(&v, &req("acme/app", op::PUSH, Some("agents/claude/oauth"))).is_ok());
        assert!(authorize(&v, &req("acme/app", op::READ, None)).is_ok());
        assert!(authorize(&v, &req("acme/app", op::PUSH, Some("main"))).is_err());
        assert!(authorize(&v, &req("acme/other", op::READ, None)).is_err());
        assert!(authorize(&v, &req("acme/app", op::MERGE, Some("main"))).is_err());

        // A sub-agent can narrow further but never widen.
        let sub = attenuate(
            &agent,
            &Restrictions {
                operations: vec![op::READ.into(), op::MERGE.into()],
                ..Restrictions::default()
            },
        )
        .unwrap();
        let v = authority.verify(&sub).unwrap();
        assert!(authorize(&v, &req("acme/app", op::READ, None)).is_ok());
        assert!(authorize(&v, &req("acme/app", op::MERGE, Some("agents/claude/x"))).is_err());
        assert!(authorize(&v, &req("acme/app", op::PUSH, Some("agents/claude/x"))).is_err());
    }

    #[test]
    fn forged_or_foreign_tokens_are_rejected() {
        let ours = Authority::generate();
        let theirs = Authority::generate();
        let (token, _) = theirs.issue("acct_eve", None).unwrap();
        assert!(ours.verify(&token).is_err());
        assert!(ours.verify("hbt_0123").is_err());
        let (mut good, _) = ours.issue("acct_ana", None).unwrap();
        good.push('A');
        assert!(ours.verify(&good).is_err());
    }

    #[test]
    fn keys_round_trip() {
        let a = Authority::generate();
        let b = Authority::from_hex(&a.to_hex()).unwrap();
        let (token, _) = a.issue("acct_ana", None).unwrap();
        assert!(b.verify(&token).is_ok());
    }

    #[test]
    fn root_id_matches_issue() {
        let a = Authority::generate();
        let (token, root) = a.issue("acct_ana", None).unwrap();
        assert_eq!(root_id(&token).unwrap(), root);
        let narrowed = attenuate(&token, &Restrictions::default()).unwrap();
        assert_eq!(root_id(&narrowed).unwrap(), root);
    }
}
