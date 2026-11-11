//! `kor impact <file|symbol|--diff>`: before changing anything, what the change reaches through
//! the project graph (`graph_impact`): files, symbols, HTTP endpoints (the dead-code route
//! rules), migrations and tables (database layer), workspace packages (monorepo layer) and
//! tests, with an effort estimate whose formula is printed next to it.

use std::collections::BTreeSet;
use std::path::Path;

use globset::{Glob, GlobSet, GlobSetBuilder};
use kor_graph::{EdgeKind, Graph, NodeKind};
use serde::{Deserialize, Serialize};

use super::{code, git, round1};
use crate::review::DiffSource;

pub const FORMULA: &str = "hours ≈ 0.5 × changed files + 0.2 × affected files + 0.5 × endpoints + 1.0 × migrations + 0.5 × (packages − 1)";
pub const DEFAULT_DEPTH: u32 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A file or a symbol name, as `kor graph impact` takes it.
    Name(String),
    Diff(DiffSource),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    pub rule: String,
    pub file: String,
    pub line: u32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImpactReport {
    pub target: String,
    /// What is being changed (files of a diff, or the one target).
    pub changed: Vec<String>,
    /// Targets the graph does not know.
    pub not_found: Vec<String>,
    /// Affected files, changed ones excluded.
    pub files: Vec<String>,
    pub symbols: usize,
    pub endpoints: Vec<Endpoint>,
    pub migrations: Vec<String>,
    pub tables: Vec<String>,
    pub packages: Vec<String>,
    pub tests: Vec<String>,
    pub hours: f64,
    pub formula: String,
    pub depth: u32,
    pub truncated: bool,
}

fn migration_globs() -> GlobSet {
    let mut b = GlobSetBuilder::new();
    for g in [
        "**/migrations/**",
        "**/migrate/**",
        "**/alembic/versions/**",
        "**/db/changelog/**",
        "**/flyway/**",
        "**/prisma/migrations/**",
        "**/database/migrations/**",
    ] {
        if let Ok(g) = Glob::new(g) {
            b.add(g);
        }
    }
    b.build().unwrap_or_else(|_| GlobSet::empty())
}

/// Files a diff touches (`git diff --name-only`).
pub fn diff_files(root: &Path, src: &DiffSource) -> Result<Vec<String>, String> {
    let out = match src {
        DiffSource::Working => git::git(root, &["diff", "--name-only", "HEAD", "--"])?,
        DiffSource::Staged => git::git(root, &["diff", "--name-only", "--cached", "--"])?,
        DiffSource::Range(r) => {
            if r.trim().is_empty() || r.starts_with('-') || r.chars().any(char::is_whitespace) {
                return Err(format!("invalid range: {r}"));
            }
            git::git(root, &["diff", "--name-only", r, "--"])?
        }
    };
    Ok(out
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect())
}

