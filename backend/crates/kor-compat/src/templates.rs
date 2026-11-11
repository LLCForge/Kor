//! Prompt and task templates (spec 15.11): Markdown with front matter, loaded by the same
//! reader as slash commands (`adapters::claude::extras`) and invoked as `/name` or
//! `kor run template:name --input value`.
//!
//! Precedence, first name wins: project `.kor/templates/` > personal `~/.kor/templates/` >
//! enabled plugins' `templates/` > built-ins (shipped with Kor). Team templates synced from
//! Kor Server (Faz 8) will be written to a directory and read with [`load_dir`] between the
//! personal and plugin ones; nothing else changes.
//!
//! ```text
//! ---
//! name: new-endpoint
//! description: Add an HTTP endpoint
//! model: anthropic/claude-sonnet-4-5        # optional
//! inputs:
//!   - name: string # help text               (required)
//!   - method: enum(GET|POST) = GET            (optional, default)
//!   - route: string?                          (optional, empty)
//!   - auth: bool = true
//!   - file: path?                             (relative, inside the project)
//!   - limit: number = 10
//! steps:
//!   - ordered sub-goal, may use {{name}}
//! acceptance:
//!   - criterion; commands in `backticks` are run by the agent with Bash
//! ---
//! Body with {{name}} placeholders.
//! ```
//!
//! Rendering is plain `{{input}}` substitution in one pass: no shell, no file reads, no
//! expressions, and substituted values are never expanded again.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Component, Path};

use serde::Serialize;

use crate::frontmatter::split;
use crate::{Compat, Scope, Source, files, read};

/// Longest accepted input value, in characters.
const MAX_VALUE_CHARS: usize = 8_192;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", content = "values", rename_all = "lowercase")]
pub enum Kind {
    String,
    Number,
    Bool,
    /// A project-relative path (no absolute paths, no `..`).
    Path,
    Enum(Vec<String>),
}

