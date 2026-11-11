//! Rule compliance checker, runtime side (Master Build Prompt §28.3, ADR 0063): the rule set of
//! a project (every tool's instruction files Kor loads), `kor rules check` on the working tree,
//! and the model's judgment for prose rules. The deterministic checks live in
//! `kor_tools::rules_check`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kor_compat::{Applies, Compat};
use kor_providers::Provider;
use kor_tools::rules_check::{Change, Doc, Finding, RuleSet, Verdict};
use serde_json::Value;

/// Diff characters given to the judging model.
const MAX_DIFF: usize = 40_000;
/// Rule lines given to the judging model.
const MAX_RULES: usize = 120;
/// Changed files checked at most.
const MAX_FILES: usize = 500;
const MAX_FILE: u64 = 1024 * 1024;

/// `<kor home>/rules-log.jsonl`.
pub fn log_path() -> Option<PathBuf> {
    crate::kor_home().map(|h| h.join("rules-log.jsonl"))
}

/// Rule documents of a project: instructions that always apply or apply to file globs.
pub fn docs(c: &Compat) -> Vec<Doc> {
    c.instructions
        .iter()
        .filter_map(|i| {
            let globs = match &i.applies {
                Applies::Always => None,
                Applies::Globs { globs } => Some(globs.clone()),
                // Listed by description / named by the user: not checked on every edit.
                Applies::OnRequest { .. } | Applies::Manual => return None,
            };
            Some(Doc {
                source: i.source.path.clone(),
                text: i.text.clone(),
                globs,
            })
        })
        .collect()
}

/// The rule set of `compat` (violations logged to [`log_path`]).
pub fn rule_set(c: &Compat) -> Arc<RuleSet> {
    Arc::new(RuleSet::new(&docs(c), log_path()))
}

/// How a rule source is shown: relative to `root` when inside it, else with `~`.
pub fn display(root: &Path) -> impl Fn(&Path) -> String + '_ {
    move |p: &Path| {
        p.strip_prefix(root).map_or_else(
            |_| kor_tools::display_path(p),
            |r| r.to_string_lossy().replace('\\', "/"),
        )
    }
}

/// Changed files of the working tree against `base` (default `HEAD`; `staged` = the index
/// against `HEAD`): tracked changes and untracked files, text only.
///
/// # Errors
/// When `root` is not a git repository or git fails.
pub fn working_changes(
    root: &Path,
    base: Option<&str>,
    staged: bool,
) -> Result<Vec<Change>, String> {
    let g = |args: &[&str]| kor_agent::gitflow::git(root, args).map_err(|e| e.to_string());
    let top = PathBuf::from(g(&["rev-parse", "--show-toplevel"])?.trim());
    let base = base.unwrap_or("HEAD");
    let mut names: Vec<String> = if staged {
        g(&["diff", "--cached", "--name-only", "-z"])?
    } else {
        g(&["diff", "--name-only", "-z", base, "--"])?
    }
    .split('\0')
    .filter(|s| !s.is_empty())
    .map(str::to_owned)
    .collect();
    if !staged {
        names.extend(
            g(&["ls-files", "--others", "--exclude-standard", "-z"])?
                .split('\0')
                .filter(|s| !s.is_empty() && !s.starts_with(".kor/"))
                .map(str::to_owned),
        );
    }
    names.sort();
    names.dedup();
    let mut out = Vec::new();
    for n in names.into_iter().take(MAX_FILES) {
        let rev = if staged { "HEAD" } else { base };
        let before = g(&["show", &format!("{rev}:{n}")]).ok();
        let after = if staged {
            g(&["show", &format!(":{n}")]).ok()
        } else {
            let p = top.join(&n);
            std::fs::metadata(&p)
                .ok()
                .filter(|m| m.is_file() && m.len() <= MAX_FILE)
                .and_then(|_| std::fs::read_to_string(&p).ok())
        };
        out.push(Change {
            path: n,
            before,
            after,
        });
    }
    Ok(out)
}

/// The judging prompt: numbered prose rules and the diff; the answer is JSON.
pub fn judge_prompt(rules: &[&Finding], changes: &[Change]) -> String {
    let mut diff = String::new();
    for c in changes {
        diff.push_str(&crate::textdiff::unified(
            &c.path,
            c.before.as_deref(),
            c.after.as_deref(),
        ));
        if diff.len() > MAX_DIFF {
            let mut end = MAX_DIFF;
            while !diff.is_char_boundary(end) {
                end -= 1;
            }
            diff.truncate(end);
            diff.push_str("\n[…]\n");
            break;
        }
    }
    let mut p = String::from(
        "You check a code change against the project's rules. For every numbered rule decide:\n\
         \"applied\" (the change follows it), \"relevant\" (it applies but the change neither \
         follows nor breaks it clearly) or \"violated\" (the change breaks it), and skip rules that \
         do not concern this change. Answer ONLY a JSON array: \
         [{\"rule\": <number>, \"verdict\": \"applied|relevant|violated\", \"file\": \"path or null\", \
         \"line\": <new-file line or null>, \"message\": \"one short sentence\"}]\n\nRules:\n",
    );
    for (i, r) in rules.iter().take(MAX_RULES).enumerate() {
        let _ = writeln!(p, "{}. {} ({})", i + 1, r.rule, r.location());
    }
    p.push_str("\nChange (unified diff):\n");
    p.push_str(&diff);
    p
}

