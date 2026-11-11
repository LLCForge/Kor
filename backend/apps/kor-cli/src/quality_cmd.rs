//! `kor quality`, `kor debt`, `kor style [check]`, `kor stats`, `kor impact` (§9, ADR 0047).
//! The measuring lives in `kor_runtime::analysis::metrics`; this prints it with the source of
//! every number and `≈` on every estimate.

use clap::{Args, Subcommand, ValueEnum};
use kor_i18n::{Localizer, args};
use kor_runtime::analysis::metrics::{self, debt, impact, quality, stats, style};
use kor_runtime::analysis::{self, Format};
use kor_runtime::review::DiffSource;

use crate::Ctx;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutFormat {
    Text,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FindingFormat {
    Md,
    Json,
    Sarif,
}

#[derive(Debug, Args)]
pub struct QualityArgs {
    /// Show the stored snapshots (one per commit) instead of measuring.
    #[arg(long)]
    pub history: bool,
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutFormat,
}

#[derive(Debug, Args)]
pub struct DebtArgs {
    /// Hours the projection runs to (default `[stats] debt_threshold_hours`).
    #[arg(long)]
    pub threshold: Option<u32>,
    /// Files listed.
    #[arg(long, default_value_t = 20)]
    pub top: usize,
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutFormat,
}

#[derive(Debug, Args)]
pub struct StyleArgs {
    #[command(subcommand)]
    pub command: Option<StyleCmd>,
    /// Profile this author (email) instead of `git config user.email`.
    #[arg(long)]
    pub author: Option<String>,
    /// Measure and print, but do not write the project memory.
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutFormat,
}

#[derive(Debug, Clone, Subcommand)]
pub enum StyleCmd {
    /// Flag added code that departs from the stored profile.
    Check {
        #[arg(long, conflicts_with = "range")]
        staged: bool,
        /// Commit range `A..B`.
        #[arg(long)]
        range: Option<String>,
        #[arg(long, value_enum, default_value = "md")]
        format: FindingFormat,
    },
}

#[derive(Debug, Args)]
pub struct StatsArgs {
    /// This week only (default: the last four weeks).
    #[arg(long, conflicts_with = "weeks")]
    pub week: bool,
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=52))]
    pub weeks: Option<u32>,
    #[arg(long)]
    pub author: Option<String>,
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutFormat,
}

#[derive(Debug, Args)]
pub struct ImpactArgs {
    /// A file or symbol; omit with --diff/--staged/--range.
    #[arg(required_unless_present_any = ["diff", "staged", "range"])]
    pub target: Option<String>,
    /// Uncommitted changes against HEAD.
    #[arg(long, conflicts_with_all = ["target", "staged", "range"])]
    pub diff: bool,
    #[arg(long, conflicts_with_all = ["target", "range"])]
    pub staged: bool,
    #[arg(long, conflicts_with = "target")]
    pub range: Option<String>,
    #[arg(long, default_value_t = impact::DEFAULT_DEPTH, value_parser = clap::value_parser!(u32).range(1..=10))]
    pub depth: u32,
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutFormat,
}

pub fn localize_quality(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-quality"))
}
pub fn localize_debt(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-debt"))
}
pub fn localize_style(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-style"))
        .mut_subcommand("check", |s| s.about(l.t("cmd-style-check")))
}
pub fn localize_stats(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-stats"))
}
pub fn localize_impact(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-impact"))
}

fn fail(l: &Localizer, e: impl std::fmt::Display) -> bool {
    eprintln!(
        "{}",
        l.t_args("metrics-error", &args!("error" => e.to_string()))
    );
    false
}

fn json(v: &impl serde::Serialize) -> anyhow::Result<bool> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(true)
}

