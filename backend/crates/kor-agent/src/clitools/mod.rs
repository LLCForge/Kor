//! Universal tool calling, part two (§26.2, ADR 0056): every command on this computer is a tool.
//!
//! * [`Catalog`]: programs found on `PATH` (cached in `<kor home>/tools/catalog.json`, refreshed
//!   every 24 h and whenever a PATH directory changes — a new install). Well-known CLIs get a
//!   description, usage examples and their version; the rest are listed by name and described on
//!   demand from their own `--help` ([`help`]).
//! * [`defs`]: typed tool definitions — the user's (`<kor home>/tools/*.toml`), the project's
//!   (`.kor/tools/*.toml`, trusted projects only), the team's (`tools/team/`) and the ones the
//!   model generated from `--help` (`tools/generated/`). They reach the model as real tools through
//!   the normal registry: arguments are validated against the schema, the program runs without a
//!   shell inside the session sandbox, and the permission matrix applies to the rendered command.
//! * [`search`]: `tools_search`, so the model can ask "which tools are there?" and read a help
//!   summary before using an unfamiliar command; [`prompt_section`] lists the most relevant tools
//!   compactly within a budget derived from the model's context window.

pub mod defs;
pub mod help;
pub mod search;
pub mod team;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The catalog is rebuilt after this long even when PATH did not change.
pub const MAX_AGE_MS: u64 = 24 * 60 * 60 * 1000;
const VERSION_TIMEOUT: Duration = Duration::from_secs(3);
/// Names kept from PATH (a Windows PATH can hold thousands of programs).
const MAX_ENTRIES: usize = 6000;
const CATALOG_VERSION: u32 = 1;

/// A well-known CLI: description, examples, whether it needs the network, how to ask its version.
struct Known {
    name: &'static str,
    about: &'static str,
    examples: &'static [&'static str],
    network: bool,
    version: Option<&'static [&'static str]>,
    /// Project files that make it relevant (`Cargo.toml` → cargo).
    markers: &'static [&'static str],
}

const V: Option<&[&str]> = Some(&["--version"]);

