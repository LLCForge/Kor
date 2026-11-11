//! Devin: Markdown under `.devin/` (no published schema; read like the generic fallback, but
//! the whole folder and under Devin's name).

use super::{Adapter, mcp, md_rules};
use crate::{Applies, Compat, Eco, LoadOptions, Scope};

pub(crate) struct Devin;

impl Adapter for Devin {
    fn dirs(&self) -> &'static [&'static str] {
        &[".devin"]
    }
    fn project(&self, o: &LoadOptions, c: &mut Compat) {
        let dir = o.root.join(".devin");
        md_rules(
            c,
            &dir,
            ".md",
            Eco::Devin,
            Scope::Project,
            &Applies::Always,
            &[],
        );
        mcp(c, &dir.join("mcp.json"), Eco::Devin, Scope::Project);
    }
}
