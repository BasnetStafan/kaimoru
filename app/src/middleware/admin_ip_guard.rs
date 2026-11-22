//! Admin IP allowlist guard.
//!
//! The `/admin` surface answers only requests whose source IP
//! is on `admin_allowed_ips` in the config (exact addresses or
//! CIDR ranges). Everything else gets a plain 404, so a refused
//! source learns nothing about whether an admin exists. The
//! guard is the outermost layer on the admin router: a refused
//! request costs no session or database work.
//!
//! Which IP counts (the effective IP) depends on how the
//! request reached the app:
//!
//! - Through the Cloudflare tunnel, cloudflared runs on this
//!   host and connects from loopback, so the peer alone cannot
//!   tell visitors apart. Cloudflare's edge sets
//!   `CF-Connecting-IP` to the true visitor address, and the
//!   header is honoured ONLY from a loopback peer — the one
//!   case where the process presenting it is the tunnel
//!   itself. From any other peer the header is ignored: on a
//!   directly reachable deployment (Caddy, an ALB) it is just
//!   a string the visitor typed, and judging them by it would
//!   let anyone claim an allowlisted address.
//! - Otherwise the shared [`crate::session::client_ip`] rule
//!   applies: the first `X-Forwarded-For` entry when
//!   `TRUST_PROXY_HEADERS=1` (the AWS deployment, where the ALB
//!   is the sole ingress), else the direct peer.
//!
//! An unparseable source is refused (fail closed). Entries are
//! parsed once at router assembly; an invalid config entry
//! stops the boot with a clear message rather than silently
//! widening or narrowing the list.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::config::CONFIG;
use crate::session::client_ip;

/// One parsed allowlist entry: a network address and a prefix
/// length. An exact address is stored as a full-length prefix
/// (32 or 128), so matching is one masked comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Entry {
    V4 { net: u32, prefix: u8 },
    V6 { net: u128, prefix: u8 },
}

impl Entry {
    /// Parses `addr` or `addr/prefix` (for example `203.0.113.7`
    /// or `203.0.113.0/24`). Anything else is an error naming
    /// the offending entry.
    fn parse(raw: &str) -> Result<Entry, String> {
        let raw = raw.trim();
        let (addr_raw, prefix_raw) = match raw.split_once('/') {
            Some((addr, prefix)) => (addr, Some(prefix)),
            None => (raw, None),
        };
        let addr: IpAddr = addr_raw
            .trim()
            .parse()
            .map_err(|_| format!("invalid admin_allowed_ips entry: {raw:?}"))?;
        let prefix = match prefix_raw {
            Some(p) => Some(
                p.trim()
                    .parse::<u8>()
                    .map_err(|_| format!("invalid admin_allowed_ips entry: {raw:?}"))?,
            ),
            None => None,
        };
        match addr {
            IpAddr::V4(a) => {
                let prefix = prefix.unwrap_or(32);
                if prefix > 32 {
                    return Err(format!("invalid admin_allowed_ips entry: {raw:?}"));
                }
                Ok(Entry::V4 {
                    net: u32::from(a),
                    prefix,
                })
            }
            IpAddr::V6(a) => {
                let prefix = prefix.unwrap_or(128);
                if prefix > 128 {
                    return Err(format!("invalid admin_allowed_ips entry: {raw:?}"));
                }
                Ok(Entry::V6 {
                    net: u128::from(a),
                    prefix,
                })
            }
        }
    }

    /// Whether `ip` falls inside this entry. Address families
    /// never cross-match.
    fn matches(&self, ip: &IpAddr) -> bool {
        match (self, ip) {
            (Entry::V4 { net, prefix }, IpAddr::V4(a)) => {
                masked_eq(u32::from(*a) as u128, *net as u128, *prefix, 32)
            }
            (Entry::V6 { net, prefix }, IpAddr::V6(a)) => {
                masked_eq(u128::from(*a), *net, *prefix, 128)
            }
            _ => false,
        }
    }
}

/// Compares the high `prefix` bits of two addresses held as
/// u128 (IPv4 values are zero-extended, with `bits` = 32).
fn masked_eq(ip: u128, net: u128, prefix: u8, bits: u8) -> bool {
    if prefix == 0 {
        return true;
    }
    let shift = bits - prefix;
    (ip >> shift) == (net >> shift)
}

/// The parsed allowlist, built once and shared by every admin
/// request.
#[derive(Debug, Clone)]
pub struct AdminAllowlist(Arc<Vec<Entry>>);

