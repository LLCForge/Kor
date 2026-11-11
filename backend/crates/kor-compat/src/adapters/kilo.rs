//! Kilo Code: same layout as Roo under `.kilocode/` (`rules/`, `rules-<mode>/`, `mcp.json`) and
//! legacy `.kilocoderules`.

use super::Adapter;
use super::roo::family;
use crate::{Compat, Eco, LoadOptions};

pub(crate) struct Kilo;

impl Adapter for Kilo {
    fn dirs(&self) -> &'static [&'static str] {
        &[".kilocode"]
    }
    fn project(&self, o: &LoadOptions, c: &mut Compat) {
        family(c, o, ".kilocode", ".kilocoderules", Eco::Kilo, "Kilo Code");
    }
}