/// The model's answer as findings (unknown rule numbers and malformed items are dropped).
pub fn parse_judgement(answer: &str, rules: &[&Finding]) -> Vec<Finding> {
    let start = answer.find('[');
    let end = answer.rfind(']');
    let (Some(s), Some(e)) = (start, end) else {
        return Vec::new();
    };
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&answer[s..=e]) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|it| {
            let n = usize::try_from(it.get("rule")?.as_u64()?).ok()?;
            let r = rules.get(n.checked_sub(1)?)?;
            let verdict = match it.get("verdict")?.as_str()? {
                "applied" => Verdict::Applied,
                "relevant" => Verdict::Relevant,
                "violated" => Verdict::Violated,
                _ => return None,
            };
            Some(Finding {
                verdict,
                rule: r.rule.clone(),
                source: r.source.clone(),
                line: r.line,
                file: it.get("file").and_then(Value::as_str).map(str::to_owned),
                file_line: it
                    .get("line")
                    .and_then(Value::as_u64)
                    .and_then(|l| u32::try_from(l).ok()),
                message: it
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .chars()
                    .take(300)
                    .collect(),
                deterministic: false,
            })
        })
        .collect()
}

/// One model call: the answer text and the usage the provider reported.
async fn ask(
    provider: &Arc<dyn Provider>,
    model: &str,
    prompt: String,
) -> Result<(String, Option<kor_core::Usage>), String> {
    use futures::StreamExt as _;
    let req = kor_providers::ChatRequest {
        model: model.to_owned(),
        system: Some(
            "You check code changes against written rules. Answer only with the JSON asked for."
                .into(),
        ),
        messages: vec![kor_core::Message::text(kor_core::Role::User, prompt)],
        max_tokens: Some(4_000),
        ..kor_providers::ChatRequest::default()
    };
    let mut stream = provider
        .complete(req, kor_providers::CancellationToken::new())
        .await
        .map_err(|e| e.message)?;
    let mut text = String::new();
    let mut usage = None;
    while let Some(ev) = stream.next().await {
        match ev.map_err(|e| e.message)? {
            kor_core::StreamEvent::TextDelta { text: t } => text.push_str(&t),
            kor_core::StreamEvent::Usage { usage: u } => usage = Some(u),
            _ => {}
        }
    }
    if text.trim().is_empty() {
        return Err("empty reply".into());
    }
    Ok((text, usage))
}

