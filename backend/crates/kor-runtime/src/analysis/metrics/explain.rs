//! `kor explain <file>`: code archaeology. History (commits, age, last change), who wrote the
//! lines today (blame share), renames and moves (`git log --follow -M`), refactor commits,
//! danger zones (complexity × the number of commits that still own lines in a function), and
//! the file's place in the project graph. All of it works offline; [`prompt`] turns it into a
//! request for a plain-language explanation at a level, with author names replaced by
//! "Author 1/2/…" (names are only shown locally).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::code::{self, FileStats};
use super::git::{self, LogOpts};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Junior,
    #[default]
    Normal,
    Expert,
}

impl Level {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "junior" => Self::Junior,
            "normal" => Self::Normal,
            "expert" => Self::Expert,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Author {
    pub name: String,
    pub email: String,
    /// Commits touching the file.
    pub commits: u32,
    /// Lines of the current file `git blame` gives them.
    pub lines: u32,
    /// `lines` / all lines, 0–100.
    pub share: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitRef {
    pub hash: String,
    pub time: i64,
    pub subject: String,
    pub author: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rename {
    pub commit: CommitRef,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DangerZone {
    pub name: String,
    pub line: u32,
    pub end: u32,
    pub complexity: u32,
    /// Distinct commits that last changed a line of this function (blame).
    pub commits: u32,
    /// Lines changed in the last 90 days.
    pub recent_lines: u32,
    /// `complexity × commits`.
    pub score: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphPlace {
    pub community: Option<String>,
    /// Files and symbols that depend on this file directly.
    pub dependents: Vec<String>,
    /// Files changed together with this one (ratio of shared commits).
    pub co_changes: Vec<(String, f32)>,
    pub dependents_total: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Archaeology {
    pub file: String,
    pub lang: Option<String>,
    pub stats: Option<FileStats>,
    pub tracked: bool,
    pub commits: usize,
    pub first: Option<CommitRef>,
    pub last: Option<CommitRef>,
    pub age_days: Option<i64>,
    pub last_change_days: Option<i64>,
    pub churn_90d: u32,
    pub churn_365d: u32,
    pub authors: Vec<Author>,
    pub renames: Vec<Rename>,
    pub refactors: Vec<CommitRef>,
    pub danger: Vec<DangerZone>,
    pub graph: Option<GraphPlace>,
    /// Where each part came from; parts that could not be read say why.
    pub sources: Vec<String>,
}

fn cref(c: &git::Commit) -> CommitRef {
    CommitRef {
        hash: c.hash.chars().take(10).collect(),
        time: c.time,
        subject: c.subject.clone(),
        author: c.author.clone(),
    }
}

fn is_refactor(subject: &str) -> bool {
    let s = subject.to_lowercase();
    s.starts_with("refactor")
        || [
            "refactor",
            "restructure",
            "rewrite",
            "extract",
            "rename",
            "move ",
            "split ",
            "cleanup",
            "clean up",
            "yeniden düzen",
            "taşı",
            "ayır",
        ]
        .iter()
        .any(|w| s.contains(w))
}

/// Facts about `file` (project-relative or absolute inside `root`). `graph` is optional so
/// callers that already hold the project graph pass it; otherwise it is opened here.
pub fn archaeology(
    root: &Path,
    file: &str,
    graph: Option<&kor_graph::Graph>,
) -> Result<Archaeology, String> {
    let path = root.join(file);
    let canon_root = root.canonicalize().map_err(|e| e.to_string())?;
    let canon = path
        .canonicalize()
        .map_err(|_| format!("not found: {file}"))?;
    if !canon.starts_with(&canon_root) || !canon.is_file() {
        return Err(format!("not a file inside the project: {file}"));
    }
    let rel = super::rel(&canon_root, &canon);
    let now = super::now_secs();
    let mut a = Archaeology {
        file: rel.clone(),
        lang: code::lang_of(&rel).map(|l| l.name().to_owned()),
        stats: None,
        tracked: false,
        commits: 0,
        first: None,
        last: None,
        age_days: None,
        last_change_days: None,
        churn_90d: 0,
        churn_365d: 0,
        authors: Vec::new(),
        renames: Vec::new(),
        refactors: Vec::new(),
        danger: Vec::new(),
        graph: None,
        sources: Vec::new(),
    };
    if let (Some(lang), Ok(text)) = (code::lang_of(&rel), std::fs::read_to_string(&canon)) {
        a.stats = Some(code::analyze(lang, &rel, &text));
        a.sources.push("tree-sitter: functions, complexity".into());
    }
    let commits = if git::is_repo(root) {
        git::log(
            root,
            &LogOpts {
                path: Some(&rel),
                follow: true,
                ..LogOpts::default()
            },
        )?
    } else {
        a.sources.push("git: not a repository".into());
        Vec::new()
    };
    a.tracked = !commits.is_empty();
    if a.tracked {
        a.sources
            .push(format!("git log --follow -M: {} commits", commits.len()));
        a.commits = commits.len();
        a.first = commits.last().map(cref);
        a.last = commits.first().map(cref);
        a.age_days = commits.last().map(|c| (now - c.time) / 86_400);
        a.last_change_days = commits.first().map(|c| (now - c.time) / 86_400);
        let since = |d: i64| {
            u32::try_from(
                commits
                    .iter()
                    .filter(|c| c.time >= now - d * 86_400)
                    .count(),
            )
            .unwrap_or(u32::MAX)
        };
        a.churn_90d = since(90);
        a.churn_365d = since(365);
        for c in &commits {
            for f in &c.files {
                if let Some(from) = &f.from
                    && from != &f.path
                {
                    a.renames.push(Rename {
                        commit: cref(c),
                        from: from.clone(),
                        to: f.path.clone(),
                    });
                }
            }
        }
        a.refactors = commits
            .iter()
            .filter(|c| is_refactor(&c.subject))
            .map(cref)
            .collect();
        authors_and_danger(root, &rel, &commits, now, &mut a);
    }
    place(root, &rel, graph, &mut a);
    Ok(a)
}

fn authors_and_danger(
    root: &Path,
    rel: &str,
    commits: &[git::Commit],
    now: i64,
    a: &mut Archaeology,
) {
    let mut by_email: BTreeMap<String, Author> = BTreeMap::new();
    for c in commits {
        let e = by_email.entry(c.email.clone()).or_insert_with(|| Author {
            name: c.author.clone(),
            email: c.email.clone(),
            commits: 0,
            lines: 0,
            share: 0.0,
        });
        e.commits += 1;
    }
    let blame = match git::blame(root, rel) {
        Ok(b) => {
            a.sources
                .push(format!("git blame -w -M: {} lines", b.len()));
            b
        }
        Err(e) => {
            a.sources.push(format!("git blame: {e}"));
            Vec::new()
        }
    };
    for l in &blame {
        let e = by_email.entry(l.email.clone()).or_insert_with(|| Author {
            name: l.author.clone(),
            email: l.email.clone(),
            commits: 0,
            lines: 0,
            share: 0.0,
        });
        e.lines += 1;
    }
    let total = blame.len().max(1);
    let mut authors: Vec<Author> = by_email
        .into_values()
        .map(|mut x| {
            #[allow(clippy::cast_precision_loss)]
            let s = f64::from(x.lines) * 100.0 / total as f64;
            x.share = super::round1(s);
            x
        })
        .collect();
    authors.sort_by(|x, y| y.lines.cmp(&x.lines).then(y.commits.cmp(&x.commits)));
    a.authors = authors;
    let Some(stats) = &a.stats else { return };
    let mut zones: Vec<DangerZone> = stats
        .functions
        .iter()
        .map(|f| {
            let span: Vec<&git::BlameLine> = blame
                .iter()
                .filter(|b| b.line >= f.line && b.line <= f.end)
                .collect();
            let owners: BTreeSet<&str> = span.iter().map(|b| b.hash.as_str()).collect();
            let commits = u32::try_from(owners.len()).unwrap_or(u32::MAX);
            let recent = span.iter().filter(|b| b.time >= now - 90 * 86_400).count();
            DangerZone {
                name: f.name.clone(),
                line: f.line,
                end: f.end,
                complexity: f.complexity,
                commits,
                recent_lines: u32::try_from(recent).unwrap_or(u32::MAX),
                score: f.complexity.saturating_mul(commits.max(1)),
            }
        })
        .filter(|z| z.complexity >= 5 && z.score >= 10)
        .collect();
    zones.sort_by(|x, y| y.score.cmp(&x.score).then(x.line.cmp(&y.line)));
    zones.truncate(5);
    a.danger = zones;
}

fn place(root: &Path, rel: &str, graph: Option<&kor_graph::Graph>, a: &mut Archaeology) {
    let owned;
    let g = if let Some(g) = graph {
        g
    } else {
        match super::graph(root) {
            Ok(g) => {
                owned = g;
                &owned
            }
            Err(e) => {
                a.sources.push(format!("project graph: {e}"));
                return;
            }
        }
    };
    let Some(ex) = g.explain(rel) else {
        a.sources
            .push("project graph: file not in the graph".into());
        return;
    };
    let imp = g.impact(rel, 1);
    let mut dependents: Vec<String> = imp
        .affected
        .iter()
        .map(|e| match &e.node.file {
            Some(f) if f != &e.node.name => format!("{f}#{}", e.node.name),
            _ => e.node.name.clone(),
        })
        .collect();
    dependents.sort();
    dependents.dedup();
    let total = dependents.len();
    dependents.truncate(15);
    a.graph = Some(GraphPlace {
        community: ex.community.map(|c| c.label),
        dependents,
        co_changes: ex
            .co_changes
            .iter()
            .take(8)
            .filter_map(|l| match l.kind {
                kor_graph::EdgeKind::CoChanges { ratio } => Some((l.node.id.clone(), ratio)),
                _ => None,
            })
            .collect(),
        dependents_total: total,
    });
    a.sources
        .push("project graph: community, dependents, co-change".into());
}

/// The model request for `level`, in `language` (`Türkçe`, `English`, `français`). Author names
/// and emails are replaced; the file's code is included (secrets masked, first 12 000 chars).
pub fn prompt(a: &Archaeology, level: Level, language: &str, source: &str) -> String {
    let style = match level {
        Level::Junior => {
            "The reader is a junior developer: plain words, short sentences, explain every technical term the first time, one everyday analogy, no jargon without a definition."
        }
        Level::Normal => {
            "The reader is an experienced developer new to this file: clear and direct."
        }
        Level::Expert => {
            "The reader is an expert: be terse, focus on design trade-offs, risks and where changes are dangerous."
        }
    };
    let mut facts = serde_json::to_value(a).unwrap_or_default();
    if let Some(list) = facts.get_mut("authors").and_then(|v| v.as_array_mut()) {
        for (i, x) in list.iter_mut().enumerate() {
            x["name"] = serde_json::json!(format!("Author {}", i + 1));
            x["email"] = serde_json::json!("");
        }
    }
    for key in ["first", "last"] {
        if let Some(c) = facts.get_mut(key).filter(|v| v.is_object()) {
            c["author"] = serde_json::json!("");
        }
    }
    for list in ["refactors"] {
        if let Some(arr) = facts.get_mut(list).and_then(|v| v.as_array_mut()) {
            for c in arr {
                c["author"] = serde_json::json!("");
            }
        }
    }
    if let Some(arr) = facts.get_mut("renames").and_then(|v| v.as_array_mut()) {
        for r in arr {
            r["commit"]["author"] = serde_json::json!("");
        }
    }
    if let Some(s) = facts.get_mut("stats").filter(|v| v.is_object()) {
        // Only the function table matters to the reader; names lists are long.
        s["names"] = serde_json::json!([]);
        s["undocumented"] = serde_json::json!([]);
    }
    let code: String = kor_memory::mask(source).chars().take(12_000).collect();
    format!(
        "Explain the file `{file}` to a developer, in {language}. {style}\n\
         Use only the facts below (from git, git blame, tree-sitter and the project graph) and the code; \
         do not invent history. Cover: what the file does, how it got here (age, renames, refactors), \
         who owns it (use the Author numbers), the danger zones with their line ranges and why they are risky, \
         and what depends on it. Keep it under 350 words. Markdown.\n\n\
         Facts (JSON):\n{facts}\n\nCode:\n```\n{code}\n```\n",
        file = a.file,
    )
}

#[cfg(test)]
#[allow(clippy::format_push_string, clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::analysis::metrics::fixture;

    #[test]
    fn archaeology_of_a_renamed_and_refactored_file() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        fixture::init(d);
        let mut body = String::from("pub fn route(x: i32) -> i32 {\n");
        for i in 0..8 {
            body.push_str(&format!("    if x == {i} {{ return {i}; }}\n"));
        }
        body.push_str("    0\n}\n");
        fixture::write(d, "src/old.rs", &body);
        fixture::commit(d, "feat: add router", "Ayşe Dev", "ayse@example.com", 400);
        std::fs::create_dir_all(d.join("src/net")).unwrap();
        std::fs::rename(d.join("src/old.rs"), d.join("src/net/router.rs")).unwrap();
        fixture::commit(
            d,
            "refactor: move router",
            "Bora Dev",
            "bora@example.com",
            100,
        );
        fixture::write(
            d,
            "src/net/router.rs",
            &body.replace("return 3", "return 33"),
        );
        fixture::commit(d, "fix: three", "Bora Dev", "bora@example.com", 10);

        let a = archaeology(d, "src/net/router.rs", None).unwrap();
        assert_eq!(a.commits, 3);
        assert_eq!(a.age_days, Some(400));
        assert_eq!(a.last_change_days, Some(10));
        assert_eq!(a.churn_90d, 1);
        assert_eq!(a.renames.len(), 1);
        assert_eq!(a.renames[0].from, "src/old.rs");
        assert_eq!(a.refactors.len(), 1);
        assert_eq!(a.authors.len(), 2);
        assert_eq!(a.authors[0].email, "ayse@example.com");
        assert!(a.authors[0].lines > a.authors[1].lines);
        let z = &a.danger[0];
        assert_eq!(
            (z.name.as_str(), z.line, z.complexity, z.commits),
            ("route", 1, 9, 2)
        );
        assert!(a.graph.is_some());

        let p = prompt(&a, Level::Junior, "English", "pub fn route() {}");
        assert!(p.contains("Author 1") && !p.contains("Ayşe") && !p.contains("bora@example.com"));
        assert!(p.contains("junior developer"));
        assert!(archaeology(d, "../outside.rs", None).is_err());
    }
}