#[rustfmt::skip]
const KNOWN: &[Known] = &[
    Known { name: "git", about: "distributed version control", examples: &["git status", "git log --oneline -20", "git diff --stat"], network: false, version: V, markers: &[".git"] },
    Known { name: "gh", about: "GitHub CLI: pull requests, issues, releases, Actions", examples: &["gh pr list", "gh pr view 12 --comments", "gh run list"], network: true, version: V, markers: &[".github"] },
    Known { name: "glab", about: "GitLab CLI: merge requests, issues, pipelines", examples: &["glab mr list", "glab ci status"], network: true, version: V, markers: &[".gitlab-ci.yml"] },
    Known { name: "docker", about: "containers and images; `docker compose` for multi-container apps", examples: &["docker ps", "docker compose up -d", "docker logs -f <container>"], network: true, version: V, markers: &["Dockerfile", "docker-compose.yml", "compose.yaml", "compose.yml"] },
    Known { name: "podman", about: "daemonless containers (Docker-compatible)", examples: &["podman ps", "podman build -t app ."], network: true, version: V, markers: &["Containerfile"] },
    Known { name: "kubectl", about: "Kubernetes cluster control", examples: &["kubectl get pods -A", "kubectl logs deploy/api", "kubectl describe pod <name>"], network: true, version: Some(&["version", "--client"]), markers: &["k8s", "kustomization.yaml"] },
    Known { name: "helm", about: "Kubernetes package manager", examples: &["helm list -A", "helm template ./chart"], network: true, version: Some(&["version", "--short"]), markers: &["Chart.yaml"] },
    Known { name: "terraform", about: "infrastructure as code", examples: &["terraform plan", "terraform validate"], network: true, version: Some(&["version"]), markers: &["main.tf"] },
    Known { name: "aws", about: "Amazon Web Services CLI", examples: &["aws s3 ls", "aws sts get-caller-identity"], network: true, version: V, markers: &[] },
    Known { name: "az", about: "Microsoft Azure CLI", examples: &["az account show", "az group list"], network: true, version: V, markers: &[] },
    Known { name: "gcloud", about: "Google Cloud CLI", examples: &["gcloud config list", "gcloud run services list"], network: true, version: V, markers: &[] },
    Known { name: "ffmpeg", about: "convert, cut and encode audio/video", examples: &["ffmpeg -i in.mov -c:v libx264 out.mp4", "ffmpeg -i in.mp4 -ss 5 -frames:v 1 frame.png"], network: false, version: Some(&["-version"]), markers: &[] },
    Known { name: "ffprobe", about: "inspect media files (codecs, duration, streams)", examples: &["ffprobe -hide_banner in.mp4"], network: false, version: Some(&["-version"]), markers: &[] },
    Known { name: "magick", about: "ImageMagick: convert, resize and compose images", examples: &["magick in.png -resize 50% out.png"], network: false, version: V, markers: &[] },
    Known { name: "psql", about: "PostgreSQL client", examples: &["psql \"$DATABASE_URL\" -c '\\dt'"], network: true, version: V, markers: &[] },
    Known { name: "mysql", about: "MySQL/MariaDB client", examples: &["mysql -e 'SHOW TABLES' db"], network: true, version: V, markers: &[] },
    Known { name: "sqlite3", about: "SQLite database shell", examples: &["sqlite3 app.db '.tables'"], network: false, version: V, markers: &[] },
    Known { name: "redis-cli", about: "Redis client", examples: &["redis-cli ping"], network: true, version: V, markers: &[] },
    Known { name: "mongosh", about: "MongoDB shell", examples: &["mongosh --eval 'db.stats()'"], network: true, version: V, markers: &[] },
    Known { name: "node", about: "JavaScript runtime", examples: &["node script.js", "node --test"], network: false, version: V, markers: &["package.json"] },
    Known { name: "npm", about: "Node package manager and script runner", examples: &["npm run test", "npm ls --depth=0", "npm install"], network: true, version: V, markers: &["package-lock.json", "package.json"] },
    Known { name: "npx", about: "run a package's binary", examples: &["npx tsc --noEmit"], network: true, version: V, markers: &["package.json"] },
    Known { name: "pnpm", about: "fast Node package manager", examples: &["pnpm install", "pnpm -r test"], network: true, version: V, markers: &["pnpm-lock.yaml"] },
    Known { name: "yarn", about: "Node package manager", examples: &["yarn install", "yarn test"], network: true, version: V, markers: &["yarn.lock"] },
    Known { name: "bun", about: "JavaScript runtime, bundler and package manager", examples: &["bun install", "bun test"], network: true, version: V, markers: &["bun.lockb", "bun.lock"] },
    Known { name: "deno", about: "secure JavaScript/TypeScript runtime", examples: &["deno test", "deno task dev"], network: true, version: V, markers: &["deno.json", "deno.jsonc"] },
    Known { name: "tsc", about: "TypeScript compiler", examples: &["tsc --noEmit"], network: false, version: V, markers: &["tsconfig.json"] },
    Known { name: "cargo", about: "Rust build tool and package manager", examples: &["cargo check", "cargo test -p <crate>", "cargo clippy --all-targets"], network: true, version: V, markers: &["Cargo.toml"] },
    Known { name: "rustc", about: "Rust compiler", examples: &["rustc --explain E0382"], network: false, version: V, markers: &["Cargo.toml"] },
    Known { name: "go", about: "Go toolchain", examples: &["go build ./...", "go test ./...", "go vet ./..."], network: true, version: Some(&["version"]), markers: &["go.mod"] },
    Known { name: "python", about: "Python interpreter", examples: &["python -m pytest", "python script.py"], network: false, version: V, markers: &["pyproject.toml", "requirements.txt", "setup.py"] },
    Known { name: "python3", about: "Python 3 interpreter", examples: &["python3 -m venv .venv"], network: false, version: V, markers: &["pyproject.toml", "requirements.txt"] },
    Known { name: "pip", about: "Python package installer", examples: &["pip install -r requirements.txt", "pip list --outdated"], network: true, version: V, markers: &["requirements.txt"] },
    Known { name: "uv", about: "fast Python package and project manager", examples: &["uv sync", "uv run pytest"], network: true, version: V, markers: &["uv.lock", "pyproject.toml"] },
    Known { name: "poetry", about: "Python dependency management", examples: &["poetry install", "poetry run pytest"], network: true, version: V, markers: &["poetry.lock"] },
    Known { name: "pytest", about: "Python test runner", examples: &["pytest -q", "pytest -k name"], network: false, version: V, markers: &["pytest.ini", "conftest.py"] },
    Known { name: "ruby", about: "Ruby interpreter", examples: &["ruby script.rb"], network: false, version: V, markers: &["Gemfile"] },
    Known { name: "bundle", about: "Ruby Bundler", examples: &["bundle install", "bundle exec rspec"], network: true, version: V, markers: &["Gemfile"] },
    Known { name: "php", about: "PHP interpreter", examples: &["php -l file.php", "php artisan test"], network: false, version: V, markers: &["composer.json"] },
    Known { name: "composer", about: "PHP dependency manager", examples: &["composer install", "composer outdated"], network: true, version: V, markers: &["composer.json"] },
    Known { name: "java", about: "Java runtime", examples: &["java -jar app.jar"], network: false, version: Some(&["-version"]), markers: &["pom.xml", "build.gradle"] },
    Known { name: "mvn", about: "Maven build tool", examples: &["mvn -q test"], network: true, version: V, markers: &["pom.xml"] },
    Known { name: "gradle", about: "Gradle build tool", examples: &["gradle test"], network: true, version: V, markers: &["build.gradle", "build.gradle.kts"] },
    Known { name: "dotnet", about: ".NET SDK: build, test, run", examples: &["dotnet build", "dotnet test"], network: true, version: V, markers: &["global.json"] },
    Known { name: "make", about: "build from a Makefile", examples: &["make", "make test"], network: false, version: V, markers: &["Makefile"] },
    Known { name: "just", about: "command runner (justfile)", examples: &["just --list", "just test"], network: false, version: V, markers: &["justfile"] },
    Known { name: "cmake", about: "C/C++ build configuration", examples: &["cmake -B build", "cmake --build build"], network: false, version: V, markers: &["CMakeLists.txt"] },
    Known { name: "curl", about: "HTTP requests", examples: &["curl -sS https://example.com/health", "curl -I <url>"], network: true, version: V, markers: &[] },
    Known { name: "wget", about: "download files over HTTP", examples: &["wget <url>"], network: true, version: V, markers: &[] },
    Known { name: "ssh", about: "remote shell", examples: &["ssh user@host 'uptime'"], network: true, version: None, markers: &[] },
    Known { name: "rsync", about: "sync files locally or over SSH", examples: &["rsync -av src/ dst/"], network: true, version: V, markers: &[] },
    Known { name: "jq", about: "query and transform JSON", examples: &["jq '.items[].name' data.json"], network: false, version: V, markers: &[] },
    Known { name: "yq", about: "query and transform YAML", examples: &["yq '.services' compose.yml"], network: false, version: V, markers: &[] },
    Known { name: "rg", about: "ripgrep: fast recursive text search", examples: &["rg -n 'TODO' src"], network: false, version: V, markers: &[] },
    Known { name: "fd", about: "fast file finder", examples: &["fd -e rs"], network: false, version: V, markers: &[] },
    Known { name: "tar", about: "create and extract archives", examples: &["tar -czf out.tgz dir", "tar -xzf in.tgz"], network: false, version: V, markers: &[] },
    Known { name: "7z", about: "7-Zip archiver", examples: &["7z x archive.7z"], network: false, version: None, markers: &[] },
    Known { name: "openssl", about: "certificates, keys and hashes", examples: &["openssl x509 -in cert.pem -noout -text"], network: false, version: Some(&["version"]), markers: &[] },
    Known { name: "gpg", about: "OpenPGP encryption and signatures", examples: &["gpg --verify file.sig file"], network: false, version: V, markers: &[] },
    Known { name: "pandoc", about: "convert documents (Markdown, HTML, DOCX, PDF)", examples: &["pandoc README.md -o README.pdf"], network: false, version: V, markers: &[] },
    Known { name: "shellcheck", about: "lint shell scripts", examples: &["shellcheck script.sh"], network: false, version: V, markers: &[] },
    Known { name: "eslint", about: "JavaScript/TypeScript linter", examples: &["eslint src"], network: false, version: V, markers: &[".eslintrc.json", "eslint.config.js"] },
    Known { name: "prettier", about: "code formatter", examples: &["prettier --check ."], network: false, version: V, markers: &[".prettierrc"] },
    Known { name: "ruff", about: "fast Python linter and formatter", examples: &["ruff check .", "ruff format ."], network: false, version: V, markers: &["ruff.toml"] },
    Known { name: "semgrep", about: "static analysis rules", examples: &["semgrep --config auto"], network: true, version: V, markers: &[] },
    Known { name: "pwsh", about: "PowerShell 7", examples: &["pwsh -NoProfile -Command Get-ChildItem"], network: false, version: V, markers: &[] },
    Known { name: "code", about: "Visual Studio Code CLI", examples: &["code --list-extensions"], network: false, version: None, markers: &[".vscode"] },
    Known { name: "ollama", about: "run local language models", examples: &["ollama list", "ollama run qwen3"], network: false, version: V, markers: &[] },
];

