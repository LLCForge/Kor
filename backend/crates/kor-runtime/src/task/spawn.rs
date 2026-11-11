//! The production side of `kor task`: sub-agents assembled like any session (`prepare`), and
//! the orchestrator / reviewer models from `[orchestrator]`.

use std::path::Path;

use kor_core::PermissionMode;
use kor_i18n::{Lang, Localizer};
use kor_tools::ToolCtx;

use super::{Model, Spawned, Spawner};

/// Sub-agents from the user's configuration. Each one is a full session on its worktree:
/// permissions, sandbox, vault, hooks, transcript and its own cost control.
#[derive(Debug)]
pub struct RuntimeSpawner {
    pub lang: Lang,
    pub loaded: kor_config::Loaded,
}

impl RuntimeSpawner {
    pub fn new(lang: Lang, loaded: &kor_config::Loaded) -> Self {
        let mut loaded = loaded.clone();
        // N graph builds and watchers in N worktrees cost more than they save for one task.
        loaded.config.graph.enabled = false;
        Self { lang, loaded }
    }
}

impl Spawner for RuntimeSpawner {
    fn spawn(&self, worktree: &Path, _role: &str, model: Option<&str>) -> Result<Spawned, String> {
        let l = Localizer::new(self.lang);
        let root = worktree.canonicalize().map_err(|e| e.to_string())?;
        // Unattended: commands run without asking only inside an enforcing sandbox; otherwise
        // edits are allowed and every command question is answered "no".
        let enforcing =
            crate::sandbox::setup(&l, &self.loaded.config.sandbox, crate::offline_mode())
                .kind
                .is_enforcing();
        let mode = if enforcing && self.loaded.config.sandbox.mode == kor_config::SandboxMode::On {
            PermissionMode::Auto
        } else {
            PermissionMode::AcceptEdits
        };
        let opts = crate::Options {
            root,
            model: model.map(str::to_owned),
            mode: Some(mode),
            ..crate::Options::default()
        };
        let p = crate::prepare(&l, &self.loaded, &opts, &mut Vec::new())?;
        Ok(Spawned {
            agent: p.agent,
            cost: p.cost,
            session: p.path,
        })
    }

    fn command_ctx(&self, dir: &Path) -> Result<ToolCtx, String> {
        let l = Localizer::new(self.lang);
        let mut ctx = ToolCtx::new(dir).map_err(|e| e.to_string())?;
        let sbx = crate::sandbox::setup(&l, &self.loaded.config.sandbox, crate::offline_mode());
        ctx.sandbox = sbx.kind;
        ctx.network = sbx.network && !crate::offline_mode();
        ctx.sandbox_options = sbx.options;
        ctx.scrub_env = std::env::vars_os()
            .filter_map(|(k, _)| k.into_string().ok())
            .filter(|k| k.ends_with("_API_KEY") || k.ends_with("_TOKEN"))
            .collect();
        Ok(ctx)
    }
}

/// Orchestrator and reviewer: `[orchestrator] model` / `review_model`, else `[routing] review`
/// for the reviewer, else the session model. Privacy mode refuses non-local providers.
///
/// # Errors
/// No model configured, or the provider cannot be built or is not allowed.
pub fn models(
    l: &Localizer,
    loaded: &kor_config::Loaded,
    orchestrator_model: Option<&str>,
) -> Result<(Model, Model), String> {
    let cfg = &loaded.config;
    let pick = |s: &str| (!s.trim().is_empty()).then(|| s.trim().to_owned());
    let build = |m: Option<String>| -> Result<Model, String> {
        let (provider, model) = crate::oneshot_provider(l, loaded, m.as_deref())?;
        let cost = crate::cost_control(
            l,
            cfg,
            &provider,
            &model,
            crate::privacy_mode(),
            &mut Vec::new(),
        );
        Ok(Model {
            provider,
            model,
            cost: Some(cost),
        })
    };
    let planner_ref = orchestrator_model
        .and_then(pick)
        .or_else(|| pick(&cfg.orchestrator.model));
    let reviewer_ref = pick(&cfg.orchestrator.review_model)
        .or_else(|| pick(&cfg.routing.review))
        .or_else(|| planner_ref.clone());
    // Separate ledgers so the two roles are tagged apart.
    Ok((build(planner_ref)?, build(reviewer_ref)?))
}
