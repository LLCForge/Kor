//! Aider: `CONVENTIONS.md` and the files named by `read:` in `.aider.conf.yml` (project) and
//! `~/.aider.conf.yml` (user). A project config may not pull in files from outside the project
//! unless the project is trusted.

use std::path::Path;

use super::{Adapter, plain};
use crate::frontmatter::split;
use crate::{Applies, Compat, Eco, LoadOptions, Scope, read};

pub(crate) struct Aider;

impl Adapter for Aider {
    fn user(&self, o: &LoadOptions, c: &mut Compat) {
        if let Some(home) = &o.home {
            conf(
                c,
                &home.join(".aider.conf.yml"),
                Scope::User,
                home,
                Some(home),
            );
        }
    }
    fn project(&self, o: &LoadOptions, c: &mut Compat) {
        let conventions = o.root.join("CONVENTIONS.md");
        plain(c, &conventions, Eco::Aider, Scope::Project, Applies::Always);
        let home = o.home.as_deref();
        conf(
            c,
            &o.root.join(".aider.conf.yml"),
            Scope::Project,
            &o.root,
            home,
        );
    }
}

/// `read:` entries (a scalar, `[a, b]` or a `- item` list), relative to the config's directory.
fn conf(c: &mut Compat, path: &Path, scope: Scope, base: &Path, home: Option<&Path>) {
    let Some(text) = read(path, &mut c.warnings) else {
        return;
    };
    // The top-level `key: value` subset of YAML is exactly what the front matter parser reads.
    let (front, _) = split(&format!("---\n{text}\n---\n"));
    let inside = base.canonicalize().ok();
    for entry in front.list("read") {
        let target = match (entry.strip_prefix("~/"), home) {
            (Some(rest), Some(h)) => h.join(rest),
            _ => base.join(&entry),
        };
        let Ok(target) = target.canonicalize() else {
            c.warnings
                .push(format!("{}: `read: {entry}` not found", path.display()));
            continue;
        };
        let outside = inside.as_ref().is_none_or(|i| !target.starts_with(i));
        if outside
            && !c.grant(
                scope,
                &format!("read `{entry}` outside the project from {}", path.display()),
            )
        {
            continue;
        }
        // CONVENTIONS.md named here too is dropped later as a duplicate text.
        plain(c, &target, Eco::Aider, scope, Applies::Always);
    }
}
