//! Verified edit loop (spec 8.1): after a file-changing tool call, the project's own checker
//! looks at the changed files and its errors go back to the model in the tool result.
//!
//! Checkers are picked from project files, run as argv (no shell), inside the project root,
//! offline, with API keys removed from the environment, a timeout and capped output.
//! Checkers that can run project code (`cargo check` runs build scripts and proc macros,
//! `tsc` comes from the project's `node_modules`, `go vet` may invoke cgo) are marked
//! [`Check::runs_project_code`]; the agent runs those only when a `Bash` call with the same
//! command would be allowed.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use kor_platform::NoWindow as _;
use kor_tools::ToolOutput;
use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;

use crate::{AgentEvent, Approver, Decision, PermissionReply, PermissionRequest};

/// Failed checks allowed per file in one user turn before the harness stops checking it.
pub const MAX_ROUNDS: u32 = 3;
/// Bytes of checker output given to the model.
const MAX_OUTPUT: usize = 4_000;
const MAX_LINES: usize = 40;

/// `agent.verify` and the per-turn round counters.
#[derive(Debug, Clone)]
pub struct Verify {
    /// `false` with `agent.verify = "off"`.
    pub enabled: bool,
    /// A `Write` that replaces an existing file longer than this is refused with a hint to
    /// use `Edit` (`agent.write_max_lines`; 0 turns the check off).
    pub write_max_lines: usize,
    pub timeout: Duration,
    /// Failed checks per file in the current turn.
    pub(crate) failures: HashMap<PathBuf, u32>,
}

impl Default for Verify {
    fn default() -> Self {
        Self {
            enabled: true,
            write_max_lines: 400,
            timeout: Duration::from_secs(120),
            failures: HashMap::new(),
        }
    }
}

impl Verify {
    /// From `agent.verify` (`enabled`) and `agent.write_max_lines`.
    pub fn new(enabled: bool, write_max_lines: u32) -> Self {
        Self {
            enabled,
            write_max_lines: usize::try_from(write_max_lines).unwrap_or(usize::MAX),
            ..Self::default()
        }
    }
}

/// One checker invocation covering one or more changed files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(&'static str, &'static str)>,
    /// Build tools that may execute code from the project; gated like `Bash`.
    pub runs_project_code: bool,
    /// Changed files, absolute.
    pub files: Vec<PathBuf>,
    /// Only output lines containing this word count (`error`), when set.
    error_word: Option<&'static str>,
}

impl Check {
    /// The command as one line, for permission rules and messages.
    pub fn command_line(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// What a check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Pass,
    /// Errors located in the changed files.
    Fail(String),
    /// The checker ran but gave no answer (timed out, crashed); the reason is for the model.
    Skipped(String),
    /// The checker is not installed; nothing to report.
    Unavailable,
}

/// Nearest ancestor directory of `file` (up to and including `root`) that contains `marker`.
fn nearest(root: &Path, file: &Path, marker: &str) -> Option<PathBuf> {
    file.ancestors()
        .skip(1)
        .take_while(|d| d.starts_with(root))
        .find(|d| d.join(marker).is_file())
        .map(Path::to_path_buf)
}

fn rel(base: &Path, file: &Path) -> String {
    file.strip_prefix(base)
        .unwrap_or(file)
        .to_string_lossy()
        .replace('\\', "/")
}

fn s(p: &Path) -> String {
    kor_tools::display_path(p)
}

/// Test files that by naming convention cover `file` (`a.test.ts`, `a_test.go`, `test_a.py`,
/// `tests/a.rs` in the nearest crate…). Only files that exist; the file itself is never listed.
/// shortcut: name conventions only, no import graph; use kor-graph edges if this misses too much.
pub fn related_tests(root: &Path, file: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(stem), Some(ext)) = (
        file.parent(),
        file.file_stem().and_then(|s| s.to_str()),
        file.extension().and_then(|e| e.to_str()),
    ) else {
        return Vec::new();
    };
    let mut cands = vec![
        dir.join(format!("{stem}.test.{ext}")),
        dir.join(format!("{stem}.spec.{ext}")),
        dir.join(format!("{stem}_test.{ext}")),
        dir.join(format!("test_{stem}.{ext}")),
        dir.join("__tests__").join(format!("{stem}.test.{ext}")),
        dir.join("tests").join(format!("test_{stem}.{ext}")),
    ];
    if ext == "rs"
        && let Some(krate) = nearest(root, file, "Cargo.toml")
    {
        cands.push(krate.join("tests").join(format!("{stem}.rs")));
    }
    let mut out: Vec<PathBuf> = cands
        .into_iter()
        .filter(|p| p != file && p.is_file())
        .collect();
    out.dedup();
    out
}

