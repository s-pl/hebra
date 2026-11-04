//! What the package registries share: who is calling (Bearer like the API,
//! or Basic, as `docker login` sends it, with a mandate as the password),
//! and who may read and publish an owner's packages.

use std::sync::Arc;

use axum::http::{HeaderMap, HeaderValue, header};

use crate::db::{Account, OrgRole, Package};
use crate::error::ApiResult;
use crate::identity::Principal;
use crate::server::Server;

fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0;
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\n' | b'\r' => continue,
            _ => return None,
        };
        buf = (buf << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((buf >> bits) & 0xff).ok()?);
        }
    }
    Some(out)
}

pub(crate) fn base64_bytes(text: &str) -> Option<Vec<u8>> {
    base64_decode(text)
}

/// The caller of a registry request. `Basic user:mandate` counts as
/// `Bearer mandate`.
pub(crate) async fn caller(s: &Server, headers: &HeaderMap) -> ApiResult<Option<Arc<Principal>>> {
    let basic = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Basic "))
        .and_then(|b| base64_decode(b.trim()))
        .and_then(|raw| String::from_utf8(raw).ok())
        .and_then(|pair| pair.split_once(':').map(|(_, p)| p.to_owned()));
    match basic {
        Some(password) => {
            let mut fixed = HeaderMap::new();
            if let Ok(v) = HeaderValue::from_str(&format!("Bearer {password}")) {
                fixed.insert(header::AUTHORIZATION, v);
            }
            s.authenticate(&fixed).await
        }
        None => s.authenticate(headers).await,
    }
}

/// People of the owner, its agents and runners, and members of an
/// organization publish its packages.
pub(crate) async fn may_publish(
    s: &Server,
    who: Option<&Principal>,
    owner: &Account,
) -> ApiResult<bool> {
    let Some(who) = who else { return Ok(false) };
    if who.account.site_admin || who.account.id == owner.id {
        return Ok(true);
    }
    if who.owner.as_ref().is_some_and(|o| o.id == owner.id) {
        return Ok(true);
    }
    Ok(matches!(
        s.db.org_role(&owner.id, &who.account.id).await?,
        Some(OrgRole::Owner | OrgRole::Admin | OrgRole::Member)
    ))
}

/// Anyone reads public packages; private ones, whoever may publish them.
pub(crate) async fn may_read(
    s: &Server,
    who: Option<&Principal>,
    owner: &Account,
    package: Option<&Package>,
) -> ApiResult<bool> {
    if package.is_some_and(|p| p.public) {
        return Ok(true);
    }
    may_publish(s, who, owner).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_credentials_decode() {
        assert_eq!(
            base64_decode("dXNlcjpoYm1fdG9rZW4=").unwrap(),
            b"user:hbm_token"
        );
        assert_eq!(base64_decode("Zm9v").unwrap(), b"foo");
        assert!(base64_decode("@@").is_none());
    }
}
