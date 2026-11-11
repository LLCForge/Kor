//! `[env]` → the environment agent commands run in (§15.7, ADR 0043). Shared by the session
//! (`prepare`), `kor env status` and the desktop `env.status`. Detection, wrapping and the version
//! check live in `kor_tools::devenv`.

use std::path::Path;

use kor_i18n::{Localizer, args};
use kor_sandbox::SandboxKind;
use kor_tools::devenv::{self, Detected, Kind, OnMismatch, Session, Version};
use kor_tools::{Shell, ToolCtx};
use serde::Serialize;

pub fn on_mismatch(cfg: &kor_config::DevEnv) -> OnMismatch {
    match cfg.on_mismatch {
        kor_config::OnMismatch::Stop => OnMismatch::Stop,
        kor_config::OnMismatch::Warn => OnMismatch::Warn,
    }
}

/// Container environments cannot be confined by a host sandbox: their commands ask instead.
pub fn asks(env: &Session, sandbox: SandboxKind) -> bool {
    env.wrap && sandbox.is_enforcing() && env.detected.kind.as_ref().is_some_and(Kind::container)
}

/// The session's environment and localized notes about it; `None` when `[env] enabled = false`
/// or the project has no environment or version files.
pub fn session(
    l: &Localizer,
    cfg: &kor_config::DevEnv,
    root: &Path,
    sandbox: SandboxKind,
) -> (Option<Session>, Vec<String>) {
    if !cfg.enabled {
        return (None, Vec::new());
    }
    let det = devenv::detect(root, &cfg.service);
    let mut notes = Vec::new();
    if det.dockerfile_dev {
        notes.push(l.t("env-dockerfile-only"));
    }
    let Some(env) = Session::new(det, on_mismatch(cfg), sandbox, &devenv::on_path) else {
        return (None, notes);
    };
    if let Some(k) = &env.detected.kind {
        if env.wrap {
            notes.push(l.t_args(
                "env-active",
                &args!("kind" => k.name(), "program" => k.program()),
            ));
            if asks(&env, sandbox) {
                notes.push(l.t_args("env-container-asks", &args!("kind" => k.name())));
            }
        } else if sandbox == SandboxKind::Docker {
            notes.push(l.t_args("env-docker-sandbox", &args!("kind" => k.name())));
        } else if env.tool_missing {
            let key = match on_mismatch(cfg) {
                OnMismatch::Stop => "env-missing-stop",
                OnMismatch::Warn => "env-missing-warn",
            };
            notes.push(l.t_args(
                key,
                &args!("kind" => k.name(), "program" => k.program(), "hint" => k.install_hint()),
            ));
        }
    }
    (Some(env), notes)
}

/// `kor env status` / desktop `env.status`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub enabled: bool,
    pub on_mismatch: OnMismatch,
    pub detected: Detected,
    /// Commands are wrapped in the environment.
    pub wrap: bool,
    pub tool_missing: bool,
    /// Why no wrapper applies (English, as the model sees it).
    pub reason: Option<String>,
    /// What `cargo test` would run as.
    pub example: Option<String>,
    /// `inside` (wrapper inside the OS sandbox), `asks` (container: no sandbox, every command
    /// asks), `image` (Docker sandbox image is the environment), `none` (no sandbox).
    pub sandbox: &'static str,
    pub sandbox_backend: &'static str,
    pub versions: Vec<Version>,
    /// The refusal or warning the agent would get (English), if any.
    pub problems: Option<String>,
}

/// Detect, then run the version probe exactly as a session's first command would (same wrapper,
/// same sandbox).
pub async fn status(config: &kor_config::Config, root: &Path) -> Status {
    // Notes are not shown here; the localizer must not live across the probe's await.
    let sbx = {
        let l = Localizer::new(kor_i18n::Lang::En);
        crate::sandbox::setup(&l, &config.sandbox, crate::offline_mode())
    };
    let cfg = &config.env;
    let det = devenv::detect(root, &cfg.service);
    let mut st = Status {
        enabled: cfg.enabled,
        on_mismatch: on_mismatch(cfg),
        detected: det.clone(),
        wrap: false,
        tool_missing: false,
        reason: None,
        example: None,
        sandbox: "none",
        sandbox_backend: sbx.kind.as_str(),
        versions: Vec::new(),
        problems: None,
    };
    if !cfg.enabled {
        return st;
    }
    let Some(env) = Session::new(det, st.on_mismatch, sbx.kind, &devenv::on_path) else {
        return st;
    };
    let Ok(mut ctx) = ToolCtx::new(root) else {
        return st;
    };
    ctx.sandbox = sbx.kind;
    ctx.network = sbx.network && !crate::offline_mode();
    ctx.sandbox_options = sbx.options;
    let shell = if sbx.kind == SandboxKind::Docker {
        Shell::container()
    } else {
        Shell::detect()
    };
    let (p, a) = env.command(&shell, &ctx.root, "cargo test");
    st.example = Some(
        std::iter::once(p)
            .chain(a.into_iter().map(|x| {
                if x.contains(' ') {
                    format!("\"{x}\"")
                } else {
                    x
                }
            }))
            .collect::<Vec<_>>()
            .join(" "),
    );
    st.sandbox = if sbx.kind == SandboxKind::Docker && env.detected.kind.is_some() {
        "image"
    } else if asks(&env, sbx.kind) {
        "asks"
    } else if sbx.kind.is_enforcing() {
        "inside"
    } else {
        "none"
    };
    st.versions = env.check(&ctx, &shell).await.to_vec();
    st.wrap = env.wrap;
    st.tool_missing = env.tool_missing;
    st.reason.clone_from(&env.reason);
    st.problems = env.problems();
    st
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send<T: Send>(_: T) {}

    #[test]
    fn status_future_is_send() {
        let c = kor_config::Config::default();
        assert_send(status(&c, Path::new(".")));
    }

    #[test]
    fn notes_say_what_runs_where() {
        let l = Localizer::new(kor_i18n::Lang::En);
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let cfg = kor_config::DevEnv::default();
        assert!(session(&l, &cfg, root, SandboxKind::None).0.is_none());

        std::fs::write(root.join(".nvmrc"), "20\n").unwrap();
        let (env, notes) = session(&l, &cfg, root, SandboxKind::None);
        assert!(env.is_some() && notes.is_empty(), "{notes:?}");

        std::fs::create_dir_all(root.join(".devcontainer")).unwrap();
        std::fs::write(root.join(".devcontainer/devcontainer.json"), "{}").unwrap();
        let (env, notes) = session(&l, &cfg, root, SandboxKind::Docker);
        assert!(!env.unwrap().wrap);
        assert!(notes[0].contains("devcontainer"), "{notes:?}");

        let off = kor_config::DevEnv {
            enabled: false,
            ..cfg
        };
        assert!(session(&l, &off, root, SandboxKind::None).0.is_none());
    }
}