/// Checkers for the changed `files` (absolute, already confined to the project). Files
/// outside `root`, deleted files and files without a known checker are skipped. Files that
/// share a command are checked once.
pub fn plan(root: &Path, files: &[PathBuf]) -> Vec<Check> {
    let mut checks: Vec<Check> = Vec::new();
    for f in files {
        if !f.starts_with(root) || !f.is_file() {
            continue;
        }
        let Some(c) = plan_one(root, f) else { continue };
        if let Some(same) = checks
            .iter_mut()
            .find(|x| x.program == c.program && x.args == c.args && x.cwd == c.cwd)
        {
            same.files.push(f.clone());
        } else {
            checks.push(c);
        }
    }
    checks
}

fn plan_one(root: &Path, file: &Path) -> Option<Check> {
    let ext = file.extension()?.to_str()?.to_ascii_lowercase();
    let check = |program: &str, args: Vec<String>, cwd: PathBuf| Check {
        program: program.to_owned(),
        args,
        cwd,
        env: Vec::new(),
        runs_project_code: true,
        files: vec![file.to_path_buf()],
        error_word: None,
    };
    match ext.as_str() {
        "rs" => {
            let dir = nearest(root, file, "Cargo.toml")?;
            let mut c = check(
                "cargo",
                vec![
                    "check".into(),
                    "--offline".into(),
                    "--quiet".into(),
                    "--message-format=short".into(),
                    "--manifest-path".into(),
                    s(&dir.join("Cargo.toml")),
                ],
                dir,
            );
            c.error_word = Some("error");
            Some(c)
        }
        "ts" | "tsx" | "mts" | "cts" => {
            let dir = nearest(root, file, "tsconfig.json")?;
            let tsc = dir
                .ancestors()
                .take_while(|d| d.starts_with(root))
                .map(|d| d.join("node_modules/typescript/bin/tsc"))
                .find(|p| p.is_file())?;
            let mut c = check(
                "node",
                vec![
                    s(&tsc),
                    "--noEmit".into(),
                    "--pretty".into(),
                    "false".into(),
                    "-p".into(),
                    s(&dir),
                ],
                dir,
            );
            c.error_word = Some("error");
            Some(c)
        }
        "go" => {
            let dir = nearest(root, file, "go.mod")?;
            let pkg = file.parent().map(|p| rel(&dir, p)).unwrap_or_default();
            let target = if pkg.is_empty() {
                ".".to_owned()
            } else {
                format!("./{pkg}")
            };
            let mut c = check("go", vec!["vet".into(), target], dir);
            // Never download a toolchain or modules for a check.
            c.env = vec![("GOTOOLCHAIN", "local"), ("GOPROXY", "off")];
            Some(c)
        }
        // Single-file syntax checks; neither runs the file. `-I` keeps Python from importing
        // modules out of the project directory.
        "py" => Some(Check {
            runs_project_code: false,
            ..check(
                "python",
                vec!["-I".into(), "-m".into(), "py_compile".into(), s(file)],
                root.to_path_buf(),
            )
        }),
        "js" | "mjs" | "cjs" => Some(Check {
            runs_project_code: false,
            ..check("node", vec!["--check".into(), s(file)], root.to_path_buf())
        }),
        _ => None,
    }
}

/// Run `check` and keep only the problems that point at the changed files.
///
/// shortcut: errors located in other files (a caller broken by a signature change) are not
/// reported, so pre-existing errors elsewhere never block the model; upgrade with a
/// before/after diff of the checker output.
///
/// With `confine`, a checker that runs project code goes through the session's sandbox, like the
/// `Bash` call it stands for (ADR 0033); under Docker it is not run (the image has no host
/// toolchain), so the change is reported as not verified instead of passing by accident.
pub async fn run(
    check: &Check,
    timeout: Duration,
    scrub_env: &[String],
    confine: Option<&kor_tools::ToolCtx>,
) -> Outcome {
    let mut cmd = tokio::process::Command::new(&check.program);
    cmd.args(&check.args)
        .current_dir(&check.cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .no_window();
    for k in scrub_env {
        cmd.env_remove(k);
    }
    for (k, v) in &check.env {
        cmd.env(k, v);
    }
    if let Some(ctx) = confine.filter(|c| check.runs_project_code && c.sandbox.is_enforcing()) {
        return run_confined(check, timeout, ctx, &cmd).await;
    }
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Outcome::Unavailable,
        Err(e) => return Outcome::Skipped(format!("`{}` could not start: {e}", check.program)),
    };
    let pid = child.id();
    let out = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Outcome::Skipped(format!("`{}` failed to run: {e}", check.program)),
        Err(_) => {
            if let Some(pid) = pid {
                kor_platform::kill_tree(pid);
            }
            return Outcome::Skipped(format!(
                "`{}` timed out after {}s; the change is not verified",
                check.command_line(),
                timeout.as_secs()
            ));
        }
    };
    if out.status.success() {
        return Outcome::Pass;
    }
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    filter(check, &text)
}

