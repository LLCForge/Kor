//! Network guard (spec §6.5, ADR 0035): one place that decides which hosts Kor may connect to.
//!
//! Every Kor HTTP client is built from [`client_builder`]. Its DNS resolver asks [`check_host`]
//! before any lookup, so a refused host fails at once (no connection, no retry storm), and the
//! addresses a name resolves to must match the policy too (no DNS-rebinding way around it).
//! IP-literal URLs skip resolvers in hyper; callers whose host comes from configuration (provider
//! base URLs) also call [`check_url`] before sending.
//!
//! - [`Policy::Open`]: everything allowed (the default).
//! - [`Policy::Privacy`]: loopback, plus the hosts of configured local providers when they
//!   resolve to private-LAN addresses.
//! - [`Policy::Offline`]: loopback only.

use std::collections::VecDeque;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};

/// Which hosts are reachable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Policy {
    #[default]
    Open,
    Privacy,
    Offline,
}

/// A connection the policy refused, with a user-facing (localized when set) message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct Blocked {
    pub host: String,
    pub policy: Policy,
    pub message: String,
}

/// One host Kor tried to reach, and whether the guard let it through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    pub host: String,
    pub allowed: bool,
}

static POLICY: AtomicU8 = AtomicU8::new(0);
static LOCAL_HOSTS: RwLock<Vec<String>> = RwLock::new(Vec::new());
static MESSAGES: RwLock<Option<(String, String)>> = RwLock::new(None);
static ATTEMPTS: Mutex<VecDeque<Attempt>> = Mutex::new(VecDeque::new());
const MAX_ATTEMPTS: usize = 256;

pub fn set_policy(p: Policy) {
    POLICY.store(
        match p {
            Policy::Open => 0,
            Policy::Privacy => 1,
            Policy::Offline => 2,
        },
        Ordering::Relaxed,
    );
}

pub fn policy() -> Policy {
    match POLICY.load(Ordering::Relaxed) {
        0 => Policy::Open,
        1 => Policy::Privacy,
        _ => Policy::Offline,
    }
}

/// Hosts of the user's configured local providers (reachable in privacy mode when they are on
/// the LAN). Replaces the previous list.
pub fn set_local_hosts(hosts: impl IntoIterator<Item = String>) {
    let mut v: Vec<String> = hosts.into_iter().map(|h| h.to_ascii_lowercase()).collect();
    v.sort_unstable();
    v.dedup();
    *LOCAL_HOSTS.write().unwrap_or_else(PoisonError::into_inner) = v;
}

/// Localized refusal texts; `{host}` is replaced. Without them the messages are English.
pub fn set_messages(privacy: String, offline: String) {
    *MESSAGES.write().unwrap_or_else(PoisonError::into_inner) = Some((privacy, offline));
}

/// Hosts checked since start (newest last, at most 256): the test hook behind ADR 0035.
pub fn attempts() -> Vec<Attempt> {
    ATTEMPTS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .cloned()
        .collect()
}

pub fn clear_attempts() {
    ATTEMPTS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
}

/// `localhost`, `*.localhost`, `127.x`, `::1` (with or without brackets).
pub fn is_loopback_host(host: &str) -> bool {
    let h = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    h == "localhost"
        || h.ends_with(".localhost")
        || h.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

fn is_lan(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || (v6.segments()[0] & 0xfe00) == 0xfc00 // unique local fc00::/7
                || (v6.segments()[0] & 0xffc0) == 0xfe80 // link local fe80::/10
                || v6.to_ipv4_mapped().is_some_and(|v4| is_lan(IpAddr::V4(v4)))
        }
    }
}

fn record(host: &str, allowed: bool) {
    let mut a = ATTEMPTS.lock().unwrap_or_else(PoisonError::into_inner);
    if a.len() == MAX_ATTEMPTS {
        a.pop_front();
    }
    a.push_back(Attempt {
        host: host.to_owned(),
        allowed,
    });
}