impl std::fmt::Display for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::String => f.write_str("string"),
            Self::Number => f.write_str("number"),
            Self::Bool => f.write_str("bool"),
            Self::Path => f.write_str("path"),
            Self::Enum(v) => write!(f, "{}", v.join("|")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Input {
    pub name: String,
    pub kind: Kind,
    pub required: bool,
    pub default: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Template {
    pub name: String,
    pub description: String,
    pub inputs: Vec<Input>,
    pub model: Option<String>,
    pub steps: Vec<String>,
    pub acceptance: Vec<String>,
    pub body: String,
    /// `None` for a built-in template.
    pub source: Option<Source>,
}

/// Why the given inputs do not fit a template. The CLI and desktop localize these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputError {
    Unknown(String),
    Missing(String),
    Duplicate(String),
    /// The value does not fit the input's type; `expected` is the type as written.
    Invalid {
        name: String,
        expected: String,
    },
    /// More bare values than inputs.
    ExtraValue(String),
    /// An unclosed quote.
    Syntax,
}

/// Input name → validated value (defaults filled in, booleans as `true`/`false`).
pub type Values = BTreeMap<String, String>;

const BUILTIN: [&str; 3] = [
    include_str!("../templates/new-endpoint.md"),
    include_str!("../templates/write-migration.md"),
    include_str!("../templates/test-from-bug.md"),
];

/// Templates shipped with Kor.
pub fn builtins() -> Vec<Template> {
    BUILTIN
        .iter()
        .filter_map(|t| parse("", t, None).ok())
        .collect()
}

/// Every template of this project: loaded ones first, then built-ins nobody overrode.
pub fn all(c: &Compat) -> Vec<Template> {
    let mut out = c.templates.clone();
    for b in builtins() {
        if !out.iter().any(|t| t.name == b.name) {
            out.push(b);
        }
    }
    out
}

pub fn find(c: &Compat, name: &str) -> Option<Template> {
    let name = name.strip_prefix("template:").unwrap_or(name);
    c.templates
        .iter()
        .find(|t| t.name == name)
        .cloned()
        .or_else(|| builtins().into_iter().find(|t| t.name == name))
}

/// Read `dir/*.md` as templates; invalid files become warnings. Earlier definitions win.
pub fn load_dir(c: &mut Compat, dir: &Path, eco: crate::Eco, scope: Scope) {
    for p in files(dir, "md") {
        let Some(text) = read(&p, &mut c.warnings) else {
            continue;
        };
        let stem = p
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let source = Source {
            eco,
            scope,
            path: p.clone(),
        };
        match parse(&stem, &text, Some(source)) {
            Ok(t) if !c.templates.iter().any(|x| x.name == t.name) => c.templates.push(t),
            Ok(_) => {}
            Err(e) => c
                .warnings
                .push(format!("{}: invalid template: {e}", p.display())),
        }
    }
}

/// A name usable as a file name and after `/`: 1-64 ASCII letters, digits, `-` or `_`.
pub fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.starts_with(|c: char| c.is_ascii_alphanumeric())
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Parse one template file. `stem` names it when the front matter has no `name`.
pub fn parse(stem: &str, text: &str, source: Option<Source>) -> Result<Template, String> {
    let (front, body) = split(text);
    let name = front.get("name").unwrap_or(stem).to_owned();
    if !valid_name(&name) {
        return Err(format!(
            "name `{name}` must be 1-64 letters, digits, `-` or `_`"
        ));
    }
    let description = front
        .get("description")
        .ok_or("`description` is required")?
        .to_owned();
    let mut inputs: Vec<Input> = Vec::new();
    for item in front.items("inputs") {
        let input = parse_input(item)?;
        if inputs.iter().any(|i| i.name == input.name) {
            return Err(format!("input `{}` is declared twice", input.name));
        }
        inputs.push(input);
    }
    let t = Template {
        name,
        description,
        inputs,
        model: front.get("model").map(str::to_owned),
        steps: front.items("steps").to_vec(),
        acceptance: front.items("acceptance").to_vec(),
        body: body.trim().to_owned(),
        source,
    };
    if t.body.is_empty() && t.steps.is_empty() {
        return Err("a template needs a body or `steps`".into());
    }
    for text in std::iter::once(&t.body)
        .chain(&t.steps)
        .chain(&t.acceptance)
    {
        for key in placeholders(text) {
            if !t.inputs.iter().any(|i| i.name == key) {
                return Err(format!("`{{{{{key}}}}}` is not a declared input"));
            }
        }
    }
    Ok(t)
}

/// `name: type[?] [= default] [# description]`.
fn parse_input(item: &str) -> Result<Input, String> {
    let (spec, description) = match item.split_once(" #") {
        Some((s, d)) => (s, Some(d.trim().to_owned()).filter(|d| !d.is_empty())),
        None => (item, None),
    };
    let (name, rest) = spec
        .split_once(':')
        .ok_or_else(|| format!("input `{item}`: expected `name: type`"))?;
    let name = name.trim().to_owned();
    if !valid_name(&name) {
        return Err(format!(
            "input name `{name}` must be letters, digits, `-` or `_`"
        ));
    }
    let (ty, default) = match rest.split_once('=') {
        Some((t, d)) => (t.trim(), Some(unquote(d.trim()))),
        None => (rest.trim(), None),
    };
    let (ty, optional) = match ty.strip_suffix('?') {
        Some(t) => (t.trim(), true),
        None => (ty, false),
    };
    let kind = match ty {
        "string" => Kind::String,
        "number" => Kind::Number,
        "bool" => Kind::Bool,
        "path" => Kind::Path,
        _ => {
            let values: Vec<String> = ty
                .strip_prefix("enum(")
                .and_then(|r| r.strip_suffix(')'))
                .ok_or_else(|| {
                    format!(
                        "input `{name}`: unknown type `{ty}` (string, number, bool, path, enum(a|b))"
                    )
                })?
                .split(['|', ','])
                .map(|v| unquote(v.trim()))
                .filter(|v| !v.is_empty())
                .collect();
            if values.is_empty() {
                return Err(format!("input `{name}`: enum needs at least one value"));
            }
            Kind::Enum(values)
        }
    };
    let default = match default {
        Some(d) => Some(
            check(&kind, &d)
                .map_err(|()| format!("input `{name}`: default `{d}` is not a {kind}"))?,
        ),
        None => None,
    };
    Ok(Input {
        required: !optional && default.is_none(),
        name,
        kind,
        default,
        description,
    })
}

fn unquote(s: &str) -> String {
    for q in ['"', '\''] {
        if let Some(inner) = s.strip_prefix(q).and_then(|r| r.strip_suffix(q)) {
            return inner.to_owned();
        }
    }
    s.to_owned()
}

/// `{{ key }}` names in `text`.
fn placeholders(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { break };
        out.push(after[..end].trim().to_owned());
        rest = &after[end + 2..];
    }
    out
}