async fn run_confined(
    check: &Check,
    timeout: Duration,
    ctx: &kor_tools::ToolCtx,
    cmd: &tokio::process::Command,
) -> Outcome {
    if ctx.sandbox == kor_sandbox::SandboxKind::Docker {
        return Outcome::Skipped(format!(
            "`{}` was not run: it runs project code and the Docker sandbox image has no host \
             toolchain; the change is not verified",
            check.command_line()
        ));
    }
    let child = match kor_sandbox::spawn(ctx.sandbox, &ctx.sandbox_policy(), cmd) {
        Ok(c) => c,
        Err(e) => {
            return Outcome::Skipped(format!(
                "`{}` could not start in the sandbox: {e}",
                check.program
            ));
        }
    };
    // Dropping the child on timeout kills its whole tree.
    let out = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Outcome::Skipped(format!("`{}` failed to run: {e}", check.program)),
        Err(_) => {
            return Outcome::Skipped(format!(
                "`{}` timed out after {}s; the change is not verified",
                check.command_line(),
                timeout.as_secs()
            ));
        }
    };
    if out.status.success() {
        return Outcome::Pass;
    }
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    filter(check, &text)
}

fn filter(check: &Check, text: &str) -> Outcome {
    let text = text.replace('\\', "/");
    let names: Vec<String> = check.files.iter().map(|f| rel(&check.cwd, f)).collect();
    let mentions = |l: &str| names.iter().any(|n| l.contains(n.as_str()));
    let errors: String = if check.runs_project_code {
        text.lines()
            .filter(|l| mentions(l) && check.error_word.is_none_or(|w| l.contains(w)))
            .take(MAX_LINES)
            .collect::<Vec<_>>()
            .join("\n")
    } else if mentions(&text) {
        // A single-file check: everything it printed is about that file.
        text.lines().take(MAX_LINES).collect::<Vec<_>>().join("\n")
    } else {
        // Not about the file at all (for example a launcher stub asking to install Python).
        String::new()
    };
    if errors.trim().is_empty() {
        return Outcome::Pass;
    }
    Outcome::Fail(cap(errors.trim()))
}

fn cap(s: &str) -> String {
    if s.len() <= MAX_OUTPUT {
        return s.to_owned();
    }
    let end = (0..=MAX_OUTPUT)
        .rev()
        .find(|&i| s.is_char_boundary(i))
        .unwrap_or(0);
    format!("{}\n… [output truncated]", &s[..end])
}

impl crate::Agent {
    /// `Write` over an existing file longer than `write_max_lines`: the hint for the model
    /// (spec 8.1, "never rewrites whole files").
    pub(crate) fn write_guard(&self, tool: &str, input: &Value) -> Option<String> {
        let max = self.verify.write_max_lines;
        if tool != "Write" || max == 0 {
            return None;
        }
        let path = input.get("file_path").and_then(Value::as_str)?;
        let existing = std::fs::read_to_string(self.ctx.resolve(path).ok()?).ok()?;
        let lines = existing.lines().count();
        (lines > max).then(|| {
            format!(
                "not written: `{path}` already has {lines} lines and rewriting a whole file over {max} lines \
                 is refused. Change only what needs to change with Edit (exact text replacement), \
                 MultiEdit or apply_patch."
            )
        })
    }

