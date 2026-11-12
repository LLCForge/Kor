//! Continue: `.continue/rules/*.md` (`alwaysApply`, `globs`, `description` front matter),
//! `rules:` in `.continue/config.yaml` (inline strings or `{name, rule, globs, description,
//! alwaysApply}`), `.continue/mcpServers/*.{yaml,yml,json}` (one server per file).
//! Global: `~/.continue/rules/`, `~/.continue/config.yaml`.

use std::path::Path;

use serde_json::Value;

use super::{Adapter, mcp_name, md_rules, src};
use crate::{Applies, Compat, Eco, Instruction, LoadOptions, Scope, read};

pub(crate) struct Continue;

impl Adapter for Continue {
    fn dirs(&self) -> &'static [&'static str] {
        &[".continue"]
    }
    fn user(&self, o: &LoadOptions, c: &mut Compat) {
        if let Some(home) = &o.home {
            load(c, &home.join(".continue"), Scope::User);
        }
    }
    fn project(&self, o: &LoadOptions, c: &mut Compat) {
        load(c, &o.root.join(".continue"), Scope::Project);
    }
}

fn load(c: &mut Compat, dir: &Path, scope: Scope) {
    md_rules(
        c,
        &dir.join("rules"),
        ".md",
        Eco::Continue,
        scope,
        &Applies::Always,
        &[],
    );
    config_rules(c, &dir.join("config.yaml"), scope);
    let mut servers: Vec<_> = ["yaml", "yml", "json"]
        .iter()
        .flat_map(|ext| crate::files(&dir.join("mcpServers"), ext))
        .collect();
    servers.sort();
    for p in servers {
        if let Some(name) = p.file_stem().map(|s| s.to_string_lossy().into_owned()) {
            mcp_name(c, &name, Eco::Continue, scope, &p);
        }
    }
}

/// `rules:` in `config.yaml`. Hub references (`uses:`) are not fetched: Kor reads local files only.
fn config_rules(c: &mut Compat, path: &Path, scope: Scope) {
    let Some(text) = read(path, &mut c.warnings) else {
        return;
    };
    let doc: Value = match serde_yaml::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            c.warnings.push(format!("{}: {e}", path.display()));
            return;
        }
    };
    for r in doc["rules"].as_array().into_iter().flatten() {
        let (text, applies) = match r {
            Value::String(s) => (s.clone(), Applies::Always),
            Value::Object(o) if o.contains_key("uses") => {
                c.warnings.push(format!(
                    "{}: hub rule `{}` is not fetched; copy it into .continue/rules/",
                    path.display(),
                    o["uses"].as_str().unwrap_or("?")
                ));
                continue;
            }
            Value::Object(o) => {
                let globs: Vec<String> = match &o.get("globs") {
                    Some(Value::String(g)) => vec![g.clone()],
                    Some(Value::Array(a)) => a
                        .iter()
                        .filter_map(|g| g.as_str().map(str::to_owned))
                        .collect(),
                    _ => Vec::new(),
                };
                let description = o.get("description").and_then(Value::as_str);
                let applies = if !globs.is_empty() {
                    Applies::Globs { globs }
                } else if let (Some(Value::Bool(false)), Some(d)) =
                    (o.get("alwaysApply"), description)
                {
                    Applies::OnRequest {
                        description: d.to_owned(),
                    }
                } else {
                    Applies::Always
                };
                (
                    o.get("rule")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    applies,
                )
            }
            _ => continue,
        };
        if !text.trim().is_empty() {
            c.instructions.push(Instruction {
                source: src(Eco::Continue, scope, path),
                applies,
                text,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_yaml_rules_are_read_with_their_activation() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("config.yaml");
        std::fs::write(
            &p,
            "name: x
rules:
  - Always answer in Turkish
  - name: ts
    rule: Use strict mode
    globs: \"**/*.ts\"
  - name: db
    rule: Migrations are reversible
    alwaysApply: false
    description: Database work
  - uses: acme/rules
",
        )
        .unwrap();
        let mut c = Compat::default();
        config_rules(&mut c, &p, Scope::Project);
        let got: Vec<_> = c
            .instructions
            .iter()
            .map(|i| (i.text.as_str(), &i.applies))
            .collect();
        assert_eq!(got.len(), 3, "{got:?}");
        assert_eq!(got[0], ("Always answer in Turkish", &Applies::Always));
        assert_eq!(
            got[1].1,
            &Applies::Globs {
                globs: vec!["**/*.ts".into()]
            }
        );
        assert_eq!(
            got[2].1,
            &Applies::OnRequest {
                description: "Database work".into()
            }
        );
        assert!(
            c.warnings.iter().any(|w| w.contains("acme/rules")),
            "{:?}",
            c.warnings
        );
    }
}
