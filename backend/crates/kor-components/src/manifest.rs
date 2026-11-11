//! The pinned component list (`components.toml`, embedded in the binary).

use std::path::{Component as PathPart, Path};

use serde::Deserialize;

/// How a download is unpacked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Archive {
    #[serde(rename = "zip")]
    Zip,
    #[serde(rename = "tar.gz")]
    TarGz,
    #[serde(rename = "tar.xz")]
    TarXz,
    /// The download is the file itself (for example a `.traineddata` model).
    #[serde(rename = "file")]
    File,
}

/// One downloadable build of a component.
#[derive(Debug, Clone, Deserialize)]
pub struct Artifact {
    /// `<os>-<arch>[-musl]` or `any`.
    pub target: String,
    pub url: String,
    /// Lowercase hex SHA-256 of the download.
    pub sha256: String,
    /// Exact size in bytes; downloads larger than this are aborted.
    pub size: u64,
    pub archive: Archive,
    /// Main executable, library or file inside the install directory (`/`-separated).
    pub exe: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Component {
    pub name: String,
    pub version: String,
    pub about: String,
    /// Components installed before this one (for example a model's tokenizer).
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default, rename = "target")]
    pub targets: Vec<Artifact>,
}

impl Component {
    /// The build for `target` (an `any` build fits every target).
    pub fn artifact(&self, target: &str) -> Option<&Artifact> {
        self.targets
            .iter()
            .find(|a| a.target == target)
            .or_else(|| self.targets.iter().find(|a| a.target == "any"))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    #[serde(rename = "component")]
    pub components: Vec<Component>,
}

/// `a/b/c` with only normal parts: no root, drive, `..` or empty path.
pub(crate) fn is_plain_relative(p: &Path) -> bool {
    let mut any = false;
    for part in p.components() {
        match part {
            PathPart::Normal(_) => any = true,
            PathPart::CurDir => {}
            _ => return false,
        }
    }
    any
}

impl Manifest {
    /// Parse and check a manifest: unique safe names, hex hashes, relative `exe` paths.
    pub fn parse(text: &str) -> Result<Self, String> {
        let m: Self = toml::from_str(text).map_err(|e| format!("components manifest: {e}"))?;
        let mut seen = std::collections::HashSet::new();
        for c in &m.components {
            let name_ok = !c.name.is_empty()
                && c.name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
            let version_ok = !c.version.is_empty()
                && c.version
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"+.-_".contains(&b));
            if !name_ok || !version_ok || !seen.insert(c.name.as_str()) {
                return Err(format!("components manifest: bad entry `{}`", c.name));
            }
            for a in &c.targets {
                let hex = a.sha256.len() == 64
                    && a.sha256
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
                if !hex || a.size == 0 || !is_plain_relative(Path::new(&a.exe)) {
                    return Err(format!(
                        "components manifest: bad build `{}` for {}",
                        c.name, a.target
                    ));
                }
            }
        }
        for c in &m.components {
            if c.requires
                .iter()
                .any(|r| r == &c.name || !seen.contains(r.as_str()))
            {
                return Err(format!(
                    "components manifest: bad `requires` of `{}`",
                    c.name
                ));
            }
        }
        Ok(m)
    }

    /// The manifest compiled into Kor.
    pub fn builtin() -> Self {
        // The embedded file is checked by the `builtin_manifest_is_valid` test.
        Self::parse(include_str!("../components.toml")).unwrap_or(Self {
            components: Vec::new(),
        })
    }

    pub fn get(&self, name: &str) -> Option<&Component> {
        self.components.iter().find(|c| c.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_manifest_is_valid() {
        let m = Manifest::parse(include_str!("../components.toml")).unwrap();
        for name in [
            "python",
            "node",
            "pdfium",
            "tesseract",
            "tesseract-lang-tur",
            "tesseract-lang-eng",
            "chromium",
        ] {
            assert!(m.get(name).is_some(), "{name}");
        }
        for name in ["python", "pdfium"] {
            for t in [
                "windows-x86_64",
                "windows-aarch64",
                "linux-x86_64",
                "linux-aarch64",
                "linux-x86_64-musl",
                "linux-aarch64-musl",
                "macos-x86_64",
                "macos-aarch64",
            ] {
                assert!(m.get(name).unwrap().artifact(t).is_some(), "{name} {t}");
            }
        }
        for c in &m.components {
            for a in &c.targets {
                assert!(a.url.starts_with("https://"), "{}", a.url);
            }
        }
        assert!(
            m.get("tesseract-lang-tur")
                .unwrap()
                .artifact("linux-x86_64")
                .is_some()
        );
        let e5 = m.get("embed-e5-small").unwrap();
        assert_eq!(e5.requires, ["embed-e5-small-tokenizer"]);
        assert!(e5.artifact("macos-aarch64").is_some());
        // Offline runtime (ADR 0034): every glibc/Windows/macOS target, models for any target.
        let llama = m.get("llama-server").unwrap();
        for t in [
            "windows-x86_64",
            "windows-aarch64",
            "linux-x86_64",
            "linux-aarch64",
            "macos-x86_64",
            "macos-aarch64",
        ] {
            assert!(llama.artifact(t).is_some(), "llama-server {t}");
        }
        let models: Vec<_> = m
            .components
            .iter()
            .filter(|c| c.name.starts_with("model-"))
            .collect();
        assert!(models.len() >= 3);
        for c in models {
            let a = c.artifact("linux-x86_64").unwrap();
            let gguf = Path::new(&a.exe)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("gguf"));
            assert!(gguf && a.archive == Archive::File, "{}", c.name);
        }
    }

    #[test]
    fn requires_must_name_another_component() {
        let one = |req: &str| {
            format!(
                "[[component]]\nname = \"a\"\nversion = \"1\"\nabout = \"\"\nrequires = [\"{req}\"]\n\
                 [[component]]\nname = \"b\"\nversion = \"1\"\nabout = \"\"\n"
            )
        };
        assert!(Manifest::parse(&one("b")).is_ok());
        assert!(Manifest::parse(&one("a")).is_err());
        assert!(Manifest::parse(&one("zzz")).is_err());
    }

    #[test]
    fn rejects_unsafe_entries() {
        let bad_exe = r#"
            [[component]]
            name = "x"
            version = "1"
            about = ""
            [[component.target]]
            target = "any"
            url = "https://e/x"
            sha256 = "0000000000000000000000000000000000000000000000000000000000000000"
            size = 1
            archive = "file"
            exe = "../x"
        "#;
        assert!(Manifest::parse(bad_exe).is_err());
        assert!(
            Manifest::parse(&bad_exe.replace("../x", "x").replace("\"x\"", "\"../x\"")).is_err()
        );
        assert!(Manifest::parse(&bad_exe.replace("../x", "x")).is_ok());
        assert!(!is_plain_relative(Path::new("/etc/passwd")));
        assert!(!is_plain_relative(Path::new("")));
        assert!(is_plain_relative(Path::new("./a/b")));
    }
}
