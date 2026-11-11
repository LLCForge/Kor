//! Faz 1 acceptance: projects set up with each tool load completely, every rule with the right
//! source and activation. One fixture per tool under `tests/fixtures/tools/<tool>/{project,home}`,
//! one golden listing per tool under `tests/golden/tools/<tool>.txt`.
//! Regenerate with `UPDATE_GOLDEN=1 cargo test -p kor-compat --test tools` and review the diff.

use std::fmt::Write as _;
use std::path::Path;

use kor_compat::{Applies, Compat, LoadOptions, load};

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &dest);
        } else {
            std::fs::copy(e.path(), dest).unwrap();
        }
    }
}

fn listing(c: &Compat, base: &Path) -> String {
    let base = base.display().to_string();
    let rel = |p: &Path| {
        p.display()
            .to_string()
            .replace(&base, "")
            .replace('\\', "/")
            .trim_start_matches('/')
            .to_owned()
    };
    let who = |s: &kor_compat::Source| format!("{:?}/{:?}", s.eco, s.scope).to_lowercase();
    let mut out = String::new();
    for i in &c.instructions {
        let how = match &i.applies {
            Applies::Always => "always".to_owned(),
            Applies::Globs { globs } => format!("globs {}", globs.join(" ")),
            Applies::OnRequest { description } => format!("on-request \"{description}\""),
            Applies::Manual => "manual".to_owned(),
        };
        let first = i.text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        let _ = writeln!(
            out,
            "rule {} {} [{how}] {first}",
            who(&i.source),
            rel(&i.source.path)
        );
    }
    for x in &c.commands {
        let _ = writeln!(
            out,
            "command /{} {} {}",
            x.name,
            who(&x.source),
            rel(&x.source.path)
        );
    }
    for x in &c.agents {
        let _ = writeln!(
            out,
            "agent {} {} {}",
            x.name,
            who(&x.source),
            rel(&x.source.path)
        );
    }
    for x in &c.skills {
        let _ = writeln!(
            out,
            "skill {} {} {}",
            x.name,
            who(&x.source),
            rel(&x.source.path)
        );
    }
    for x in &c.mcp_servers {
        let _ = writeln!(
            out,
            "mcp {} {} {}",
            x.name,
            who(&x.source),
            rel(&x.source.path)
        );
    }
    for s in &c.untrusted_skipped {
        let _ = writeln!(out, "skipped {}", s.replace(&base, "").replace('\\', "/"));
    }
    for w in &c.warnings {
        let _ = writeln!(out, "warning {}", w.replace(&base, "").replace('\\', "/"));
    }
    out
}

fn check(tool: &str) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixture = dir.join("tests/fixtures/tools").join(tool);
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    copy_dir(&fixture.join("project"), &base.join("repo"));
    std::fs::create_dir_all(base.join("home")).unwrap();
    if fixture.join("home").is_dir() {
        copy_dir(&fixture.join("home"), &base.join("home"));
    }
    std::fs::create_dir(base.join("repo/.git")).unwrap();
    std::fs::write(base.join("outside.md"), "OUTSIDE SECRET").unwrap();
    let c = load(&LoadOptions {
        root: base.join("repo"),
        home: Some(base.join("home")),
        codex_home: Some(base.join("home/.codex")),
        trusted: false,
    });
    let actual = listing(&c, &base);
    let golden = dir.join("tests/golden/tools").join(format!("{tool}.txt"));
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(golden.parent().unwrap()).unwrap();
        std::fs::write(&golden, &actual).unwrap();
    }
    let expected = std::fs::read_to_string(&golden)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert_eq!(actual, expected, "golden mismatch for {tool}");
    assert!(!actual.contains("OUTSIDE SECRET") && !actual.contains("secret-token"));
}

macro_rules! tools {
    ($($t:ident),* $(,)?) => { $( #[test] fn $t() { check(stringify!($t)); } )* };
}

tools!(
    cursor,
    claude,
    roo,
    kilo,
    windsurf,
    cline,
    cline_file,
    copilot,
    gemini,
    aider,
    opencode,
    continue_dev,
    kiro,
    devin,
    agents,
    generic,
);
