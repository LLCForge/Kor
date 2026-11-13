//! `[sandbox]` → the backend, network rule and limits of agent commands (§6.4, ADR 0033).
//! Shared by the session (`prepare`), `kor sandbox`, `kor doctor` and the desktop.

use kor_config::{SandboxBackend, SandboxMode};
use kor_i18n::{Localizer, args};
use kor_sandbox::{Filter, Options, Preference, SandboxKind};

/// The backend `[sandbox]` asks for on this machine; `None` unless `mode = "on"`.
pub fn kind(cfg: &kor_config::Sandbox) -> SandboxKind {
    if cfg.mode != SandboxMode::On {
        return SandboxKind::None;
    }
    kor_sandbox::select(match cfg.backend {
        SandboxBackend::Auto => Preference::Auto,
        SandboxBackend::Native => Preference::Native,
        SandboxBackend::Docker => Preference::Docker,
    })
}

/// What a session's commands get.
#[derive(Debug, Clone)]
pub struct Setup {
    pub kind: SandboxKind,
    pub network: bool,
    pub options: Options,
    /// Localized warnings: dropped allowlist entries, unfiltered allowlist, missing image.
    pub notes: Vec<String>,
}

/// `cfg` checked and turned into a [`Setup`]. Offline mode keeps the network off whatever the
/// allowlist says. Invalid entries are dropped (closed), never widened.
pub fn setup(l: &Localizer, cfg: &kor_config::Sandbox, offline: bool) -> Setup {
    let kind = kind(cfg);
    let mut notes = Vec::new();
    let mut allow = Vec::new();
    for entry in &cfg.network {
        if kor_sandbox::valid_allow(entry.trim()) {
            allow.push(entry.trim().to_owned());
        } else {
            notes.push(l.t_args("sandbox-net-invalid", &args!("entry" => entry.clone())));
        }
    }
    let mut network = !allow.is_empty();
    if offline && network {
        network = false;
        notes.push(l.t("sandbox-net-offline"));
    }
    // `*` anywhere means the whole network: no filter to apply.
    if allow.iter().any(|a| a == "*") {
        allow.clear();
    }
    if network && !allow.is_empty() && kind.is_enforcing() {
        match kor_sandbox::enforcement(kind).filter {
            Filter::None => {
                notes.push(l.t_args("sandbox-net-wide", &args!("backend" => kind.as_str())));
            }
            Filter::Proxy => {
                notes.push(l.t_args("sandbox-net-proxy", &args!("backend" => kind.as_str())));
            }
            Filter::Domains => {}
        }
    }
    let mut image = cfg.image.trim().to_owned();
    if !kor_sandbox::valid_image(&image) {
        notes.push(l.t_args("sandbox-image-invalid", &args!("image" => image.clone())));
        image = kor_sandbox::DEFAULT_IMAGE.into();
    }
    if kind == SandboxKind::Docker {
        let st = kor_sandbox::docker_state(&image);
        if !st.daemon {
            notes.push(l.t("sandbox-docker-down"));
        } else if !st.image_present {
            notes.push(l.t_args("sandbox-docker-image", &args!("image" => image.clone())));
        }
    }
    Setup {
        kind,
        network,
        options: Options {
            allow,
            overlay: cfg.overlay,
            memory_mb: cfg.memory_mb,
            cpu_percent: cfg.cpu_percent,
            image,
        },
        notes,
    }
}

/// `kor sandbox status` / desktop `sandbox.status`: the configured mode and what `mode = "on"`
/// uses and enforces here.
#[derive(Debug, Clone)]
pub struct Status {
    pub mode: SandboxMode,
    pub preference: SandboxBackend,
    /// Backend, network and limits as `mode = "on"` would apply them.
    pub setup: Setup,
    pub enforcement: kor_sandbox::Enforcement,
    /// Only when Docker is the backend or was asked for.
    pub docker: Option<kor_sandbox::DockerState>,
}

pub fn status(l: &Localizer, cfg: &kor_config::Sandbox) -> Status {
    let on = kor_config::Sandbox {
        mode: SandboxMode::On,
        ..cfg.clone()
    };
    let setup = setup(l, &on, crate::offline_mode());
    let docker = (setup.kind == SandboxKind::Docker || cfg.backend == SandboxBackend::Docker)
        .then(|| kor_sandbox::docker_state(&setup.options.image));
    Status {
        mode: cfg.mode,
        preference: cfg.backend,
        enforcement: kor_sandbox::enforcement(setup.kind),
        setup,
        docker,
    }
}

pub fn preference_str(p: SandboxBackend) -> &'static str {
    match p {
        SandboxBackend::Auto => "auto",
        SandboxBackend::Native => "native",
        SandboxBackend::Docker => "docker",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l() -> Localizer {
        Localizer::new(kor_i18n::Lang::En)
    }

    #[test]
    fn allowlist_is_checked_and_offline_wins() {
        let cfg = kor_config::Sandbox {
            mode: SandboxMode::Off,
            network: vec!["github.com:443".into(), "bad entry".into(), " *:80 ".into()],
            image: "--privileged".into(),
            ..kor_config::Sandbox::default()
        };
        let s = setup(&l(), &cfg, false);
        assert_eq!(s.kind, SandboxKind::None);
        assert!(s.network);
        assert_eq!(s.options.allow, ["github.com:443", "*:80"]);
        assert_eq!(s.options.image, kor_sandbox::DEFAULT_IMAGE);
        assert_eq!(s.notes.len(), 2, "{:?}", s.notes);
        assert!(s.notes[0].contains("bad entry"));

        let s = setup(&l(), &cfg, true);
        assert!(!s.network);

        let all = kor_config::Sandbox {
            network: vec!["*".into(), "github.com:443".into()],
            ..cfg
        };
        let s = setup(&l(), &all, false);
        assert!(s.network && s.options.allow.is_empty());

        let off = setup(&l(), &kor_config::Sandbox::default(), false);
        assert!(!off.network, "no network by default");
        assert!(!off.options.overlay);
        let overlay = kor_config::Sandbox {
            overlay: true,
            ..kor_config::Sandbox::default()
        };
        assert!(setup(&l(), &overlay, false).options.overlay);
        assert_eq!((off.options.memory_mb, off.options.cpu_percent), (8192, 90));
    }
}
