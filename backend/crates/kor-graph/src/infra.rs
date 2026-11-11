//! Infrastructure layer (spec §15.16, ADR 0044): servers scanned by `kor discover` and what runs
//! on them. A scan is an [`InfraSnapshot`] (no secrets, no file contents: env files are paths
//! only). Kor keeps every scan in its home (`<kor home>/infra/<host>.{md,json}`); a project links
//! a host by holding a copy in `<root>/.kor/graph/infra/<host>.json`, which the graph reads like
//! the database snapshots of ADR 0024.
//!
//! Nodes: `server:<host>` ([`NodeKind::Server`]) and `service:<host>/<kind>/<name>`
//! ([`NodeKind::Service`]). Edges: service → server `RunsOn` (`Extracted`) and code → service
//! `RunsOn` (`Inferred`) when a service carries the project's or a package's name (unit,
//! container, Compose service or project, image) or its working directory / command line has a
//! path segment with that name. The code side is the package node in a monorepo, else the root
//! manifest file node; no node is invented for code that is not in the graph.

use crate::model::{Confidence, EdgeKind, NodeKind};
use crate::resolve::Derived;
use crate::workspace::Workspace;
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const SERVER_PREFIX: &str = "server:";
pub const SERVICE_PREFIX: &str = "service:";