fn blocked(host: &str, policy: Policy) -> Blocked {
    let custom = MESSAGES
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let template = match (custom, policy) {
        (Some((p, _)), Policy::Privacy) => p,
        (Some((_, o)), _) => o,
        (None, Policy::Privacy) => {
            "Privacy mode: Kor does not connect to {host} (only local providers are allowed)."
                .into()
        }
        (None, _) => {
            "Offline mode: Kor does not connect to {host} (only this computer is allowed).".into()
        }
    };
    Blocked {
        host: host.to_owned(),
        policy,
        message: template.replace("{host}", host),
    }
}

/// May Kor connect to `host` under the current policy? Records the attempt.
pub fn check_host(host: &str) -> Result<(), Blocked> {
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    let p = policy();
    let ok = match p {
        Policy::Open => true,
        Policy::Offline => is_loopback_host(&host),
        // A configured name is checked again after resolution ([`addrs_fit`]).
        Policy::Privacy => {
            is_loopback_host(&host)
                || (host.parse::<IpAddr>().map_or(true, is_lan)
                    && LOCAL_HOSTS
                        .read()
                        .unwrap_or_else(PoisonError::into_inner)
                        .contains(&host))
        }
    };
    record(&host, ok);
    if ok { Ok(()) } else { Err(blocked(&host, p)) }
}

/// Host of an `scheme://[user@]host[:port]/...` URL, lowercase, IPv6 without brackets.
pub fn url_host(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if let Some(v6) = host.strip_prefix('[') {
        v6.split_once(']')?.0
    } else {
        host.split(':').next()?
    };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// The refusal for a program that would reach the network itself (MCP server, `git fetch`):
/// `None` when the policy is open.
pub fn refusal(target: &str) -> Option<Blocked> {
    let p = policy();
    (p != Policy::Open).then(|| blocked(target, p))
}

/// [`check_host`] for a URL. A URL without a host is refused unless the policy is open.
pub fn check_url(url: &str) -> Result<(), Blocked> {
    match url_host(url) {
        Some(h) => check_host(&h),
        None if policy() == Policy::Open => Ok(()),
        None => Err(blocked(url, policy())),
    }
}

/// Resolved addresses must fit the policy as well: a local name pointing at the internet is
/// refused (DNS rebinding).
fn addrs_fit(addrs: &[SocketAddr]) -> bool {
    match policy() {
        Policy::Open => true,
        Policy::Offline => addrs.iter().all(|a| a.ip().is_loopback()),
        Policy::Privacy => addrs.iter().all(|a| is_lan(a.ip())),
    }
}

/// The resolver every Kor client uses.
#[derive(Debug, Default, Clone, Copy)]
pub struct GuardResolver;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Resolved addresses are reused this long (§32.2 DNS cache, ADR 0062); the policy checks run on
/// every resolve all the same.
const DNS_TTL: std::time::Duration = std::time::Duration::from_secs(60);
/// Hosts remembered at most (the cache is cleared when full).
const DNS_CACHE: usize = 256;

type DnsCache = std::collections::HashMap<String, (std::time::Instant, Vec<SocketAddr>)>;
static DNS: std::sync::LazyLock<std::sync::Mutex<DnsCache>> =
    std::sync::LazyLock::new(std::sync::Mutex::default);

fn dns_cache() -> std::sync::MutexGuard<'static, DnsCache> {
    DNS.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

async fn lookup(host: &str) -> std::io::Result<Vec<SocketAddr>> {
    if let Some((at, addrs)) = dns_cache().get(host)
        && at.elapsed() < DNS_TTL
    {
        return Ok(addrs.clone());
    }
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, 0)).await?.collect();
    let mut c = dns_cache();
    if c.len() >= DNS_CACHE {
        c.clear();
    }
    c.insert(host.to_owned(), (std::time::Instant::now(), addrs.clone()));
    Ok(addrs)
}