fn num(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

fn short(c: &str) -> String {
    c.chars().take(10).collect()
}

// ---------------------------------------------------------------------------------------------
// kor quality
// ---------------------------------------------------------------------------------------------

pub fn quality(ctx: &Ctx, a: &QualityArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let root = kor_runtime::project_root()?;
    if a.history {
        let h = quality::history(&root);
        if a.format == OutFormat::Json {
            return json(&h);
        }
        if h.is_empty() {
            println!("{}", l.t("quality-history-none"));
        }
        for s in &h {
            println!(
                "{}",
                l.t_args(
                    "quality-history-row",
                    &args!("date" => stats::fmt_day(s.taken_at), "commit" => short(&s.commit),
                        "score" => s.overall.map_or_else(|| "-".into(), num),
                        "grade" => s.overall.map_or("-", |x| metrics::Grade::from_score(x).as_str()),
                        "debt" => num(s.debt_hours))
                )
            );
        }
        return Ok(true);
    }
    let r = quality::run(&root, l);
    if a.format == OutFormat::Json {
        return json(&r);
    }
    print_quality(l, &r);
    Ok(true)
}

fn dim_name(l: &Localizer, key: &str) -> String {
    l.t(&format!("quality-dim-{key}"))
}

pub(crate) fn print_quality(l: &Localizer, r: &quality::QualityReport) {
    let commit = r.commit.as_deref().map_or_else(|| "-".to_owned(), short);
    match (r.overall, r.grade) {
        (Some(s), Some(g)) => println!(
            "{}",
            l.t_args("quality-head", &args!("grade" => g.as_str(), "score" => num(s), "files" => r.files, "commit" => commit))
        ),
        _ => println!("{}", l.t("quality-head-none")),
    }
    if r.dirty {
        println!("{}", l.t("quality-dirty"));
    }
    for d in &r.dimensions {
        println!();
        match (d.score, d.grade) {
            (Some(s), Some(g)) => {
                println!(
                    "{}",
                    l.t_args("quality-dim", &args!("name" => dim_name(l, &d.key), "grade" => g.as_str(), "score" => num(s)))
                );
                let formula = if d.key == "documentation"
                    && d.metrics.first().is_some_and(|m| m.value == 0.0)
                {
                    "quality-formula-documentation-nopublic".to_owned()
                } else {
                    format!("quality-formula-{}", d.key)
                };
                println!(
                    "  {}",
                    l.t_args("quality-formula", &args!("formula" => l.t(&formula)))
                );
            }
            _ => println!(
                "{}",
                l.t_args("quality-dim-none", &args!("name" => dim_name(l, &d.key)))
            ),
        }
        if let Some(n) = &d.note {
            println!("  {}", l.t(&format!("quality-note-{n}")));
        }
        for m in &d.metrics {
            println!(
                "  {}",
                l.t_args(
                    "quality-metric",
                    &args!("label" => l.t(&format!("quality-m-{}", m.key)),
                        "value" => format!("{}{}", num(m.value), if m.unit == "%" { "%" } else { "" }),
                        "source" => m.source.clone())
                )
            );
        }
    }
    println!();
    println!(
        "{}",
        l.t_args("quality-debt", &args!("hours" => num(r.debt_hours)))
    );
    if let Some(t) = &r.trend {
        let sign = |v: f64| {
            if v > 0.0 {
                format!("+{}", num(v))
            } else {
                num(v)
            }
        };
        println!(
            "{}",
            l.t_args(
                "quality-trend",
                &args!("commit" => short(&t.since), "date" => stats::fmt_day(t.since_at),
                    "overall" => t.overall.map_or_else(|| "-".into(), sign), "debt" => sign(t.debt_hours))
            )
        );
        for (k, v) in &t.dimensions {
            println!("  {}: {}", dim_name(l, k), sign(*v));
        }
    }
    if !r.findings.is_empty() {
        println!();
        println!(
            "{}",
            l.t_args(
                "quality-improve",
                &args!("shown" => r.findings.len(), "total" => r.findings_total)
            )
        );
        for f in &r.findings {
            println!(
                "  [{}] {} {} ({})",
                f.severity.as_str(),
                f.location(),
                f.message,
                f.rule
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// kor debt
// ---------------------------------------------------------------------------------------------

fn threshold(ctx: &Ctx, a: Option<u32>) -> f64 {
    let cfg = ctx
        .config
        .as_ref()
        .map_or(0, |c| c.config.stats.debt_threshold_hours);
    let h = a.unwrap_or(cfg);
    if h == 0 {
        debt::DEFAULT_THRESHOLD_HOURS
    } else {
        f64::from(h)
    }
}

pub fn debt(ctx: &Ctx, a: &DebtArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let root = kor_runtime::project_root()?;
    let p = metrics::Project::gather(&root);
    let d = debt::debt(&root, &p, threshold(ctx, a.threshold));
    if a.format == OutFormat::Json {
        return json(&d);
    }
    print_debt(l, &d, a.top);
    Ok(true)
}

pub(crate) fn test_basis(l: &Localizer, t: &debt::TestBasis) -> String {
    match t {
        debt::TestBasis::Coverage { percent, source } => l.t_args(
            "debt-test-coverage",
            &args!("pct" => num(*percent), "source" => source.clone()),
        ),
        debt::TestBasis::NotInReport { source } => {
            l.t_args("debt-test-missing", &args!("source" => source.clone()))
        }
        debt::TestBasis::TestsFound => l.t("debt-test-yes"),
        debt::TestBasis::NoTests => l.t("debt-test-no"),
    }
}

fn print_debt(l: &Localizer, d: &debt::Debt, top: usize) {
    println!(
        "{}",
        l.t_args("debt-head", &args!("hours" => num(d.total_hours), "files" => d.items.len(), "scanned" => d.files_scanned))
    );
    println!("{}", l.t("debt-formula"));
    if d.items.is_empty() {
        println!("{}", l.t("debt-none"));
    }
    for i in d.items.iter().take(top) {
        println!(
            "  {}",
            l.t_args(
                "debt-item",
                &args!("file" => i.file.clone(), "hours" => num(i.hours), "cc" => i.complexity_excess,
                    "lines" => i.code_lines, "dup" => i.duplicated_lines, "churn" => i.churn,
                    "test" => test_basis(l, &i.test))
            )
        );
        if let Some((name, line, cc)) = &i.worst {
            println!(
                "      {}",
                l.t_args(
                    "debt-worst",
                    &args!("name" => name.clone(), "line" => *line, "cc" => *cc)
                )
            );
        }
    }
    match &d.growth {
        Some(g) => {
            let basis = match &g.basis {
                debt::GrowthBasis::Snapshots { count, from, to } => l.t_args(
                    "debt-basis-snapshots",
                    &args!("count" => *count, "from" => from.clone(), "to" => to.clone()),
                ),
                debt::GrowthBasis::Git { rev } => {
                    l.t_args("debt-basis-git", &args!("rev" => rev.clone()))
                }
            };
            println!(
                "{}",
                l.t_args(
                    "debt-growth",
                    &args!("hours" => num(g.per_month), "basis" => basis)
                )
            );
        }
        None => println!("{}", l.t("debt-growth-none")),
    }
    let th = num(d.threshold_hours);
    match d.months_to_threshold {
        Some(m) if m <= 0.0 => println!(
            "{}",
            l.t_args("debt-threshold-reached", &args!("threshold" => th))
        ),
        Some(m) => println!(
            "{}",
            l.t_args(
                "debt-threshold-months",
                &args!("threshold" => th, "months" => num(m))
            )
        ),
        None => println!(
            "{}",
            l.t_args("debt-threshold-none", &args!("threshold" => th))
        ),
    }
}

// ---------------------------------------------------------------------------------------------
// kor style
// ---------------------------------------------------------------------------------------------

fn home(l: &Localizer) -> Option<std::path::PathBuf> {
    let h = kor_runtime::kor_home();
    if h.is_none() {
        eprintln!("{}", l.t("memory-no-home"));
    }
    h
}

fn render(l: &Localizer, r: &analysis::Report, f: FindingFormat) {
    let format = match f {
        FindingFormat::Md => Format::Markdown,
        FindingFormat::Json => Format::Json,
        FindingFormat::Sarif => Format::Sarif,
    };
    print!("{}", analysis::render(r, format, l));
}

pub fn style(ctx: &Ctx, a: &StyleArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let root = kor_runtime::project_root()?;
    let Some(home) = home(l) else {
        return Ok(false);
    };
    if let Some(StyleCmd::Check {
        staged,
        range,
        format,
    }) = &a.command
    {
        let src = match (range, staged) {
            (Some(r), _) => DiffSource::Range(r.clone()),
            (None, true) => DiffSource::Staged,
            (None, false) => DiffSource::Working,
        };
        return Ok(match style::check(&home, &root, &src, l) {
            Ok(r) if r.findings.is_empty() && *format == FindingFormat::Md => {
                println!("{}", l.t("style-check-clean"));
                true
            }
            Ok(r) => {
                render(l, &r, *format);
                true
            }
            Err(e) => fail(l, e),
        });
    }
    let p = match style::profile(&root, a.author.as_deref()) {
        Ok(p) => p,
        Err(e) => return Ok(fail(l, e)),
    };
    if a.format == OutFormat::Json {
        return json(&p);
    }
    print_style(l, &p);
    if !a.dry_run {
        match style::save(&home, &root, &p) {
            Ok(true) => println!("{}", l.t("style-saved")),
            Ok(false) => println!("{}", l.t("style-saved-kept")),
            Err(e) => return Ok(fail(l, e)),
        }
    }
    Ok(true)
}

fn print_style(l: &Localizer, p: &style::StyleProfile) {
    println!(
        "{}",
        l.t_args("style-head", &args!("author" => p.author.clone(), "files" => p.files, "commits" => p.commits, "lines" => p.lines))
    );
    println!("{}", l.t("style-naming"));
    for c in &p.naming {
        println!(
            "  {}",
            l.t_args(
                "style-naming-row",
                &args!("lang" => c.lang.clone(), "category" => c.category.clone(), "conv" => c.dominant.clone(),
                    "share" => num(c.share * 100.0), "samples" => c.samples)
            )
        );
    }
    if !p.errors.is_empty() {
        let list: Vec<String> = p.errors.iter().map(|(k, v)| format!("{k} ×{v}")).collect();
        println!(
            "{}",
            l.t_args("style-errors", &args!("list" => list.join(", ")))
        );
    }
    println!(
        "{}",
        l.t_args(
            "style-functions",
            &args!("count" => p.functions, "early" => num(p.early_return_share * 100.0),
                "lines" => num(p.avg_function_lines), "cc" => num(p.avg_complexity))
        )
    );
    println!(
        "{}",
        l.t_args("style-comments", &args!("density" => format!("{:.2}", p.comment_density), "docs" => num(p.doc_share * 100.0)))
    );
    if !p.preferences.is_empty() {
        println!("{}", l.t("style-prefs"));
        for x in &p.preferences {
            println!("  {x}");
        }
    }
    println!(
        "{}",
        l.t_args("style-source", &args!("source" => p.source.clone()))
    );
}

// ---------------------------------------------------------------------------------------------
// kor stats
// ---------------------------------------------------------------------------------------------

pub fn stats_cmd(ctx: &Ctx, a: &StatsArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let root = kor_runtime::project_root()?;
    let team = ctx.config.as_ref().is_ok_and(|c| c.config.stats.team);
    let o = stats::Options {
        author: a.author.clone(),
        weeks: if a.week { 1 } else { a.weeks.unwrap_or(4) },
        team,
        audit: kor_runtime::secure::audit_path(),
    };
    let s = match stats::stats(&root, &o) {
        Ok(s) => s,
        Err(e) => return Ok(fail(l, e)),
    };
    if a.format == OutFormat::Json {
        return json(&s);
    }
    print_stats(l, &s);
    Ok(true)
}

fn print_stats(l: &Localizer, s: &stats::Stats) {
    println!(
        "{}",
        l.t_args("stats-head", &args!("author" => s.author.clone()))
    );
    println!("{}", l.t("stats-private"));
    for w in &s.weeks {
        println!();
        println!(
            "{}",
            l.t_args(
                "stats-week",
                &args!("from" => stats::fmt_day(w.start), "to" => stats::fmt_day(w.end - 1))
            )
        );
        println!(
            "  {}",
            l.t_args(
                "stats-commits",
                &args!("commits" => w.commits, "days" => w.active_days)
            )
        );
        println!(
            "  {}",
            l.t_args(
                "stats-human",
                &args!("added" => w.human.added, "deleted" => w.human.deleted)
            )
        );
        println!(
            "  {}",
            l.t_args(
                "stats-assisted",
                &args!("added" => w.assisted.added, "deleted" => w.assisted.deleted)
            )
        );
        println!(
            "  {}",
            l.t_args("stats-agent", &args!("added" => w.agent.added, "deleted" => w.agent.deleted, "commits" => w.agent_commits, "writes" => w.agent_writes))
        );
        println!(
            "  {}",
            l.t_args(
                "stats-types",
                &args!("fixes" => w.fixes, "features" => w.features)
            )
        );
        match w.coverage {
            Some((a, b)) => println!(
                "  {}",
                l.t_args(
                    "stats-coverage",
                    &args!("from" => num(a), "to" => num(b), "delta" => num(b - a))
                )
            ),
            None => println!("  {}", l.t("stats-coverage-none")),
        }
    }
    println!();
    println!(
        "{}",
        l.t_args(
            "stats-streak",
            &args!("days" => s.streak, "longest" => s.longest_streak)
        )
    );
    match &s.team {
        Some(rows) => {
            println!("{}", l.t("stats-team-head"));
            for (i, r) in rows.iter().enumerate() {
                println!(
                    "  {}",
                    l.t_args(
                        "stats-team-row",
                        &args!("rank" => i + 1, "author" => r.author.clone(), "commits" => r.commits, "added" => r.added,
                            "deleted" => r.deleted, "fixes" => r.fixes, "features" => r.features)
                    )
                );
            }
        }
        None => println!("{}", l.t("stats-team-off")),
    }
    println!(
        "{}",
        l.t_args("stats-sources", &args!("list" => s.sources.join("; ")))
    );
}

// ---------------------------------------------------------------------------------------------
// kor impact
// ---------------------------------------------------------------------------------------------

pub fn impact_cmd(ctx: &Ctx, a: &ImpactArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let root = kor_runtime::project_root()?;
    let target = match (&a.target, &a.range, a.staged) {
        (Some(t), _, _) => impact::Target::Name(t.clone()),
        (None, Some(r), _) => impact::Target::Diff(DiffSource::Range(r.clone())),
        (None, None, true) => impact::Target::Diff(DiffSource::Staged),
        (None, None, false) => impact::Target::Diff(DiffSource::Working),
    };
    let g = match metrics::graph(&root) {
        Ok(g) => g,
        Err(e) => return Ok(fail(l, e)),
    };
    let r = match impact::impact(&root, &g, &target, a.depth) {
        Ok(r) => r,
        Err(e) => return Ok(fail(l, e)),
    };
    if a.format == OutFormat::Json {
        return json(&r);
    }
    print_impact(l, &r);
    Ok(true)
}

fn list(v: &[String], max: usize) -> String {
    let mut s = v.iter().take(max).cloned().collect::<Vec<_>>().join(", ");
    if v.len() > max {
        s.push_str(", …");
    }
    s
}

fn print_impact(l: &Localizer, r: &impact::ImpactReport) {
    println!(
        "{}",
        l.t_args(
            "impact-head",
            &args!("target" => r.target.clone(), "depth" => r.depth)
        )
    );
    if !r.not_found.is_empty() {
        println!(
            "{}",
            l.t_args("impact-not-found", &args!("list" => list(&r.not_found, 10)))
        );
    }
    println!(
        "  {}",
        l.t_args(
            "impact-files",
            &args!("count" => r.files.len(), "list" => list(&r.files, 8))
        )
    );
    println!(
        "  {}",
        l.t_args("impact-symbols", &args!("count" => r.symbols))
    );
    println!(
        "  {}",
        l.t_args("impact-endpoints", &args!("count" => r.endpoints.len()))
    );
    for e in r.endpoints.iter().take(15) {
        println!("    {}:{} {} ({})", e.file, e.line, e.name, e.rule);
    }
    println!(
        "  {}",
        l.t_args(
            "impact-migrations",
            &args!("count" => r.migrations.len(), "list" => list(&r.migrations, 8))
        )
    );
    println!(
        "  {}",
        l.t_args(
            "impact-tables",
            &args!("count" => r.tables.len(), "list" => list(&r.tables, 8))
        )
    );
    println!(
        "  {}",
        l.t_args(
            "impact-packages",
            &args!("count" => r.packages.len(), "list" => list(&r.packages, 8))
        )
    );
    println!(
        "  {}",
        l.t_args(
            "impact-tests",
            &args!("count" => r.tests.len(), "list" => list(&r.tests, 8))
        )
    );
    if r.truncated {
        println!("  {}", l.t("impact-truncated"));
    }
    println!(
        "{}",
        l.t_args("impact-hours", &args!("hours" => num(r.hours)))
    );
    println!("  {}", l.t("impact-formula"));
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct Q {
        #[command(flatten)]
        a: QualityArgs,
    }
    #[derive(Parser)]
    struct D {
        #[command(flatten)]
        a: DebtArgs,
    }
    #[derive(Parser)]
    struct S {
        #[command(flatten)]
        a: StyleArgs,
    }
    #[derive(Parser)]
    struct St {
        #[command(flatten)]
        a: StatsArgs,
    }
    #[derive(Parser)]
    struct I {
        #[command(flatten)]
        a: ImpactArgs,
    }

    #[test]
    fn parses() {
        assert!(
            Q::try_parse_from(["quality", "--history", "--format", "json"])
                .unwrap()
                .a
                .history
        );
        assert_eq!(
            D::try_parse_from(["debt", "--threshold", "300"])
                .unwrap()
                .a
                .threshold,
            Some(300)
        );
        let s = S::try_parse_from(["style", "check", "--staged", "--format", "sarif"]).unwrap();
        assert!(matches!(
            s.a.command,
            Some(StyleCmd::Check {
                staged: true,
                format: FindingFormat::Sarif,
                ..
            })
        ));
        assert!(
            S::try_parse_from(["style", "--author", "a@b.c", "--dry-run"])
                .unwrap()
                .a
                .dry_run
        );
        assert!(St::try_parse_from(["stats", "--week"]).unwrap().a.week);
        assert!(St::try_parse_from(["stats", "--week", "--weeks", "3"]).is_err());
        assert!(I::try_parse_from(["impact", "src/db.rs", "--depth", "2"]).is_ok());
        assert!(I::try_parse_from(["impact", "--staged"]).is_ok());
        assert!(I::try_parse_from(["impact"]).is_err());
        assert!(I::try_parse_from(["impact", "x", "--diff"]).is_err());
    }
}