/// One program in the catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Known description, or the cached `--help` summary; empty when not described yet.
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub examples: Vec<String>,
    /// Reaches the network (left out of the prompt in privacy mode).
    #[serde(default)]
    pub network: bool,
    /// A well-known CLI (ranked first, version probed).
    #[serde(default)]
    pub known: bool,
}

/// `<kor home>/tools/catalog.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    pub version: u32,
    pub generated_at: u64,
    pub fingerprint: String,
    pub tools: Vec<Entry>,
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// `1.2.3` (or `1.2`) in a `--version` output.
pub fn parse_version(out: &str) -> Option<String> {
    let bytes = out.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit()
            && (i == 0 || !(bytes[i - 1].is_ascii_digit() || bytes[i - 1] == b'.'))
        {
            let start = i;
            let mut dots = 0;
            while i < bytes.len()
                && (bytes[i].is_ascii_digit()
                    || (bytes[i] == b'.' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit()))
            {
                if bytes[i] == b'.' {
                    dots += 1;
                }
                i += 1;
            }
            if dots >= 1 {
                return Some(out[start..i].to_owned());
            }
        }
        i += 1;
    }
    None
}

async fn probe_version(path: PathBuf, args: &'static [&'static str]) -> Option<String> {
    let mut cmd = tokio::process::Command::new(path);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    kor_platform::NoWindow::no_window(&mut cmd);
    let out = tokio::time::timeout(VERSION_TIMEOUT, cmd.output())
        .await
        .ok()?
        .ok()?;
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    parse_version(&text)
}

