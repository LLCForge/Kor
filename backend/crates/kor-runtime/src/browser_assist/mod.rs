//! Side-panel assistants of the Kor browser extension (spec §27.11–§27.13, ADR 0058). The
//! extension reads the page; everything here runs in the core, so the CLI, desktop and side panel
//! share it (spec §2 rule 9: the extension is a thin client).
//!
//! - [`docs`]: "use this page in my project" — the page explained with the project's own stack,
//!   versions and code; version mismatches; a `Doc` node in the graph; a local page cache.
//! - [`forms`]: form filling from local, keychain-encrypted profiles; never password, card, CVV or
//!   one-time-code fields; never submits.
//! - [`page_code`]: a picked page component → code in the project's stack and tokens, with a
//!   visual similarity score and per-file review before writes.

pub mod docs;
pub mod forms;
pub mod page_code;

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;

use crate::analysis::deps;

/// What the project is built with, from its manifests (`kor audit --licenses` inventory).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Stack {
    /// `rust`, `typescript`, `javascript`, `python`, `go`, `php`, ….
    pub languages: Vec<String>,
    /// UI framework: `react`, `vue`, `svelte`, `solid`, `angular`, `leptos`, or empty.
    pub ui: String,
    /// Other notable frameworks (`next`, `tauri`, `django`, `laravel`, …).
    pub frameworks: Vec<String>,
    /// Direct dependencies: name → version (empty when unpinned).
    pub deps: BTreeMap<String, String>,
}

const UI: &[(&str, &str)] = &[
    ("react", "react"),
    ("vue", "vue"),
    ("svelte", "svelte"),
    ("solid-js", "solid"),
    ("@angular/core", "angular"),
    ("leptos", "leptos"),
    ("preact", "react"),
];

const FRAMEWORKS: &[&str] = &[
    "next",
    "nuxt",
    "@sveltejs/kit",
    "astro",
    "remix",
    "@remix-run/react",
    "vite",
    "tailwindcss",
    "@tauri-apps/api",
    "tauri",
    "axum",
    "actix-web",
    "django",
    "flask",
    "fastapi",
    "laravel/framework",
    "symfony/framework-bundle",
    "express",
    "@nestjs/core",
    "@mui/material",
    "@chakra-ui/react",
    "antd",
    "@radix-ui/react-slot",
    "storybook",
    "@storybook/react",
    "vitest",
    "jest",
    "@playwright/test",
];

impl Stack {
    /// Read the project's manifests (no network).
    pub fn detect(root: &Path) -> Self {
        let inv = deps::scan(root);
        let mut s = Self::default();
        for p in inv.direct() {
            s.deps.insert(p.name.clone(), p.version.clone());
            let lang = match p.ecosystem {
                deps::Ecosystem::Cargo => "rust",
                deps::Ecosystem::Npm => {
                    if root.join("tsconfig.json").is_file() {
                        "typescript"
                    } else {
                        "javascript"
                    }
                }
                deps::Ecosystem::PyPI => "python",
                deps::Ecosystem::Go => "go",
                deps::Ecosystem::Packagist => "php",
                deps::Ecosystem::NuGet => "csharp",
                deps::Ecosystem::RubyGems => "ruby",
                deps::Ecosystem::Maven => "java",
            };
            if !s.languages.iter().any(|l| l == lang) {
                s.languages.push(lang.to_owned());
            }
        }
        // Without a lockfile the inventory is empty: the root manifests' declared versions.
        if let Ok(text) = std::fs::read_to_string(root.join("package.json"))
            && let Ok(v) = serde_json::from_str::<serde_json::Value>(&text)
        {
            for section in ["dependencies", "devDependencies"] {
                for (k, ver) in v[section].as_object().into_iter().flatten() {
                    s.deps
                        .entry(k.clone())
                        .or_insert_with(|| ver.as_str().unwrap_or("").to_owned());
                }
            }
            let lang = if root.join("tsconfig.json").is_file() {
                "typescript"
            } else {
                "javascript"
            };
            if !s.languages.iter().any(|l| l == lang) {
                s.languages.push(lang.to_owned());
            }
        }
        if let Ok(text) = std::fs::read_to_string(root.join("Cargo.toml"))
            && let Ok(v) = text.parse::<toml::Table>()
        {
            for section in ["dependencies", "dev-dependencies"] {
                for (k, ver) in v
                    .get(section)
                    .and_then(toml::Value::as_table)
                    .into_iter()
                    .flatten()
                {
                    let ver = ver
                        .as_str()
                        .or_else(|| ver.get("version").and_then(toml::Value::as_str))
                        .unwrap_or("");
                    s.deps.entry(k.clone()).or_insert_with(|| ver.to_owned());
                }
            }
            if !s.languages.iter().any(|l| l == "rust") {
                s.languages.push("rust".to_owned());
            }
        }
        s.ui = UI
            .iter()
            .find(|(dep, _)| s.deps.contains_key(*dep))
            .map(|(_, ui)| (*ui).to_owned())
            .unwrap_or_default();
        s.frameworks = FRAMEWORKS
            .iter()
            .filter(|f| s.deps.contains_key(**f))
            .map(|f| (*f).to_owned())
            .collect();
        s
    }

    /// One line for prompts: `typescript; UI react 19.1.0; next 15.0.0, tailwindcss 4.0.0`.
    pub fn summary(&self) -> String {
        let v = |n: &str| {
            self.deps
                .get(n)
                .filter(|v| !v.is_empty())
                .map(|v| format!(" {v}"))
                .unwrap_or_default()
        };
        let ui = UI.iter().find(|(_, u)| *u == self.ui).map(|(d, _)| *d);
        let mut out = if self.languages.is_empty() {
            "unknown language".to_owned()
        } else {
            self.languages.join(", ")
        };
        if let Some(d) = ui {
            out.push_str(&format!("; UI {}{}", self.ui, v(d)));
        }
        if !self.frameworks.is_empty() {
            let list: Vec<String> = self
                .frameworks
                .iter()
                .map(|f| format!("{f}{}", v(f)))
                .collect();
            out.push_str(&format!("; {}", list.join(", ")));
        }
        out
    }
}

/// The reply language for prompts.
pub(crate) fn language_name(lang: kor_i18n::Lang) -> &'static str {
    match lang {
        kor_i18n::Lang::Tr => "Turkish",
        kor_i18n::Lang::Fr => "French",
        _ => "English",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack_reads_ui_and_frameworks_from_manifests() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name":"app","dependencies":{"react":"19.1.0","next":"15.0.0"},"devDependencies":{"tailwindcss":"4.0.0"}}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("tsconfig.json"), "{}").unwrap();
        let s = Stack::detect(dir.path());
        assert_eq!(s.ui, "react");
        assert_eq!(s.languages, ["typescript"]);
        assert!(
            s.frameworks.contains(&"next".to_owned())
                && s.frameworks.contains(&"tailwindcss".to_owned())
        );
        let line = s.summary();
        assert!(line.starts_with("typescript; UI react"), "{line}");
    }
}