async fn resolve_guarded(host: String) -> Result<reqwest::dns::Addrs, BoxError> {
    check_host(&host)?;
    let addrs = lookup(&host).await?;
    if !addrs_fit(&addrs) {
        record(&host, false);
        return Err(Box::new(blocked(&host, policy())));
    }
    Ok(Box::new(addrs.into_iter()))
}

impl reqwest::dns::Resolve for GuardResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(resolve_guarded(name.as_str().to_owned()))
    }
}

/// The start of every Kor HTTP client: guarded resolver; no system proxy outside open mode
/// (local servers are reached directly, and a proxy would be a way out).
pub fn client_builder() -> reqwest::ClientBuilder {
    let b = reqwest::Client::builder().dns_resolver(Arc::new(GuardResolver));
    if policy() == Policy::Open {
        b
    } else {
        b.no_proxy()
    }
}

/// A request error for the user: the guard's own (localized) refusal when it was one.
pub fn describe(e: &reqwest::Error) -> String {
    blocked_in(e).map_or_else(|| e.to_string(), |b| b.message.clone())
}

/// The [`Blocked`] refusal inside an error chain (reqwest wraps resolver errors).
pub fn blocked_in<'a>(e: &'a (dyn std::error::Error + 'static)) -> Option<&'a Blocked> {
    let mut cur: Option<&'a (dyn std::error::Error + 'static)> = Some(e);
    while let Some(err) = cur {
        if let Some(b) = err.downcast_ref::<Blocked>() {
            return Some(b);
        }
        cur = err.source();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // One test: the policy is process-global.
    #[tokio::test]
    async fn policy_decides_hosts_and_clients_fail_fast() {
        set_policy(Policy::Open);
        assert!(check_host("api.openai.com").is_ok());

        set_policy(Policy::Offline);
        assert!(check_url("http://127.0.0.1:11434/v1").is_ok());
        assert!(check_url("http://localhost:8080").is_ok());
        assert!(check_url("http://[::1]:1/x").is_ok());
        let e = check_url("https://openrouter.ai/api/v1").unwrap_err();
        assert_eq!(e.host, "openrouter.ai");
        assert!(e.message.contains("openrouter.ai"), "{}", e.message);
        assert!(check_url("http://192.168.1.5:11434").is_err());

        set_policy(Policy::Privacy);
        set_local_hosts(["gpu-box.lan".to_owned(), "192.168.1.5".to_owned()]);
        assert!(check_url("http://192.168.1.5:11434").is_ok());
        assert!(check_url("http://gpu-box.lan:11434").is_ok());
        assert!(check_url("http://192.168.1.6:11434").is_err());
        assert!(check_url("https://api.anthropic.com").is_err());
        set_messages("gizlilik {host}".into(), "çevrimdışı {host}".into());
        assert_eq!(check_host("x.com").unwrap_err().message, "gizlilik x.com");

        set_policy(Policy::Privacy);
        assert!(addrs_fit(&[
            "192.168.1.5:1".parse().unwrap(),
            "[fd00::1]:1".parse().unwrap()
        ]));
        assert!(!addrs_fit(&["8.8.8.8:1".parse().unwrap()]));
        set_policy(Policy::Offline);
        assert!(!addrs_fit(&["192.168.1.5:1".parse().unwrap()]));
        assert!(addrs_fit(&["127.0.0.1:1".parse().unwrap()]));

        let a = attempts();
        assert!(a.iter().any(|a| a.host == "openrouter.ai" && !a.allowed));
        assert!(a.iter().any(|a| a.host == "127.0.0.1" && a.allowed));
        assert_eq!(url_host("http://u:p@[::1]:8/x").as_deref(), Some("::1"));

        // A client built while open still obeys a policy switched on later.
        set_policy(Policy::Open);
        let client = client_builder().build().unwrap();
        set_policy(Policy::Offline);
        let started = std::time::Instant::now();
        let err = client
            .get("http://kor-leak.invalid/")
            .send()
            .await
            .unwrap_err();
        set_policy(Policy::Open);
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        let b = blocked_in(&err).expect("blocked in chain");
        assert_eq!(b.host, "kor-leak.invalid");
    }
}