/// Validate and normalize one value.
fn check(kind: &Kind, v: &str) -> Result<String, ()> {
    if v.chars().count() > MAX_VALUE_CHARS
        || v.chars().any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(());
    }
    match kind {
        Kind::String => Ok(v.to_owned()),
        Kind::Number => v
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .map(|_| v.trim().to_owned())
            .ok_or(()),
        Kind::Bool => match v.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "1" => Ok("true".into()),
            "false" | "no" | "off" | "0" => Ok("false".into()),
            _ => Err(()),
        },
        Kind::Enum(values) => values.iter().find(|x| *x == v).cloned().ok_or(()),
        Kind::Path => {
            let p = Path::new(v);
            let inside = !v.trim().is_empty()
                && p.components()
                    .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
            if inside { Ok(v.to_owned()) } else { Err(()) }
        }
    }
}

/// Shell-like words: whitespace separated, `"…"` and `'…'` keep spaces.
pub fn split_words(s: &str) -> Result<Vec<String>, InputError> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut any = false;
    for c in s.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (None, '"' | '\'') => {
                quote = Some(c);
                any = true;
            }
            (None, c) if c.is_whitespace() => {
                if any {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            (_, c) => {
                cur.push(c);
                any = true;
            }
        }
    }
    if quote.is_some() {
        return Err(InputError::Syntax);
    }
    if any {
        out.push(cur);
    }
    Ok(out)
}

/// Inputs from `--name value`, `--name=value`, a bare `--flag` for a bool, or bare values that
/// fill the remaining inputs in declaration order. Defaults fill the rest.
pub fn values(t: &Template, words: &[String]) -> Result<Values, InputError> {
    let mut given: Values = BTreeMap::new();
    let mut bare: Vec<&str> = Vec::new();
    let mut it = words.iter().peekable();
    while let Some(w) = it.next() {
        let Some(flag) = w.strip_prefix("--") else {
            bare.push(w);
            continue;
        };
        let (key, inline) = match flag.split_once('=') {
            Some((k, v)) => (k, Some(v.to_owned())),
            None => (flag, None),
        };
        let input = t
            .inputs
            .iter()
            .find(|i| i.name == key)
            .ok_or_else(|| InputError::Unknown(key.to_owned()))?;
        let value = match inline {
            Some(v) => v,
            None if input.kind == Kind::Bool => match it.peek() {
                Some(next) if check(&Kind::Bool, next).is_ok() => {
                    it.next().cloned().unwrap_or_default()
                }
                _ => "true".into(),
            },
            None => it.next().cloned().ok_or_else(|| InputError::Invalid {
                name: key.to_owned(),
                expected: input.kind.to_string(),
            })?,
        };
        if given.insert(key.to_owned(), value).is_some() {
            return Err(InputError::Duplicate(key.to_owned()));
        }
    }
    let free: Vec<&Input> = t
        .inputs
        .iter()
        .filter(|i| !given.contains_key(&i.name))
        .collect();
    for (n, v) in bare.into_iter().enumerate() {
        let input = free
            .get(n)
            .ok_or_else(|| InputError::ExtraValue(v.to_owned()))?;
        given.insert(input.name.clone(), v.to_owned());
    }

    let mut out = Values::new();
    for i in &t.inputs {
        let v = match (given.remove(&i.name), &i.default) {
            (Some(v), _) => check(&i.kind, &v).map_err(|()| InputError::Invalid {
                name: i.name.clone(),
                expected: i.kind.to_string(),
            })?,
            (None, Some(d)) => d.clone(),
            (None, None) if i.required => return Err(InputError::Missing(i.name.clone())),
            (None, None) => String::new(),
        };
        out.insert(i.name.clone(), v);
    }
    Ok(out)
}

/// Defaults, with `<name>` markers for required inputs: a draft the user edits before sending.
pub fn draft_values(t: &Template) -> Values {
    t.inputs
        .iter()
        .map(|i| {
            let v = i.default.clone().unwrap_or_else(|| {
                if i.required {
                    format!("<{}>", i.name)
                } else {
                    String::new()
                }
            });
            (i.name.clone(), v)
        })
        .collect()
}