impl Catalog {
    /// Scan `PATH` now: every program by name, well-known ones with description and version.
    pub async fn scan(help_dir: Option<&Path>) -> Self {
        let found = kor_platform::on_path::executables();
        Self::from_found(found, help_dir, true).await
    }

    /// The catalog of `found` programs; `versions = false` skips running anything (tests).
    pub async fn from_found(
        found: BTreeMap<String, PathBuf>,
        help_dir: Option<&Path>,
        versions: bool,
    ) -> Self {
        let mut tools = Vec::new();
        let mut probes = Vec::new();
        for (name, path) in found.into_iter().take(MAX_ENTRIES) {
            let known = KNOWN.iter().find(|k| k.name == name);
            let cached = help_dir
                .and_then(|d| help::cached(d, &name))
                .map(|h| h.summary);
            if let (Some(args), true) = (known.and_then(|k| k.version), versions) {
                probes.push((tools.len(), probe_version(path.clone(), args)));
            }
            tools.push(Entry {
                description: known
                    .map(|k| k.about.to_owned())
                    .or(cached)
                    .unwrap_or_default(),
                examples: known
                    .map(|k| k.examples.iter().map(|s| (*s).to_owned()).collect())
                    .unwrap_or_default(),
                network: known.is_some_and(|k| k.network),
                known: known.is_some(),
                version: None,
                name,
                path,
            });
        }
        let (idx, futs): (Vec<usize>, Vec<_>) = probes.into_iter().unzip();
        for (i, v) in idx.into_iter().zip(futures::future::join_all(futs).await) {
            tools[i].version = v;
        }
        Self {
            version: CATALOG_VERSION,
            generated_at: now_ms(),
            fingerprint: kor_platform::on_path::path_fingerprint(),
            tools,
        }
    }