/// Impact of `target` over the graph, `depth` hops of dependents.
#[allow(clippy::too_many_lines)] // one pass that fills each counter of the report
pub fn impact(root: &Path, g: &Graph, target: &Target, depth: u32) -> Result<ImpactReport, String> {
    let (label, changed) = match target {
        Target::Name(n) => {
            let p = Path::new(n);
            let rel = if p.is_absolute() {
                super::rel(
                    &root.canonicalize().unwrap_or_else(|_| root.to_owned()),
                    &p.canonicalize().unwrap_or_else(|_| p.to_owned()),
                )
            } else {
                n.replace('\\', "/").trim_start_matches("./").to_owned()
            };
            (n.clone(), vec![rel])
        }
        Target::Diff(src) => {
            let label = match src {
                DiffSource::Working => "working tree".to_owned(),
                DiffSource::Staged => "staged changes".to_owned(),
                DiffSource::Range(r) => r.clone(),
            };
            (label, diff_files(root, src)?)
        }
    };
    let mut ids: BTreeSet<String> = BTreeSet::new();
    let mut files: BTreeSet<String> = BTreeSet::new();
    let mut root_files: BTreeSet<String> = BTreeSet::new();
    let mut not_found = Vec::new();
    let mut truncated = false;
    for c in &changed {
        let imp = g.impact(c, depth);
        if imp.roots.is_empty() {
            not_found.push(c.clone());
            continue;
        }
        truncated |= imp.truncated;
        for r in &imp.roots {
            ids.insert(r.id.clone());
            if let Some(f) = &r.file {
                root_files.insert(f.clone());
            }
        }
        for a in &imp.affected {
            ids.insert(a.node.id.clone());
        }
        files.extend(imp.files);
    }
    let changed_set: BTreeSet<String> = changed
        .iter()
        .filter(|c| root.join(c.as_str()).is_file())
        .cloned()
        .chain(root_files.iter().cloned())
        .collect();
    let affected_files: Vec<String> = files.difference(&changed_set).cloned().collect();
    let symbols = ids
        .iter()
        .filter_map(|i| g.node(i))
        .filter(|n| {
            matches!(
                n.kind,
                NodeKind::Function | NodeKind::Struct | NodeKind::Trait
            )
        })
        .count();
    let endpoints: Vec<Endpoint> = super::deadcode::routes(root, g, &ids)
        .into_iter()
        .map(|(rule, file, line, name)| Endpoint {
            rule,
            file,
            line,
            name,
        })
        .collect();
    let tables: BTreeSet<String> = ids
        .iter()
        .filter_map(|i| g.node(i))
        .filter(|n| n.kind == NodeKind::Table)
        .map(|n| n.name.clone())
        .collect();
    let mig = migration_globs();
    let table_ids: BTreeSet<&str> = ids
        .iter()
        .filter(|i| g.node(i).is_some_and(|n| n.kind == NodeKind::Table))
        .map(String::as_str)
        .collect();
    let mut migrations: BTreeSet<String> = changed_set
        .iter()
        .chain(&affected_files)
        .filter(|f| mig.is_match(f.as_str()))
        .cloned()
        .collect();
    for e in g.edges() {
        if e.kind == EdgeKind::Contains
            && table_ids.contains(e.target.as_str())
            && mig.is_match(&e.source)
        {
            migrations.insert(e.source.clone());
        }
    }
    let all_files: Vec<String> = changed_set.iter().chain(&affected_files).cloned().collect();
    let packages: Vec<String> = match g.workspace() {
        Some(ws) => ws
            .affected(&all_files)
            .into_iter()
            .map(|p| p.name.clone())
            .collect(),
        None => Vec::new(),
    };
    let tests: Vec<String> = all_files
        .iter()
        .filter(|f| code::is_test_path(f))
        .cloned()
        .collect();
    #[allow(clippy::cast_precision_loss)]
    let hours = 0.5 * changed_set.len() as f64
        + 0.2 * affected_files.len() as f64
        + 0.5 * endpoints.len() as f64
        + migrations.len() as f64
        + 0.5 * packages.len().saturating_sub(1) as f64;
    Ok(ImpactReport {
        target: label,
        changed,
        not_found,
        files: affected_files,
        symbols,
        endpoints,
        migrations: migrations.into_iter().collect(),
        tables: tables.into_iter().collect(),
        packages,
        tests,
        hours: round1(hours),
        formula: FORMULA.to_owned(),
        depth,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_migrations_and_estimate() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        crate::analysis::metrics::fixture::write(d, "manage.py", "");
        crate::analysis::metrics::fixture::write(
            d,
            "shop/models.py",
            "def total(cart):\n    return sum(cart)\n",
        );
        crate::analysis::metrics::fixture::write(
            d,
            "shop/views.py",
            "from shop.models import total\n\ndef checkout(request):\n    return total([1])\n",
        );
        crate::analysis::metrics::fixture::write(
            d,
            "shop/migrations/0001_initial.sql",
            "CREATE TABLE orders (id INTEGER PRIMARY KEY);\n",
        );
        crate::analysis::metrics::fixture::write(
            d,
            "shop/repo.py",
            "def load(db):\n    return db.execute(\"SELECT id FROM orders\")\n",
        );
        crate::analysis::metrics::fixture::write(
            d,
            "tests/test_views.py",
            "from shop.views import checkout\n\ndef test_checkout():\n    assert checkout(None)\n",
        );
        let g = crate::analysis::metrics::graph(d).unwrap();

        let r = impact(d, &g, &Target::Name("shop/models.py".into()), DEFAULT_DEPTH).unwrap();
        assert!(r.files.contains(&"shop/views.py".to_owned()), "{r:?}");
        assert!(
            r.endpoints.iter().any(|e| e.file == "shop/views.py"),
            "{:?}",
            r.endpoints
        );
        assert!(r.hours > 0.0);
        assert_eq!(r.formula, FORMULA);

        let t = impact(d, &g, &Target::Name("orders".into()), DEFAULT_DEPTH).unwrap();
        assert_eq!(t.tables, ["orders"]);
        assert_eq!(t.migrations, ["shop/migrations/0001_initial.sql"]);
        assert!(t.files.contains(&"shop/repo.py".to_owned()), "{t:?}");

        let missing = impact(d, &g, &Target::Name("no_such_thing".into()), 2).unwrap();
        assert_eq!(missing.not_found, ["no_such_thing"]);
    }
}
