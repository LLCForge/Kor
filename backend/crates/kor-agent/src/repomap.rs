//! Repository signature map (§5.3): one line per declaration instead of whole files, bounded by a
//! token budget, so a small-context model still sees the shape of a large repository.
//! Regex outlines for the main languages; when the project graph exists its hotspot ranking
//! decides which files make the budget ([`build_ranked`]).

use std::fmt::Write as _;
use std::path::Path;

use regex::Regex;

/// Longest signature line kept.
const LINE_CHARS: usize = 160;

/// `(extensions, declaration pattern)`; each match is one trimmed line of the map.
const OUTLINES: &[(&[&str], &str)] = &[
    (
        &["rs"],
        r"^\s*(pub(\([^)]*\))?\s+)?(async\s+|const\s+|unsafe\s+)*(fn|struct|enum|trait|impl|mod|type)\b[^{;]*",
    ),
    (&["py"], r"^\s*(async\s+)?(def|class)\s+\w+[^:]*"),
    (
        &["ts", "tsx", "js", "jsx", "mjs", "cjs"],
        r"^\s*(export\s+)?(default\s+)?(abstract\s+)?(async\s+)?(function\*?\s+\w+|class\s+\w+|interface\s+\w+|type\s+\w+\s*=|enum\s+\w+)[^{]*",
    ),
    (&["go"], r"^(func|type)\s+[^{]*"),
    (
        &["java", "cs", "kt", "scala"],
        r"^\s*((public|private|protected|internal|static|abstract|sealed|final|override|async|partial|data|open)\s+)*(class|interface|record|enum|struct|object|fun)\s+\w+[^{=]*|^\s*(public|protected|internal)\s+[\w<>\[\],.?\s]+\s+\w+\s*\([^)]*\)",
    ),
    (
        &["c", "h", "cpp", "cc", "hpp", "hh"],
        r"^(class|struct|namespace|enum)\s+\w+[^;{]*|^[A-Za-z_][\w:<>,\*&\s]*\s[\*&]*[A-Za-z_][\w:]*\([^;]*\)\s*(const\s*)?\{?\s*$",
    ),
    (
        &["php"],
        r"^\s*((abstract|final|public|private|protected|static)\s+)*(class|interface|trait|enum|function)\s+\w+[^{]*",
    ),
    (&["rb"], r"^\s*(class|module|def)\s+\S[^\n]*"),
];

/// Signature map of `root`, at most `max_tokens` (≈ 4 bytes per token). Empty when nothing fits.
pub fn build(
    root: &Path,
    ai_ignore: Option<&ignore::gitignore::Gitignore>,
    max_tokens: u64,
) -> String {
    build_ranked(root, ai_ignore, max_tokens, None)
}

/// [`build`] with files in `rank` order first (the project graph's hotspots, `graph::ranked_files`)
/// and the rest shallow-first; `None` keeps the plain shallow-first order.
pub fn build_ranked(
    root: &Path,
    ai_ignore: Option<&ignore::gitignore::Gitignore>,
    max_tokens: u64,
    rank: Option<&[String]>,
) -> String {
    if max_tokens == 0 {
        return String::new();
    }
    let outlines: Vec<(&[&str], Regex)> = OUTLINES
        .iter()
        .filter_map(|(exts, re)| Regex::new(re).ok().map(|r| (*exts, r)))
        .collect();
    let mut files = crate::semantic::walk(root, ai_ignore);
    // Shallow files first: the top of the tree says most about the project.
    let pos: std::collections::HashMap<&str, usize> = rank
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(i, f)| (f.as_str(), i))
        .collect();
    let key = |rel: &str| (pos.get(rel).copied().unwrap_or(usize::MAX), rel.matches('/').count());
    files.sort_by(|a, b| (key(&a.0), &a.0).cmp(&(key(&b.0), &b.0)));
    let header = if rank.is_some() {
        "\n# Repository map\nSignatures only (≈), most connected files first (project graph). Ask the graph tools before reading a file.\n"
    } else {
        "\n# Repository map\nSignatures only (≈). Read a file with `Read` when you need its body.\n"
    };
    let mut out = String::from(header);
    let mut skipped = 0usize;
    for (rel, path, _, _) in &files {
        let ext = rel.rsplit_once('.').map_or("", |(_, e)| e);
        let Some((_, re)) = outlines.iter().find(|(exts, _)| exts.contains(&ext)) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let mut block = format!("{rel}\n");
        for line in text.lines() {
            if let Some(m) = re.find(line) {
                let sig: String = m.as_str().trim().chars().take(LINE_CHARS).collect();
                let _ = writeln!(block, "  {}", sig.trim_end_matches(['{', ' ']));
            }
        }
        if block.lines().count() == 1 {
            continue;
        }
        if crate::context::tokens(out.len() + block.len()) > max_tokens {
            skipped += 1;
            continue;
        }
        out.push_str(&block);
    }
    if out.len() == header.len() {
        return String::new();
    }
    if skipped > 0 {
        let _ = writeln!(
            out,
            "… {skipped} more files not shown (budget); use `Grep`/`Glob`."
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_without_bodies_within_budget() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("lib.rs"),
            "pub fn add(a: u32, b: u32) -> u32 {\n    a + b // BODY\n}\nstruct S;\n",
        )
        .unwrap();
        std::fs::create_dir(root.join("web")).unwrap();
        std::fs::write(
            root.join("web/app.ts"),
            "export async function load(id: string): Promise<void> {\n  secretBody();\n}\nexport class View {}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("tool.py"),
            "class A:\n    def run(self, x):\n        pass\n",
        )
        .unwrap();
        std::fs::write(root.join("notes.txt"), "fn not_code()").unwrap();
        let map = build(root, None, 10_000);
        assert!(
            map.contains("lib.rs\n  pub fn add(a: u32, b: u32) -> u32\n"),
            "{map}"
        );
        assert!(map.contains("  struct S"), "{map}");
        assert!(
            map.contains("  export async function load(id: string): Promise<void>"),
            "{map}"
        );
        assert!(map.contains("  export class View"), "{map}");
        assert!(
            map.contains("  def run(self, x)") && map.contains("  class A"),
            "{map}"
        );
        assert!(!map.contains("BODY") && !map.contains("secretBody") && !map.contains("not_code"));
        // Shallow files come before nested ones.
        assert!(map.find("lib.rs").unwrap() < map.find("web/app.ts").unwrap());

        let small = build(root, None, 40);
        assert!(crate::context::tokens(small.len()) <= 40 + 20, "{small}");
        assert!(small.contains("more files not shown"), "{small}");
        assert!(build(root, None, 0).is_empty());

        // Graph ranking puts the hot file first, whatever its depth.
        let ranked = build_ranked(root, None, 10_000, Some(&["web/app.ts".to_owned()]));
        assert!(ranked.find("web/app.ts").unwrap() < ranked.find("lib.rs").unwrap(), "{ranked}");
        assert!(ranked.contains("project graph"));
    }
}