    pub fn read(file: &Path) -> Option<Self> {
        let c: Self = serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()?;
        (c.version == CATALOG_VERSION).then_some(c)
    }

    /// # Errors
    /// When the file cannot be written.
    pub fn write(&self, file: &Path) -> std::io::Result<()> {
        if let Some(d) = file.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = file.with_extension("kor-tmp");
        std::fs::write(
            &tmp,
            serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?,
        )?;
        std::fs::rename(&tmp, file)
    }

    /// Older than a day, or PATH gained/lost a program since.
    pub fn stale(&self, now: u64) -> bool {
        now.saturating_sub(self.generated_at) > MAX_AGE_MS
            || self.fingerprint != kor_platform::on_path::path_fingerprint()
    }

    /// The cached catalog in `dir` (`<kor home>/tools`), rebuilt when stale or `force`.
    pub async fn load(dir: &Path, force: bool) -> Self {
        let file = dir.join("catalog.json");
        if !force && let Some(c) = Self::read(&file).filter(|c| !c.stale(now_ms())) {
            return c;
        }
        let c = Self::scan(Some(&dir.join("help"))).await;
        let _ = c.write(&file);
        c
    }

    /// The cached catalog without scanning (prompt assembly must not wait); `None` when absent.
    pub fn cached(dir: &Path) -> Option<Self> {
        Self::read(&dir.join("catalog.json"))
    }

    pub fn get(&self, name: &str) -> Option<&Entry> {
        let n = name.trim_end_matches(".exe").to_ascii_lowercase();
        self.tools.iter().find(|t| t.name == n)
    }

    /// Keep only what `[tools]` allows (`kor_config::ToolsConfig::permits`).
    pub fn filter(&mut self, permits: impl Fn(&str) -> bool) {
        self.tools.retain(|t| permits(&t.name));
    }
}

/// Well-known tools that matter for the project at `root` (its marker files), then the rest.
fn relevance(e: &Entry, root: Option<&Path>) -> u8 {
    let Some(k) = KNOWN.iter().find(|k| k.name == e.name) else {
        return u8::from(!e.description.is_empty());
    };
    let marked = root.is_some_and(|r| k.markers.iter().any(|m| r.join(m).exists()));
    if marked { 3 } else { 2 }
}