impl AdminAllowlist {
    /// Whether `ip` is on the list.
    fn allows(&self, ip: &IpAddr) -> bool {
        self.0.iter().any(|entry| entry.matches(ip))
    }
}

/// Builds the allowlist from the typed config. Panics on an
/// invalid entry: a typo must stop the boot, never silently
/// change who can reach the back office.
pub fn from_config() -> AdminAllowlist {
    let entries = CONFIG
        .admin_allowed_ips
        .iter()
        .map(|raw| Entry::parse(raw).unwrap_or_else(|e| panic!("config error: {e}")))
        .collect();
    AdminAllowlist(Arc::new(entries))
}

/// Resolves the effective client IP under the rules in the
/// module docs. `None` when no usable address exists.
fn effective_ip(peer: &SocketAddr, headers: &HeaderMap) -> Option<IpAddr> {
    if peer.ip().is_loopback()
        && let Some(cf) = headers
            .get("cf-connecting-ip")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<IpAddr>().ok())
    {
        return Some(cf);
    }
    client_ip(peer, headers).parse().ok()
}

/// The admin router's outermost middleware: requests from a
/// source outside the allowlist are answered 404 and logged;
/// allowed requests pass through untouched.
pub async fn require_allowed_ip(
    State(allow): State<AdminAllowlist>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path().to_string();
    match effective_ip(&peer, req.headers()) {
        Some(ip) if allow.allows(&ip) => next.run(req).await,
        Some(ip) => {
            tracing::warn!("admin refused: source {ip} (peer {peer}) path {path}");
            (StatusCode::NOT_FOUND, "Not Found").into_response()
        }
        None => {
            tracing::warn!("admin refused: unresolvable source (peer {peer}) path {path}");
            (StatusCode::NOT_FOUND, "Not Found").into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn exact_v4_matches_only_itself() {
        let e = Entry::parse("203.0.113.7").unwrap();
        assert!(e.matches(&ip("203.0.113.7")));
        assert!(!e.matches(&ip("203.0.113.8")));
    }

    #[test]
    fn cidr_v4_matches_the_whole_range() {
        let e = Entry::parse("203.0.113.0/24").unwrap();
        assert!(e.matches(&ip("203.0.113.0")));
        assert!(e.matches(&ip("203.0.113.255")));
        assert!(!e.matches(&ip("203.0.114.1")));
    }

    #[test]
    fn prefix_edges() {
        assert!(
            Entry::parse("203.0.113.7/32")
                .unwrap()
                .matches(&ip("203.0.113.7"))
        );
        assert!(
            !Entry::parse("203.0.113.7/32")
                .unwrap()
                .matches(&ip("203.0.113.6"))
        );
        assert!(
            Entry::parse("0.0.0.0/0")
                .unwrap()
                .matches(&ip("198.51.100.9"))
        );
    }

    #[test]
    fn v6_exact_and_cidr() {
        assert!(Entry::parse("::1").unwrap().matches(&ip("::1")));
        assert!(!Entry::parse("::1").unwrap().matches(&ip("::2")));
        let e = Entry::parse("2001:db8::/32").unwrap();
        assert!(e.matches(&ip("2001:db8:ffff::1")));
        assert!(!e.matches(&ip("2001:db9::1")));
    }

    #[test]
    fn families_never_cross_match() {
        assert!(!Entry::parse("203.0.113.7").unwrap().matches(&ip("::1")));
        assert!(!Entry::parse("::1").unwrap().matches(&ip("127.0.0.1")));
    }

    #[test]
    fn cf_header_counts_only_from_a_loopback_peer() {
        let mut headers = HeaderMap::new();
        headers.insert("cf-connecting-ip", "203.0.113.7".parse().unwrap());
        let loopback: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        let remote: SocketAddr = "198.51.100.9:5000".parse().unwrap();
        // Through the tunnel (loopback peer): the header names
        // the visitor.
        assert_eq!(effective_ip(&loopback, &headers), Some(ip("203.0.113.7")));
        // Directly reachable: the same header is the visitor's
        // own claim and is ignored — the peer decides.
        assert_eq!(effective_ip(&remote, &headers), Some(ip("198.51.100.9")));
        assert_eq!(
            effective_ip(&remote, &HeaderMap::new()),
            Some(ip("198.51.100.9"))
        );
    }

    #[test]
    fn invalid_entries_are_errors() {
        assert!(Entry::parse("not-an-ip").is_err());
        assert!(Entry::parse("203.0.113.7/33").is_err());
        assert!(Entry::parse("2001:db8::/129").is_err());
        assert!(Entry::parse("203.0.113.7/x").is_err());
        assert!(Entry::parse("").is_err());
    }
}