fn substitute(text: &str, values: &Values) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[start..]);
            return out;
        };
        match values.get(after[..end].trim()) {
            Some(v) => out.push_str(v),
            None => out.push_str(&rest[start..start + 2 + end + 2]),
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

/// The prompt the agent receives: the body, then the steps as ordered sub-goals, then the
/// acceptance criteria it must check before it finishes.
pub fn render(t: &Template, values: &Values) -> String {
    let mut out = format!("Task template `{}`: {}\n\n", t.name, t.description);
    out.push_str(&substitute(&t.body, values));
    if !t.steps.is_empty() {
        out.push_str(
            "\n\nSteps (ordered sub-goals; finish each one before starting the next and say which step you are on):\n",
        );
        for (n, s) in t.steps.iter().enumerate() {
            let _ = writeln!(out, "{}. {}", n + 1, substitute(s, values));
        }
    }
    if !t.acceptance.is_empty() {
        out.push_str(
            "\nAcceptance criteria (before you finish, check every one: run the commands in backticks \
             and the project's own build and test commands with Bash, then report each criterion as \
             passed or failed with the evidence):\n",
        );
        for a in &t.acceptance {
            let _ = writeln!(out, "- {}", substitute(a, values));
        }
    }
    out.trim_end().to_owned()
}

/// Where templates live under a project root or the home directory (`<base>/.kor/templates`).
pub fn dir(base: &Path) -> std::path::PathBuf {
    base.join(".kor").join("templates")
}

/// Scaffold `<dir>/<name>.md`. Never overwrites: an existing file is `AlreadyExists`, and an
/// invalid name is `InvalidInput`.
pub fn create(dir: &Path, name: &str) -> std::io::Result<std::path::PathBuf> {
    if !valid_name(name) {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{name}.md"));
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    std::io::Write::write_all(&mut f, scaffold(name).as_bytes())?;
    Ok(path)
}

/// A new template file's text.
pub fn scaffold(name: &str) -> String {
    format!(
        "---\nname: {name}\ndescription: What this template does\ninputs:\n  - target: string # What to work on\n  - careful: bool = true # Ask before risky changes\nsteps:\n  - Understand how the project already handles {{{{target}}}}\n  - Make the change\nacceptance:\n  - The project's tests pass\n---\nDo the task for {{{{target}}}}. Careful mode: {{{{careful}}}}.\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Eco;

    fn w(s: &str) -> Vec<String> {
        split_words(s).unwrap()
    }

    const T: &str = "---\nname: ep\ndescription: Add endpoint\ninputs:\n  - name: string # resource\n  - method: enum(GET|POST) = GET\n  - auth: bool = true\n  - limit: number?\n  - file: path?\nsteps:\n  - Add {{method}} for {{name}}\nacceptance:\n  - `cargo test` passes\n---\nAdd {{ name }} ({{auth}}) {{limit}}.\n";

    #[test]
    fn parses_inputs_steps_and_acceptance() {
        let t = parse("x", T, None).unwrap();
        assert_eq!(t.name, "ep");
        assert_eq!(t.inputs.len(), 5);
        assert!(t.inputs[0].required);
        assert_eq!(t.inputs[0].description.as_deref(), Some("resource"));
        assert_eq!(
            t.inputs[1].kind,
            Kind::Enum(vec!["GET".into(), "POST".into()])
        );
        assert_eq!(t.inputs[1].default.as_deref(), Some("GET"));
        assert!(!t.inputs[1].required && !t.inputs[3].required);
        assert_eq!(t.steps, ["Add {{method}} for {{name}}"]);
        assert_eq!(t.acceptance, ["`cargo test` passes"]);
        // No name in front matter: the file stem.
        let t = parse("stem", "---\ndescription: d\n---\nbody\n", None).unwrap();
        assert_eq!(t.name, "stem");
    }

    #[test]
    fn rejects_invalid_templates() {
        let bad = |front: &str, body: &str| {
            parse("x", &format!("---\n{front}\n---\n{body}\n"), None).unwrap_err()
        };
        assert!(bad("name: x", "b").contains("description"));
        assert!(bad("name: a b\ndescription: d", "b").contains("name"));
        assert!(bad("description: d\ninputs:\n  - n: int", "b").contains("unknown type"));
        assert!(bad("description: d\ninputs:\n  - n", "b").contains("name: type"));
        assert!(bad("description: d\ninputs:\n  - n: string\n  - n: bool", "b").contains("twice"));
        assert!(bad("description: d\ninputs:\n  - n: bool = maybe", "b").contains("default"));
        assert!(bad("description: d\ninputs:\n  - n: enum()", "b").contains("enum"));
        assert!(bad("description: d", "{{ghost}}").contains("ghost"));
        assert!(bad("description: d", "").contains("body"));
    }

    #[test]
    fn validates_values_strictly() {
        let t = parse("x", T, None).unwrap();
        let v = values(
            &t,
            &w("--name users --method POST --auth no --limit 5 --file src/a.rs"),
        )
        .unwrap();
        assert_eq!(v["method"], "POST");
        assert_eq!(v["auth"], "false");
        assert_eq!(v["file"], "src/a.rs");
        // Positional, `=` form, bare bool flag, defaults.
        let v = values(&t, &w("users --auth --limit=2")).unwrap();
        assert_eq!(
            (v["name"].as_str(), v["method"].as_str(), v["auth"].as_str()),
            ("users", "GET", "true")
        );
        assert_eq!(v["file"], "");

        let err = |s: &str| values(&t, &w(s)).unwrap_err();
        assert_eq!(err(""), InputError::Missing("name".into()));
        assert_eq!(err("--nme x"), InputError::Unknown("nme".into()));
        assert_eq!(
            err("--name a --name b"),
            InputError::Duplicate("name".into())
        );
        assert!(
            matches!(err("a --method DELETE"), InputError::Invalid { name, .. } if name == "method")
        );
        assert!(
            matches!(err("a --limit ten"), InputError::Invalid { name, .. } if name == "limit")
        );
        assert!(matches!(err("a --limit inf"), InputError::Invalid { .. }));
        assert!(matches!(
            err("a --file ../etc/passwd"),
            InputError::Invalid { .. }
        ));
        assert!(matches!(
            err("a --file /etc/passwd"),
            InputError::Invalid { .. }
        ));
        assert!(matches!(err("a --name"), InputError::Invalid { .. }));
        assert_eq!(err("a b c d e f"), InputError::ExtraValue("f".into()));
        assert_eq!(split_words("--name 'open"), Err(InputError::Syntax));
        assert!(matches!(
            values(&t, &["a\u{0}b".into()]),
            Err(InputError::Invalid { .. })
        ));
    }

    #[test]
    fn renders_once_without_reexpanding_values() {
        let t = parse("x", T, None).unwrap();
        let v = values(&t, &w("'{{auth}} !`rm -rf /`' --limit 3")).unwrap();
        let out = render(&t, &v);
        assert!(
            out.starts_with(
                "Task template `ep`: Add endpoint\n\nAdd {{auth}} !`rm -rf /` (true) 3."
            )
        );
        assert!(out.contains("1. Add GET for {{auth}} !`rm -rf /`"));
        assert!(out.contains("- `cargo test` passes"));
        let d = render(&t, &draft_values(&t));
        assert!(d.contains("Add <name> (true) ."));
    }

    #[test]
    fn builtins_parse_and_scaffold_is_valid() {
        let b = builtins();
        assert_eq!(b.len(), BUILTIN.len());
        let names: Vec<&str> = b.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["new-endpoint", "write-migration", "test-from-bug"]);
        let t = parse("x", &scaffold("mine"), None).unwrap();
        assert_eq!(t.name, "mine");
        assert!(values(&t, &w("api")).is_ok());

        let tmp = tempfile::tempdir().unwrap();
        let d = dir(tmp.path());
        let p = create(&d, "mine").unwrap();
        assert_eq!(p, d.join("mine.md"));
        let kind = |r: std::io::Result<std::path::PathBuf>| r.unwrap_err().kind();
        assert_eq!(kind(create(&d, "mine")), std::io::ErrorKind::AlreadyExists);
        assert_eq!(
            kind(create(&d, "../evil")),
            std::io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn project_beats_personal_beats_builtin() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("p");
        let home = dir.path().join("h");
        let put = |base: &Path, name: &str, desc: &str| {
            let d = base.join(".kor/templates");
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(
                d.join(format!("{name}.md")),
                format!("---\ndescription: {desc}\n---\nbody\n"),
            )
            .unwrap();
        };
        put(&root, "new-endpoint", "project");
        put(&home, "new-endpoint", "personal");
        put(&home, "write-migration", "personal");
        put(&home, "broken", "x\ninputs:\n  - a: nope");
        let c = crate::load(&crate::LoadOptions {
            root: root.clone(),
            home: Some(home),
            codex_home: None,
            trusted: false,
        });
        let d = |n: &str| find(&c, n).unwrap().description;
        assert_eq!(d("new-endpoint"), "project");
        assert_eq!(d("template:write-migration"), "personal");
        assert_eq!(find(&c, "test-from-bug").unwrap().source, None);
        assert!(find(&c, "broken").is_none());
        assert!(c.warnings.iter().any(|w| w.contains("invalid template")));
        let src = find(&c, "new-endpoint").unwrap().source.unwrap();
        assert_eq!((src.eco, src.scope), (Eco::Kor, Scope::Project));
        assert_eq!(all(&c).len(), 3);

        // A plugin's templates come after personal ones.
        let plugin = dir.path().join("plug");
        std::fs::create_dir_all(plugin.join("templates")).unwrap();
        std::fs::write(
            plugin.join("templates/pack.md"),
            "---\ndescription: from plugin\n---\nb\n",
        )
        .unwrap();
        let mut c = c;
        crate::add_plugin(&mut c, &plugin);
        assert_eq!(find(&c, "pack").unwrap().description, "from plugin");
    }
}