/// System-prompt section: the most relevant tools within `budget_chars` (small context windows
/// get a short list, large ones a long one). Network tools are left out in privacy mode.
pub fn prompt_section(
    cat: &Catalog,
    defined: &[String],
    root: Option<&Path>,
    budget_chars: usize,
    privacy: bool,
) -> String {
    let mut ranked: Vec<&Entry> = cat
        .tools
        .iter()
        .filter(|t| !(privacy && t.network))
        .filter(|t| relevance(t, root) > 0)
        .collect();
    ranked.sort_by(|a, b| {
        relevance(b, root)
            .cmp(&relevance(a, root))
            .then_with(|| a.name.cmp(&b.name))
    });
    let mut out = String::from(
        "\n# Command-line tools on this computer\nRun them with the shell tool. For one you do not know, call `tools_search` with `describe` set to its name first (its --help, summarized).",
    );
    if !defined.is_empty() {
        let _ = write!(
            out,
            " Typed tools (load with ToolSearch): {}.",
            defined.join(", ")
        );
    }
    out.push('\n');
    let head = out.len();
    let mut shown = 0usize;
    for t in &ranked {
        let mut line = format!("- {}", t.name);
        if let Some(v) = &t.version {
            let _ = write!(line, " {v}");
        }
        if !t.description.is_empty() {
            let _ = write!(line, ": {}", t.description);
        }
        if let Some(ex) = t.examples.first() {
            let _ = write!(line, " (e.g. `{ex}`)");
        }
        line.push('\n');
        if out.len() + line.len() > head + budget_chars {
            break;
        }
        out.push_str(&line);
        shown += 1;
    }
    let more = ranked.len().saturating_sub(shown) + cat.tools.len().saturating_sub(ranked.len());
    if more > 0 {
        let _ = writeln!(
            out,
            "({more} more programs on PATH: `tools_search` finds them.)"
        );
    }
    out
}

/// Prompt budget for the catalog: about 1.5% of the context window, 600–6000 characters.
pub fn budget_chars(context_window: u64) -> usize {
    usize::try_from((context_window / 64 * 4).clamp(600, 6000)).unwrap_or(6000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_read_from_typical_outputs() {
        assert_eq!(
            parse_version("git version 2.45.1.windows.1").as_deref(),
            Some("2.45.1")
        );
        assert_eq!(
            parse_version("ffmpeg version 7.0-full_build").as_deref(),
            Some("7.0")
        );
        assert_eq!(
            parse_version("go version go1.22.3 linux/amd64").as_deref(),
            Some("1.22.3")
        );
        assert_eq!(parse_version("v20.11.0").as_deref(), Some("20.11.0"));
        assert_eq!(
            parse_version("openjdk version \"21.0.2\" 2024-01-16").as_deref(),
            Some("21.0.2")
        );
        assert_eq!(parse_version("x64 build"), None);
    }

    #[tokio::test]
    async fn catalog_ranks_project_tools_and_respects_the_budget() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "").unwrap();
        let found: BTreeMap<String, PathBuf> = ["git", "cargo", "curl", "zzlocal", "aws"]
            .iter()
            .map(|n| ((*n).to_owned(), dir.path().join(n)))
            .collect();
        let mut cat = Catalog::from_found(found, None, false).await;
        assert_eq!(cat.get("cargo.exe").map(|e| e.known), Some(true));
        assert!(cat.get("zzlocal").is_some_and(|e| e.description.is_empty()));
        let s = prompt_section(&cat, &["ffmpeg_cut".into()], Some(dir.path()), 4000, false);
        assert!(
            s.find("- cargo").unwrap() < s.find("- curl").unwrap(),
            "{s}"
        );
        assert!(s.contains("ffmpeg_cut") && !s.contains("- zzlocal"));
        let private = prompt_section(&cat, &[], Some(dir.path()), 4000, true);
        assert!(
            !private.contains("- curl") && !private.contains("- aws") && private.contains("- git")
        );
        let tiny = prompt_section(&cat, &[], Some(dir.path()), 60, false);
        assert!(
            tiny.matches("\n- ").count() <= 1 && tiny.contains("more programs"),
            "{tiny}"
        );
        cat.filter(|n| n != "curl");
        assert!(cat.get("curl").is_none());
        let f = dir.path().join("t/catalog.json");
        cat.write(&f).unwrap();
        assert_eq!(Catalog::read(&f), Some(cat.clone()));
        assert!(!cat.stale(cat.generated_at + 1000));
        assert!(cat.stale(cat.generated_at + MAX_AGE_MS + 1));
        assert_eq!(budget_chars(8_000), 600);
        assert_eq!(budget_chars(1_000_000), 6000);
    }
}
