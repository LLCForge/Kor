//! Technical SEO (spec §29.5–29.10, §29.15, §29.16; ADR 0055).
//!
//! Deterministic first (spec §9 principle): the crawler, checks, log parsing, competitor diffs
//! and the opportunity score run without a model. A model only writes content, meta and alt
//! text, and the team summary, through the cost router's route for the job.

// Product names in docs (WordPress, Search Console, …), short loop names and `x = y.to_owned()`
// on fresh fields read better as written here.
#![allow(
    clippy::doc_markdown,
    clippy::many_single_char_names,
    clippy::assigning_clones,
    clippy::missing_panics_doc,
    clippy::format_collect
)]

pub mod agents;
pub mod audit;
pub mod backlinks;
pub mod bing;
pub mod competitors;
pub mod content;
pub mod fetch;
pub mod google;
pub mod html;
pub mod logs;
pub mod meta;
pub mod model;
pub mod notify;
pub mod opportunity;
pub mod rich;
pub mod wp;