    /// Check the files a successful change touched and append the result to `out`.
    #[allow(clippy::too_many_lines)] // pre-parse, then each checker in order
    pub(crate) async fn verify_changes(
        &mut self,
        touched: &[String],
        out: &mut ToolOutput,
        events: &UnboundedSender<AgentEvent>,
        approver: &dyn Approver,
    ) {
        if !self.verify.enabled || touched.is_empty() {
            return;
        }
        let mut files: Vec<PathBuf> = touched
            .iter()
            .filter_map(|t| self.ctx.resolve(t).ok())
            .collect();
        // Tree-sitter first: a file that no longer parses is reported at once (line and column),
        // without waiting for the compiler, and counts as a failed round like a compiler error.
        // Reading and parsing run off the async runtime (large files, many of them).
        let probe = files.clone();
        let errors: std::collections::HashMap<PathBuf, (usize, usize)> =
            tokio::task::spawn_blocking(move || {
                probe
                    .into_iter()
                    .filter_map(|f| {
                        let lang = f
                            .extension()
                            .and_then(|e| e.to_str())
                            .and_then(kor_graph::lang::Lang::from_ext)?;
                        let src = std::fs::read_to_string(&f).ok()?;
                        kor_graph::symbols::first_error(lang, &src).map(|at| (f, at))
                    })
                    .collect()
            })
            .await
            .unwrap_or_default();
        files.retain(|f| {
            let Some(&(line, col)) = errors.get(f) else {
                return true;
            };
            let n = self.verify.failures.entry(f.clone()).or_insert(0);
            *n += 1;
            let _ = write!(
                out.content,
                "\n\n[verify] syntax error in {} at line {line}, column {col} (tree-sitter, round {n}/{MAX_ROUNDS}). \
                 The change is saved; fix it before continuing.",
                rel(&self.ctx.root, f)
            );
            false
        });
        for check in plan(&self.ctx.root, &files) {
            let cmd = check.command_line();
            let shown: Vec<String> = check.files.iter().map(|f| rel(&self.ctx.root, f)).collect();
            let shown = shown.join(", ");
            let spent = check
                .files
                .iter()
                .all(|f| self.verify.failures.get(f).copied().unwrap_or(0) >= MAX_ROUNDS);
            if spent {
                let _ = write!(
                    out.content,
                    "\n\n[verify] {shown} still failed `{}` after {MAX_ROUNDS} rounds in this turn, so it is no longer \
                     checked. Stop changing it and tell the user what is still broken.",
                    check.program
                );
                let _ = events.send(AgentEvent::Notice(format!(
                    "verification stopped for {shown} after {MAX_ROUNDS} failed rounds"
                )));
                continue;
            }
            if check.runs_project_code && !self.allow_check(&cmd, approver).await {
                let _ = write!(
                    out.content,
                    "\n\n[verify] not checked: running `{cmd}` needs Bash permission."
                );
                continue;
            }
            match run(
                &check,
                self.verify.timeout,
                &self.ctx.scrub_env,
                Some(&self.ctx),
            )
            .await
            {
                Outcome::Pass => {
                    let _ = write!(
                        out.content,
                        "\n\n[verify] `{}`: no errors in {shown}.",
                        check.program
                    );
                    let tests: Vec<String> = check
                        .files
                        .iter()
                        .flat_map(|f| related_tests(&self.ctx.root, f))
                        .map(|t| rel(&self.ctx.root, &t))
                        .collect();
                    if !tests.is_empty() {
                        let _ = write!(
                            out.content,
                            " Related tests: {}. Run them before you finish.",
                            tests.join(", ")
                        );
                    }
                }
                Outcome::Fail(errors) => {
                    let mut round = 0;
                    for f in &check.files {
                        let n = self.verify.failures.entry(f.clone()).or_insert(0);
                        *n += 1;
                        round = round.max(*n);
                    }
                    let _ = write!(
                        out.content,
                        "\n\n[verify] `{}` found errors in {shown} (round {round}/{MAX_ROUNDS}). \
                         The change is saved; fix these before continuing:\n{errors}",
                        check.program
                    );
                    if round >= MAX_ROUNDS {
                        out.content.push_str(
                            "\nThis was the last automatic check of this file in this turn.",
                        );
                    }
                }
                Outcome::Skipped(reason) => {
                    let _ = write!(out.content, "\n\n[verify] {reason}");
                }
                Outcome::Unavailable => {}
            }
        }
    }

