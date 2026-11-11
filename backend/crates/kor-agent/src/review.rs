//! Per-file review of a write before anything is written (spec §12.3, ADR 0030).
//!
//! An approver may answer an edit's permission request file by file: accept, reject, or accept
//! with its own content. Only accepted files are written; the model is told which were rejected
//! or changed. A plain yes/no/always answer is a `Review` with no per-file decisions.

use std::collections::BTreeMap;
use std::fmt::Write as _;

pub use kor_tools::Proposal;
use kor_tools::{ToolCtx, ToolOutput};

use crate::PermissionReply;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileDecision {
    Accept,
    Reject,
    /// Write this content instead of the proposal.
    Edit(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review {
    /// The answer for the call, and for every file without its own decision.
    pub reply: PermissionReply,
    /// Keyed by `Proposal::path`.
    pub files: BTreeMap<String, FileDecision>,
    /// Where the answer came from when the approver knows better than its own surface
    /// (`chrome`, `tray`, `telegram`, `link`…); `None` = [`crate::Approver::surface`].
    pub by: Option<&'static str>,
}

impl From<PermissionReply> for Review {
    fn from(reply: PermissionReply) -> Self {
        Self {
            reply,
            files: BTreeMap::new(),
            by: None,
        }
    }
}

/// What to do with a reviewed call.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Run the tool as proposed.
    All,
    /// Write nothing.
    Nothing,
    /// Write a subset, or edited content.
    Partial(Partial),
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Partial {
    pub writes: Vec<(String, Option<String>)>,
    pub applied: Vec<String>,
    pub edited: Vec<String>,
    pub rejected: Vec<String>,
}

pub(crate) fn outcome(files: &[Proposal], r: &Review) -> Outcome {
    let default = if r.reply == PermissionReply::Deny {
        FileDecision::Reject
    } else {
        FileDecision::Accept
    };
    if files.is_empty() {
        return if default == FileDecision::Reject {
            Outcome::Nothing
        } else {
            Outcome::All
        };
    }
    let mut p = Partial::default();
    for f in files {
        match r.files.get(&f.path).unwrap_or(&default) {
            FileDecision::Reject => p.rejected.push(f.path.clone()),
            FileDecision::Edit(c) if f.after.as_ref() != Some(c) => {
                p.writes.push((f.path.clone(), Some(c.clone())));
                p.edited.push(f.path.clone());
            }
            FileDecision::Accept | FileDecision::Edit(_) => {
                p.writes.push((f.path.clone(), f.after.clone()));
                p.applied.push(f.path.clone());
            }
        }
    }
    if p.writes.is_empty() {
        Outcome::Nothing
    } else if p.edited.is_empty() && p.rejected.is_empty() {
        Outcome::All
    } else {
        Outcome::Partial(p)
    }
}

/// Tool result when the user rejected every file of a reviewed change.
pub(crate) fn rejected_all(files: &[Proposal]) -> String {
    let names: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    format!(
        "the user rejected every file of this change; nothing was written ({}). Do not retry it unchanged; ask or take another approach.",
        names.join(", ")
    )
}

/// Write the accepted part and tell the model exactly what happened to each file.
pub(crate) async fn apply(ctx: &ToolCtx, p: Partial) -> ToolOutput {
    if let Err(e) = kor_tools::write_reviewed(ctx, &p.writes).await {
        return ToolOutput::err(format!("{e} (the reviewed change was only partly written)"));
    }
    let mut s = String::from("The user reviewed this change file by file.");
    let mut line = |label: &str, list: &[String]| {
        if !list.is_empty() {
            let _ = write!(s, "\n{label}: {}", list.join(", "));
        }
    };
    line("Written as you proposed", &p.applied);
    line(
        "Written with the user's own edits (differs from your proposal; Read it again before changing it)",
        &p.edited,
    );
    line(
        "Rejected by the user, not written (adapt your plan; do not write these again unchanged)",
        &p.rejected,
    );
    ToolOutput::ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files() -> Vec<Proposal> {
        ["a.rs", "b.rs", "c.rs"]
            .iter()
            .map(|p| Proposal {
                path: (*p).into(),
                before: Some("old".into()),
                after: Some(format!("new {p}")),
            })
            .collect()
    }

    fn review(reply: PermissionReply, d: &[(&str, FileDecision)]) -> Review {
        Review {
            reply,
            files: d
                .iter()
                .map(|(p, d)| ((*p).to_owned(), d.clone()))
                .collect(),
            by: None,
        }
    }

    #[test]
    fn plain_answers_stay_all_or_nothing() {
        for (reply, want) in [
            (PermissionReply::Allow, Outcome::All),
            (PermissionReply::AllowAlways, Outcome::All),
            (PermissionReply::Deny, Outcome::Nothing),
        ] {
            assert_eq!(outcome(&files(), &reply.into()), want);
            assert_eq!(outcome(&[], &reply.into()), want);
        }
    }

    #[test]
    fn per_file_decisions() {
        let r = review(
            PermissionReply::Allow,
            &[
                ("b.rs", FileDecision::Reject),
                ("c.rs", FileDecision::Edit("mine".into())),
            ],
        );
        let Outcome::Partial(p) = outcome(&files(), &r) else {
            panic!("partial expected")
        };
        assert_eq!(
            p.writes,
            vec![
                ("a.rs".into(), Some("new a.rs".into())),
                ("c.rs".into(), Some("mine".into()))
            ]
        );
        assert_eq!(
            (p.applied, p.edited, p.rejected),
            (
                vec!["a.rs".into()],
                vec!["c.rs".into()],
                vec!["b.rs".into()]
            )
        );

        // Deny is the default for unlisted files; an unchanged "edit" is a plain accept.
        let r = review(
            PermissionReply::Deny,
            &[("a.rs", FileDecision::Edit("new a.rs".into()))],
        );
        let Outcome::Partial(p) = outcome(&files(), &r) else {
            panic!("partial expected")
        };
        assert_eq!((p.applied.len(), p.rejected.len()), (1, 2));

        let r = review(
            PermissionReply::Allow,
            &[
                ("a.rs", FileDecision::Reject),
                ("b.rs", FileDecision::Reject),
                ("c.rs", FileDecision::Reject),
            ],
        );
        assert_eq!(outcome(&files(), &r), Outcome::Nothing);
        assert!(rejected_all(&files()).contains("a.rs, b.rs, c.rs"));
    }
}
