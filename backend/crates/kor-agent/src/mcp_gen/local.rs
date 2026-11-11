//! `kor mcp add ./my-service`: find how the folder's program starts and register it as a stdio
//! server. Order: an MCP definition already in the folder (`.mcp.json`/`mcp.json` with one
//! server), `package.json` (`bin`, `main`, `start` script), `pyproject.toml`
//! (`[project.scripts]`), `Cargo.toml` (built binary, else `cargo run`), then `server.py` /
//! `main.py`. Paths are absolute so the server starts from any project.

use std::path::Path;

use serde_json::Value;

/// How to start the folder's server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub command: String,
    pub args: Vec<String>,
    /// What was detected (and whether it looks like an MCP server).
    pub note: String,
}

fn abs(dir: &Path, rel: &str) -> String {
    dir.join(rel).to_string_lossy().into_owned()
}

fn found(command: &str, args: Vec<String>, note: String) -> Found {
    Found {
        command: command.into(),
        args,
        note,
    }
}

fn mentions_mcp(text: &str) -> bool {
    text.contains("modelcontextprotocol")
        || text.contains("\"mcp\"")
        || text.contains("mcp =")
        || text.contains("rmcp")
        || text.contains("fastmcp")
}

/// Detect the start command of the service in `dir`.
///
/// # Errors
/// If nothing startable is found.
#[allow(clippy::too_many_lines)] // one block per project kind
pub fn detect(dir: &Path) -> Result<Found, String> {
    let dir = dir
        .canonicalize()
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    let text = dir.to_string_lossy();
    let dir = std::path::PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text));
    let read = |f: &str| {
        std::fs::read_to_string(dir.join(f))
            .ok()
            .filter(|t| t.len() < 1 << 20)
    };

    for f in [".mcp.json", "mcp.json"] {
        if let Some(v) = read(f).and_then(|t| serde_json::from_str::<Value>(&t).ok())
            && let Some(servers) = v["mcpServers"].as_object()
            && servers.len() == 1
            && let Some((name, s)) = servers.iter().next()
            && let Some(cmd) = s["command"].as_str()
        {
            let args = crate::mcp_config::strings(s.get("args"))
                .into_iter()
                .map(|a| {
                    if a.starts_with("./") || a.starts_with(".\\") {
                        abs(&dir, &a[2..])
                    } else {
                        a
                    }
                })
                .collect();
            return Ok(found(cmd, args, format!("MCP server `{name}` from {f}")));
        }
    }

    if let Some(text) = read("package.json") {
        let pkg: Value = serde_json::from_str(&text).map_err(|e| format!("package.json: {e}"))?;
        let mcp = if mentions_mcp(&text) {
            " (uses the MCP SDK)"
        } else {
            ""
        };
        let bin = match &pkg["bin"] {
            Value::String(s) => Some(s.clone()),
            Value::Object(o) => o.values().find_map(Value::as_str).map(str::to_owned),
            _ => None,
        };
        if let Some(entry) = bin.or_else(|| pkg["main"].as_str().map(str::to_owned)) {
            return Ok(found(
                "node",
                vec![abs(&dir, &entry)],
                format!("Node.js: {entry}{mcp}"),
            ));
        }
        if pkg["scripts"]["start"].is_string() {
            let d = dir.to_string_lossy().into_owned();
            return Ok(found(
                "npm",
                vec![
                    "--prefix".into(),
                    d,
                    "run".into(),
                    "--silent".into(),
                    "start".into(),
                ],
                format!("Node.js: npm start{mcp}"),
            ));
        }
    }

    if let Some(text) = read("pyproject.toml") {
        let doc: toml::Value = toml::from_str(&text).map_err(|e| format!("pyproject.toml: {e}"))?;
        let mcp = if mentions_mcp(&text) {
            " (uses the MCP SDK)"
        } else {
            ""
        };
        if let Some(script) = doc
            .get("project")
            .and_then(|p| p.get("scripts"))
            .and_then(toml::Value::as_table)
            .and_then(|t| t.keys().next().cloned())
        {
            let d = dir.to_string_lossy().into_owned();
            return Ok(found(
                "uv",
                vec!["run".into(), "--directory".into(), d, script.clone()],
                format!("Python: {script}{mcp} (run with uv)"),
            ));
        }
    }

    if let Some(text) = read("Cargo.toml") {
        let doc: toml::Value = toml::from_str(&text).map_err(|e| format!("Cargo.toml: {e}"))?;
        let mcp = if mentions_mcp(&text) {
            " (uses an MCP crate)"
        } else {
            ""
        };
        let name = doc
            .get("bin")
            .and_then(toml::Value::as_array)
            .and_then(|b| b.first())
            .and_then(|b| b.get("name"))
            .or_else(|| doc.get("package").and_then(|p| p.get("name")))
            .and_then(toml::Value::as_str)
            .ok_or("Cargo.toml has no package or bin name")?;
        let exe = kor_platform::exe(name);
        for profile in ["release", "debug"] {
            let built = dir.join("target").join(profile).join(&exe);
            if built.is_file() {
                return Ok(found(
                    &built.to_string_lossy(),
                    Vec::new(),
                    format!("Rust: target/{profile}/{exe}{mcp}"),
                ));
            }
        }
        let manifest = abs(&dir, "Cargo.toml");
        return Ok(found(
            "cargo",
            vec![
                "run".into(),
                "--quiet".into(),
                "--release".into(),
                "--manifest-path".into(),
                manifest,
                "--bin".into(),
                name.into(),
            ],
            format!("Rust: cargo run --bin {name}{mcp}"),
        ));
    }

    for f in ["server.py", "main.py", "__main__.py"] {
        if dir.join(f).is_file() {
            let py = kor_platform::python_command();
            return Ok(found(py, vec![abs(&dir, f)], format!("Python: {f}")));
        }
    }
    Err(format!(
        "{}: no package.json, pyproject.toml, Cargo.toml, .mcp.json or server.py to start",
        dir.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_each_kind() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        assert!(detect(d).is_err());
        std::fs::write(d.join("server.py"), "").unwrap();
        assert!(detect(d).unwrap().args[0].ends_with("server.py"));
        std::fs::write(
            d.join("Cargo.toml"),
            "[package]\nname = \"svc\"\n[dependencies]\nrmcp = \"0.1\"\n",
        )
        .unwrap();
        let f = detect(d).unwrap();
        assert_eq!(f.command, "cargo");
        assert!(f.note.contains("MCP crate"));
        std::fs::write(
            d.join("pyproject.toml"),
            "[project]\nname = \"x\"\n[project.scripts]\nsvc-mcp = \"svc:main\"\n",
        )
        .unwrap();
        assert_eq!(detect(d).unwrap().args.last().unwrap(), "svc-mcp");
        std::fs::write(
            d.join("package.json"),
            r#"{"bin":{"svc":"dist/index.js"},"dependencies":{"@modelcontextprotocol/sdk":"1"}}"#,
        )
        .unwrap();
        let f = detect(d).unwrap();
        assert_eq!(f.command, "node");
        assert!(f.args[0].ends_with("index.js") && f.note.contains("MCP SDK"));
        std::fs::write(
            d.join(".mcp.json"),
            r#"{"mcpServers":{"svc":{"command":"node","args":["./x.js"]}}}"#,
        )
        .unwrap();
        let f = detect(d).unwrap();
        assert!(Path::new(&f.args[0]).is_absolute() && f.note.contains("svc"));
    }
}