    /// A checker that can run project code is allowed exactly when the same `Bash` command
    /// would be: rules first, then the user (an `Ask` goes to the approver).
    async fn allow_check(&mut self, cmd: &str, approver: &dyn Approver) -> bool {
        let Some(bash) = self.tools.iter().find(|t| t.name() == "Bash").cloned() else {
            return false;
        };
        let input = serde_json::json!({ "command": cmd });
        match self.permissions.decide(&*bash, &input) {
            Decision::Allow => true,
            Decision::Deny => false,
            Decision::Ask => {
                let req = PermissionRequest {
                    id: "verify".into(),
                    tool: "Bash".into(),
                    summary: format!("verify the change: {cmd}"),
                    ..Default::default()
                };
                match approver.approve(&req).await {
                    PermissionReply::Allow => true,
                    PermissionReply::AllowAlways => {
                        self.permissions.allow_exact(&*bash, &input);
                        true
                    }
                    PermissionReply::Deny => false,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_related_tests_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        for p in [
            "web/cart.ts",
            "web/cart.test.ts",
            "py/tax.py",
            "py/tests/test_tax.py",
            "k/Cargo.toml",
            "k/src/fee.rs",
            "k/tests/fee.rs",
        ] {
            std::fs::create_dir_all(root.join(p).parent().unwrap()).unwrap();
            std::fs::write(root.join(p), "").unwrap();
        }
        let names = |f: &str| -> Vec<String> {
            related_tests(&root, &root.join(f))
                .iter()
                .map(|p| rel(&root, p))
                .collect()
        };
        assert_eq!(names("web/cart.ts"), ["web/cart.test.ts"]);
        assert_eq!(names("py/tax.py"), ["py/tests/test_tax.py"]);
        assert_eq!(names("k/src/fee.rs"), ["k/tests/fee.rs"]);
        assert!(names("web/cart.test.ts").is_empty());
    }

    #[test]
    #[allow(clippy::many_single_char_names)]
    fn picks_checkers_from_project_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let w = |p: &str, t: &str| {
            let f = root.join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(&f, t).unwrap();
            f
        };
        w("crates/a/Cargo.toml", "[package]");
        let a = w("crates/a/src/lib.rs", "");
        let b = w("crates/a/src/b.rs", "");
        let py = w("tool.py", "");
        let js = w("web/x.js", "");
        let txt = w("notes.txt", "");
        let lone = w("loose/x.rs", "");
        let checks = plan(
            &root,
            &[a.clone(), b, py, js, txt, lone, root.join("gone.py")],
        );
        assert_eq!(checks.len(), 3, "{checks:#?}");
        assert_eq!(checks[0].program, "cargo");
        assert_eq!(checks[0].files.len(), 2, "one cargo check per package");
        assert!(checks[0].runs_project_code);
        assert_eq!(checks[0].cwd, root.join("crates/a"));
        assert!(checks[0].args.contains(&"--offline".to_owned()));
        assert_eq!(checks[1].args[..3], ["-I", "-m", "py_compile"]);
        assert!(!checks[1].runs_project_code && !checks[2].runs_project_code);

        // tsc only from the project's own node_modules.
        w("app/tsconfig.json", "{}");
        let ts = w("app/src/m.ts", "");
        assert!(plan(&root, std::slice::from_ref(&ts)).is_empty());
        w("node_modules/typescript/bin/tsc", "");
        let c = &plan(&root, &[ts])[0];
        assert_eq!(c.program, "node");
        assert!(c.args.contains(&"--noEmit".to_owned()) && c.runs_project_code);

        w("go.mod", "module x");
        let g = w("pkg/v/a.go", "");
        let c = &plan(&root, &[g])[0];
        assert_eq!(c.args, ["vet", "./pkg/v"]);
        assert!(c.env.contains(&("GOPROXY", "off")));

        // Nothing outside the root.
        let other = tempfile::tempdir().unwrap();
        std::fs::write(other.path().join("x.py"), "").unwrap();
        assert!(plan(&root, &[other.path().canonicalize().unwrap().join("x.py")]).is_empty());
    }

    #[test]
    fn keeps_only_errors_in_the_changed_files() {
        let check = Check {
            program: "cargo".into(),
            args: Vec::new(),
            cwd: PathBuf::from("/p"),
            env: Vec::new(),
            runs_project_code: true,
            files: vec![PathBuf::from("/p/src/lib.rs")],
            error_word: Some("error"),
        };
        let out = "src\\lib.rs:2:5: error[E0308]: mismatched types\n\
                   src/lib.rs:9:1: warning: unused\n\
                   src/other.rs:1:1: error: old problem\n\
                   error: could not compile `p`";
        assert_eq!(
            filter(&check, out),
            Outcome::Fail("src/lib.rs:2:5: error[E0308]: mismatched types".into())
        );
        assert_eq!(filter(&check, "src/other.rs:1:1: error: x"), Outcome::Pass);

        let single = Check {
            runs_project_code: false,
            error_word: None,
            files: vec![PathBuf::from("/p/a.py")],
            ..check
        };
        assert!(
            matches!(filter(&single, "  File \"a.py\", line 1\nSyntaxError: x"), Outcome::Fail(t) if t.contains("SyntaxError"))
        );
        assert_eq!(filter(&single, "Python was not found"), Outcome::Pass);
        assert!(cap(&"é".repeat(5_000)).ends_with("[output truncated]"));
    }
}
