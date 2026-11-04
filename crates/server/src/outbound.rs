//! Requests the server makes to addresses people give it (webhooks, a cloud
//! provider's API): only to the public internet, unless the operator allows
//! private networks (`HEBRA_ALLOW_PRIVATE_URLS`).
//!
//! Without this, anyone who can add a webhook could make the server call
//! itself, its database, or a cloud metadata endpoint (169.254.169.254) and
//! read the answer in the delivery log. Names are checked when they are
//! resolved, at connection time, so a name that later points somewhere
//! private (DNS rebinding) is refused too. Redirects are not followed.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

use crate::error::{ApiError, ApiResult};

/// Whether an address is on the public internet.
#[must_use]
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => public_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => public_v4(v4),
            None => public_v6(v6),
        },
    }
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_unspecified()
        || ip.is_multicast()
        // 0.0.0.0/8, carrier-grade NAT (100.64/10), 192.0.0.0/24, benchmarks (198.18/15), reserved (240/4)
        || a == 0
        || (a == 100 && (64..128).contains(&b))
        || (a == 192 && b == 0 && ip.octets()[2] == 0)
        || (a == 198 && (18..20).contains(&b))
        || a >= 240)
}

fn public_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        // unique local fc00::/7, link-local fe80::/10, documentation 2001:db8::/32
        || (first & 0xfe00) == 0xfc00
        || (first & 0xffc0) == 0xfe80
        || (first == 0x2001 && ip.segments()[1] == 0x0db8))
}

/// Resolves names, keeping only public addresses.
struct PublicOnly;

impl Resolve for PublicOnly {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let found: Vec<SocketAddr> =
                tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            let public: Vec<SocketAddr> = found.into_iter().filter(|a| is_public(a.ip())).collect();
            if public.is_empty() {
                return Err(format!("{host} is not on the public internet").into());
            }
            let addrs: Addrs = Box::new(public.into_iter());
            Ok(addrs)
        })
    }
}

/// The client for requests to addresses people give.
pub fn client(allow_private: bool) -> anyhow::Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("hebra-server/", env!("CARGO_PKG_VERSION")));
    if !allow_private {
        builder = builder.dns_resolver(PublicOnly);
    }
    Ok(builder.build()?)
}

/// Checks an address before it is stored: `http(s)://`, and not a private
/// network by its literal host (names are checked again on each request).
pub fn check_url(url: &str, allow_private: bool) -> ApiResult<reqwest::Url> {
    let parsed =
        reqwest::Url::parse(url).map_err(|e| ApiError::bad(format!("not an address: {e}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(ApiError::bad("the address starts with https:// or http://"));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(ApiError::bad(
            "put credentials in the secret, not in the address",
        ));
    }
    if allow_private {
        return Ok(parsed);
    }
    let host = parsed.host_str().unwrap_or_default();
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    let private = match bare.parse::<IpAddr>() {
        Ok(ip) => !is_public(ip),
        Err(_) => {
            let d = bare.trim_end_matches('.').to_ascii_lowercase();
            d.is_empty()
                || d == "localhost"
                || d.ends_with(".localhost")
                || d.ends_with(".internal")
                || d.ends_with(".local")
        }
    };
    if private {
        return Err(ApiError::bad(
            "that address is on a private network; the server only calls the public internet (its operator can allow private networks with HEBRA_ALLOW_PRIVATE_URLS)",
        ));
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_networks_are_refused() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["1.1.1.1", "8.8.8.8", "2606:4700::1111", "100.128.0.1"] {
            assert!(is_public(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn addresses_are_checked_before_they_are_stored() {
        assert!(check_url("https://hooks.example.com/x", false).is_ok());
        for bad in [
            "http://127.0.0.1:8080/hook",
            "http://[::1]/",
            "http://169.254.169.254/latest/meta-data",
            "http://localhost/hook",
            "http://db.internal/",
            "ftp://example.com/",
            "https://user:pass@example.com/",
        ] {
            assert!(check_url(bad, false).is_err(), "{bad}");
        }
        assert!(check_url("http://127.0.0.1:8080/hook", true).is_ok());
    }
}
