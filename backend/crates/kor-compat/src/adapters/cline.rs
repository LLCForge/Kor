//! Cline: `.clinerules` (a single file, or a folder of `*.md` rules with optional `paths:`
//! front matter), `.clinerules/workflows/*.md` (slash commands). Global: `~/Documents/Cline/Rules/`.

use super::{Adapter, commands, md_rules, plain};
use crate::{Applies, Compat, Eco, LoadOptions, Scope};

pub(crate) struct Cline;

impl Adapter for Cline {
    fn dirs(&self) -> &'static [&'static str] {
        &[".clinerules"]
    }
    fn user(&self, o: &LoadOptions, c: &mut Compat) {
        let Some(home) = &o.home else { return };
        let dir = home.join("Documents/Cline/Rules");
        md_rules(
            c,
            &dir,
            ".md",
            Eco::Cline,
            Scope::User,
            &Applies::Always,
            &["workflows"],
        );
    }
    fn project(&self, o: &LoadOptions, c: &mut Compat) {
        let p = o.root.join(".clinerules");
        if p.is_dir() {
            md_rules(
                c,
                &p,
                ".md",
                Eco::Cline,
                Scope::Project,
                &Applies::Always,
                &["workflows"],
            );
            commands(c, &p.join("workflows"), Eco::Cline, Scope::Project, "");
        } else {
            plain(c, &p, Eco::Cline, Scope::Project, Applies::Always);
        }
    }
}