/// Ask `provider`/`model` to judge the prose rules among `findings` (the yellow ones); the
/// deterministic findings stay as they are. Returns the merged list.
///
/// # Errors
/// The model call failed.
pub async fn judge(
    provider: &Arc<dyn Provider>,
    model: &str,
    findings: Vec<Finding>,
    changes: &[Change],
) -> Result<Vec<Finding>, String> {
    let prose: Vec<&Finding> = findings.iter().filter(|f| !f.deterministic).collect();
    if prose.is_empty() {
        return Ok(findings);
    }
    let (answer, usage) = ask(provider, model, judge_prompt(&prose, changes)).await?;
    // §28.3: the judge's cost is booked under its own role, so the cost dashboard shows it apart.
    if let (Some(u), Some(ledger)) = (usage, crate::usage::file()) {
        let e = crate::usage::Entry {
            at: crate::cost::now_ms(),
            model: format!("{}/{model}", provider.id()),
            prompt: u.input,
            completion: u.output,
            cost_usd: None,
            path: None,
            role: Some("rules:judge".into()),
            media: false,
            estimated: false,
            cache_read: 0,
            cache_write: 0,
        };
        let _ = crate::usage::append(&ledger, &e);
    }
    let judged = parse_judgement(&answer, &prose);
    let mut out: Vec<Finding> = findings
        .iter()
        .filter(|f| f.deterministic)
        .cloned()
        .collect();
    out.extend(judged);
    out.sort_by(|a, b| b.verdict.cmp(&a.verdict));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kor_compat::{Eco, Instruction, Scope, Source};

    fn f(rule: &str) -> Finding {
        Finding {
            verdict: Verdict::Relevant,
            rule: rule.into(),
            source: "AGENTS.md".into(),
            line: 4,
            file: None,
            file_line: None,
            message: String::new(),
            deterministic: false,
        }
    }

    #[test]
    fn docs_skip_on_request_rules() {
        let src = |p: &str| Source {
            eco: Eco::Claude,
            scope: Scope::Project,
            path: PathBuf::from(p),
        };
        let c = Compat {
            instructions: vec![
                Instruction {
                    source: src("CLAUDE.md"),
                    applies: Applies::Always,
                    text: "- never `x`".into(),
                },
                Instruction {
                    source: src(".cursor/rules/ts.mdc"),
                    applies: Applies::Globs {
                        globs: vec!["*.ts".into()],
                    },
                    text: "a".into(),
                },
                Instruction {
                    source: src(".cursor/rules/m.mdc"),
                    applies: Applies::Manual,
                    text: "b".into(),
                },
            ],
            ..Compat::default()
        };
        let d = docs(&c);
        assert_eq!(d.len(), 2);
        assert_eq!(d[1].globs.as_deref(), Some(&["*.ts".to_owned()][..]));
    }

    #[test]
    fn prompt_and_judgement_round_trip() {
        let rules = [
            f("Keep functions short."),
            f("Write docs for public items."),
        ];
        let refs: Vec<&Finding> = rules.iter().collect();
        let p = judge_prompt(
            &refs,
            &[Change {
                path: "a.rs".into(),
                before: None,
                after: Some("pub fn x() {}\n".into()),
            }],
        );
        assert!(
            p.contains("1. Keep functions short. (AGENTS.md:4)") && p.contains("+pub fn x() {}")
        );
        let ans = "Here: [{\"rule\": 2, \"verdict\": \"violated\", \"file\": \"a.rs\", \"line\": 1, \"message\": \"no doc comment\"}, {\"rule\": 9, \"verdict\": \"applied\"}, {\"rule\": 1, \"verdict\": \"maybe\"}]";
        let j = parse_judgement(ans, &refs);
        assert_eq!(j.len(), 1);
        assert_eq!((j[0].verdict, j[0].file_line), (Verdict::Violated, Some(1)));
        assert_eq!(j[0].rule, "Write docs for public items.");
        assert!(parse_judgement("no json", &refs).is_empty());
    }

    /// A model that always answers `reply`.
    pub(crate) struct Scripted {
        manifest: kor_providers::ProviderManifest,
        reply: String,
    }

    #[async_trait::async_trait]
    impl Provider for Scripted {
        fn manifest(&self) -> &kor_providers::ProviderManifest {
            &self.manifest
        }
        async fn list_models(
            &self,
        ) -> Result<Vec<kor_providers::ModelInfo>, kor_core::ProviderError> {
            Ok(Vec::new())
        }
        async fn complete(
            &self,
            _req: kor_providers::ChatRequest,
            _c: kor_providers::CancellationToken,
        ) -> Result<kor_providers::EventStream, kor_core::ProviderError> {
            let text = self.reply.clone();
            Ok(Box::pin(futures::stream::iter(vec![Ok(
                kor_core::StreamEvent::TextDelta { text },
            )])))
        }
    }

    pub(crate) fn scripted(reply: &str) -> Arc<dyn Provider> {
        let manifest = kor_providers::Registry::builtin()
            .get("local")
            .unwrap()
            .clone();
        Arc::new(Scripted {
            manifest,
            reply: reply.into(),
        })
    }

    #[tokio::test]
    async fn judge_with_a_scripted_model() {
        let provider =
            scripted("[{\"rule\": 1, \"verdict\": \"applied\", \"message\": \"short\"}]");
        let det = Finding {
            verdict: Verdict::Violated,
            deterministic: true,
            ..f("Never `unwrap()`")
        };
        let out = judge(&provider, "m", vec![f("Keep functions short."), det], &[])
            .await
            .unwrap();
        assert_eq!(out[0].verdict, Verdict::Violated);
        assert_eq!(out[1].verdict, Verdict::Applied);
    }

    #[test]
    fn working_tree_changes() {
        let d = tempfile::tempdir().unwrap();
        let g = |args: &[&str]| kor_agent::gitflow::git(d.path(), args).unwrap();
        g(&["init", "-q"]);
        std::fs::write(d.path().join("a.rs"), "fn a() {}\n").unwrap();
        g(&["add", "."]);
        g(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "i",
        ]);
        std::fs::write(d.path().join("a.rs"), "fn a() { x.unwrap(); }\n").unwrap();
        std::fs::write(d.path().join("b.rs"), "new\n").unwrap();
        let c = working_changes(d.path(), None, false).unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].before.as_deref(), Some("fn a() {}\n"));
        assert!(c[1].before.is_none());
        g(&["add", "b.rs"]);
        let s = working_changes(d.path(), None, true).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].path, "b.rs");
    }
}