/// One read-only scan of a machine.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InfraSnapshot {
    /// File-safe id of the host ([`host_id`]).
    pub host: String,
    /// `user@host[:port]` or connection id as given; never a secret.
    pub address: String,
    /// Unix seconds.
    pub taken_at: u64,
    pub hostname: String,
    /// `PRETTY_NAME` of `/etc/os-release`, else `uname -s`.
    pub os: String,
    pub kernel: String,
    pub arch: String,
    pub services: Vec<Service>,
    pub ports: Vec<Port>,
    pub vhosts: Vec<VHost>,
    pub schedules: Vec<Schedule>,
    pub disks: Vec<Disk>,
    pub memory: Option<Memory>,
    pub languages: Vec<ToolVersion>,
    /// Paths of env files found (never their contents).
    pub env_files: Vec<String>,
    /// Steps skipped or failed, with the reason (sudo declined, tool missing, ...).
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Service {
    /// `systemd`, `docker`, `vm` (Proxmox `qm`) or `lxc` (Proxmox `pct`).
    pub kind: String,
    pub name: String,
    pub state: String,
    /// Command line / working directory (systemd), image (docker), Compose project directory.
    pub detail: String,
    /// Docker image.
    pub image: Option<String>,
    /// Compose project and service (from container labels).
    pub compose_project: Option<String>,
    pub compose_service: Option<String>,
    /// TCP ports this service listens on or publishes.
    pub ports: Vec<u16>,
    /// Virtual host names routed to it (nginx / Caddy).
    pub domains: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Port {
    pub port: u16,
    pub address: String,
    /// Process name, when visible (other users' processes need sudo).
    pub process: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VHost {
    /// `nginx` or `caddy`.
    pub server: String,
    pub names: Vec<String>,
    /// `proxy_pass` / `reverse_proxy` targets, `root` directories.
    pub upstreams: Vec<String>,
    /// Services the upstream ports resolve to.
    pub services: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Schedule {
    /// `cron` or `timer`.
    pub kind: String,
    pub when: String,
    pub what: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Disk {
    pub mount: String,
    pub size: String,
    pub used: String,
    pub avail: String,
    pub use_pct: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Memory {
    pub total_mb: u64,
    pub used_mb: u64,
    pub available_mb: u64,
    pub swap_total_mb: u64,
    pub swap_used_mb: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolVersion {
    pub name: String,
    pub version: String,
}

/// File-safe id of `address` (`user@host:port`, `host`, `[::1]:22`): the host part, lowercase,
/// `:` of IPv6 as `_`. `None` when nothing usable is left.
pub fn host_id(address: &str) -> Option<String> {
    let host = address.rsplit_once('@').map_or(address, |(_, h)| h);
    let host = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or(rest)
    } else if host.matches(':').count() == 1 {
        host.split(':').next().unwrap_or(host)
    } else {
        host
    };
    let id: String = host
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c == ':' { '_' } else { c })
        .collect();
    let ok = (1..=100).contains(&id.len())
        && !id.starts_with(['.', '-'])
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'));
    ok.then_some(id)
}

/// `<root>/.kor/graph/infra/`: hosts linked to this project.
pub fn infra_dir(root: &Path) -> PathBuf {
    root.join(".kor").join("graph").join("infra")
}

/// Write `<dir>/<host>.json` atomically. Returns the path.
///
/// # Errors
/// If the host id is not file-safe or the file cannot be written.
pub fn save(dir: &Path, snap: &InfraSnapshot) -> Result<PathBuf> {
    if host_id(&snap.host).as_deref() != Some(snap.host.as_str()) {
        return Err(Error::Host(snap.host.clone()));
    }
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.json", snap.host));
    let tmp = dir.join(format!("{}.json.tmp", snap.host));
    std::fs::write(&tmp, serde_json::to_vec_pretty(snap)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

/// Every readable snapshot in `dir`, by host.
pub fn load_dir(dir: &Path) -> Vec<InfraSnapshot> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<InfraSnapshot> = rd
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| std::fs::read(e.path()).ok())
        .filter_map(|b| serde_json::from_slice::<InfraSnapshot>(&b).ok())
        .filter(|s| host_id(&s.host).as_deref() == Some(s.host.as_str()))
        .collect();
    out.sort_by(|a, b| a.host.cmp(&b.host));
    out
}

/// Hosts linked to the project at `root`.
pub fn load_snapshots(root: &Path) -> Vec<InfraSnapshot> {
    load_dir(&infra_dir(root))
}

/// Lowercase letters and digits only (`my_app`, `My-App` and `myapp` match).
fn key(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Names a service is known by: unit without `.service`, container, Compose project/service,
/// image name without registry and tag.
fn service_names(s: &Service) -> Vec<String> {
    let mut out = vec![key(s.name.trim_end_matches(".service"))];
    out.extend(s.compose_project.iter().map(|p| key(p)));
    out.extend(s.compose_service.iter().map(|p| key(p)));
    if let Some(img) = &s.image {
        let last = img.rsplit('/').next().unwrap_or(img);
        out.push(key(last.split([':', '@']).next().unwrap_or(last)));
    }
    out.retain(|k| !k.is_empty());
    out
}

fn has_segment(detail: &str, want: &str) -> bool {
    detail
        .split(['/', ' ', '=', ':', '"', '\'', ';'])
        .any(|seg| !seg.is_empty() && key(seg) == want)
}

pub(crate) const ROOT_MANIFESTS: &[&str] = &[
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "go.mod",
    "composer.json",
    "Gemfile",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "mix.exs",
    "requirements.txt",
];

/// Server and service nodes, `RunsOn` edges. `root_name` is the project directory name.
pub(crate) fn layer(
    d: &mut Derived,
    snaps: &[InfraSnapshot],
    ws: Option<&Workspace>,
    root_name: &str,
) {
    // (match key, code node) pairs; keys shorter than 3 letters match too much.
    let mut code: Vec<(String, String)> = Vec::new();
    match ws.filter(|w| w.is_monorepo()) {
        Some(w) => {
            for p in &w.packages {
                let id = format!("{}{}", crate::layers::PACKAGE_PREFIX, p.name);
                let last = p.name.rsplit(['/', ':']).next().unwrap_or(&p.name);
                code.push((key(last), id.clone()));
                if let Some(dir) = p.root.file_name() {
                    code.push((key(&dir.to_string_lossy()), id));
                }
            }
        }
        None => {
            if let Some(m) = ROOT_MANIFESTS.iter().find(|m| d.nodes.contains_key(**m)) {
                code.push((key(root_name), (*m).to_owned()));
            }
        }
    }
    code.retain(|(k, _)| k.len() >= 3);
    for snap in snaps {
        let server = format!("{SERVER_PREFIX}{}", snap.host);
        let name = if snap.hostname.is_empty() {
            &snap.host
        } else {
            &snap.hostname
        };
        let text = [snap.os.as_str(), snap.arch.as_str(), snap.address.as_str()]
            .iter()
            .filter(|s| !s.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(" · ");
        d.node(&server, NodeKind::Server, name, None, None, &text);
        for s in &snap.services {
            if s.name.is_empty() {
                continue;
            }
            let id = format!("{SERVICE_PREFIX}{}/{}/{}", snap.host, s.kind, s.name);
            let mut text = format!("{} {}", s.kind, s.state);
            if !s.detail.is_empty() {
                text.push_str(": ");
                text.push_str(&s.detail);
            }
            if !s.domains.is_empty() {
                text.push_str("\nserves ");
                text.push_str(&s.domains.join(", "));
            }
            d.node(&id, NodeKind::Service, &s.name, None, None, &text);
            d.edge(&id, &server, EdgeKind::RunsOn, Confidence::Extracted);
            let names = service_names(s);
            for (k, node) in &code {
                if names.iter().any(|n| n == k) {
                    d.edge(
                        node,
                        &id,
                        EdgeKind::RunsOn,
                        Confidence::Inferred { score: 0.7 },
                    );
                } else if has_segment(&s.detail, k) {
                    d.edge(
                        node,
                        &id,
                        EdgeKind::RunsOn,
                        Confidence::Inferred { score: 0.6 },
                    );
                }
            }
        }
    }
}

impl crate::Graph {
    /// Hosts linked to this project (`.kor/graph/infra/*.json`).
    pub fn infra_snapshots(&self) -> &[InfraSnapshot] {
        &self.infra
    }

    /// Re-read `.kor/graph/infra/*.json`; recomputes only when a snapshot changed.
    ///
    /// # Errors
    /// If the graph cannot be saved.
    pub fn refresh_infra(&mut self) -> Result<crate::UpdateStats> {
        let snaps = load_snapshots(&self.root);
        if snaps == self.infra {
            return Ok(crate::UpdateStats::default());
        }
        self.infra = snaps;
        self.recompute()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Graph;

    fn snap() -> InfraSnapshot {
        InfraSnapshot {
            host: "web1".into(),
            address: "deploy@web1".into(),
            hostname: "web1".into(),
            os: "Debian GNU/Linux 12".into(),
            arch: "x86_64".into(),
            services: vec![
                Service {
                    kind: "systemd".into(),
                    name: "shop-api.service".into(),
                    state: "running".into(),
                    detail: "/srv/shop/bin/server --port 3000".into(),
                    ..Service::default()
                },
                Service {
                    kind: "docker".into(),
                    name: "shop-web-1".into(),
                    state: "running".into(),
                    image: Some("ghcr.io/acme/shop:1.4".into()),
                    domains: vec!["shop.example.com".into()],
                    ..Service::default()
                },
                Service {
                    kind: "systemd".into(),
                    name: "postgresql.service".into(),
                    state: "running".into(),
                    ..Service::default()
                },
            ],
            ..InfraSnapshot::default()
        }
    }

    fn has(g: &Graph, s: &str, t: &str) -> bool {
        g.edges()
            .any(|e| e.source == s && e.target == t && e.kind == EdgeKind::RunsOn)
    }

    #[test]
    fn host_ids_are_file_safe() {
        assert_eq!(
            host_id("deploy@Web1.example.com:2222").as_deref(),
            Some("web1.example.com")
        );
        assert_eq!(host_id("[::1]:22").as_deref(), Some("__1"));
        assert_eq!(host_id("fe80::1").as_deref(), Some("fe80__1"));
        assert_eq!(host_id("../etc"), None);
        assert_eq!(host_id("a/b"), None);
        assert_eq!(host_id(""), None);
    }

    #[test]
    fn layer_links_code_service_and_server() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path().join("shop");
        std::fs::create_dir_all(root.join("src"))?;
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"shop\"\n")?;
        std::fs::write(root.join("src/main.rs"), "fn main() {}\n")?;
        let mut g = Graph::open(&root)?;
        g.update()?;
        assert!(g.node("server:web1").is_none());
        let path = save(&infra_dir(&root), &snap())?;
        assert!(path.ends_with("web1.json"));
        let s = g.refresh_infra()?;
        assert!(s.changed);
        let server = g.node("server:web1").expect("server node");
        assert_eq!(server.kind, NodeKind::Server);
        let api = "service:web1/systemd/shop-api.service";
        let web = "service:web1/docker/shop-web-1";
        assert_eq!(g.node(api).map(|n| n.kind), Some(NodeKind::Service));
        assert!(has(&g, api, "server:web1"));
        assert!(has(&g, "Cargo.toml", web), "image name shop");
        assert!(has(&g, "Cargo.toml", api), "/srv/shop path segment");
        assert!(!has(
            &g,
            "Cargo.toml",
            "service:web1/systemd/postgresql.service"
        ));
        assert!(
            g.node(web)
                .is_some_and(|n| n.text.contains("shop.example.com"))
        );
        // Impact of the server reaches the code through the services.
        let imp = g.impact("server:web1", 3);
        assert!(imp.affected.iter().any(|a| a.node.id == "Cargo.toml"));
        // Persisted, and a rescan keeps the layer.
        g.update()?;
        drop(g);
        let g = Graph::open(&root)?;
        assert!(g.node(api).is_some());
        assert_eq!(g.infra_snapshots().len(), 1);
        Ok(())
    }

    #[test]
    fn monorepo_packages_are_the_code_side() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path().join("mono");
        let w = |p: &str, s: &str| -> Result<()> {
            let path = root.join(p);
            std::fs::create_dir_all(path.parent().expect("parent"))?;
            std::fs::write(path, s)?;
            Ok(())
        };
        w("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n")?;
        w(
            "crates/billing/Cargo.toml",
            "[package]\nname = \"billing\"\n",
        )?;
        w("crates/billing/src/lib.rs", "pub fn x() {}\n")?;
        w("crates/other/Cargo.toml", "[package]\nname = \"other\"\n")?;
        w("crates/other/src/lib.rs", "pub fn y() {}\n")?;
        let mut s = snap();
        s.services.push(Service {
            kind: "docker".into(),
            name: "mono-billing-1".into(),
            state: "running".into(),
            compose_project: Some("mono".into()),
            compose_service: Some("billing".into()),
            ..Service::default()
        });
        save(&infra_dir(&root), &s)?;
        let mut g = Graph::open(&root)?;
        g.update()?;
        assert!(has(&g, "pkg:billing", "service:web1/docker/mono-billing-1"));
        assert!(!has(&g, "pkg:other", "service:web1/docker/mono-billing-1"));
        Ok(())
    }

    #[test]
    fn bad_snapshots_are_refused_or_skipped() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut s = snap();
        s.host = "../x".into();
        assert!(save(dir.path(), &s).is_err());
        std::fs::write(dir.path().join("evil.json"), r#"{"host":"../evil"}"#)?;
        std::fs::write(dir.path().join("broken.json"), "{")?;
        assert!(load_dir(dir.path()).is_empty());
        Ok(())
    }
}
