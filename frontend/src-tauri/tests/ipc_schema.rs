//! §16.3 (ADR 0061): the Rust bridge and `kor-core/schema/ipc.json` describe the same surface.
//! The TS side is generated from the schema (`frontend/scripts/ipc-gen.test.ts` keeps it fresh), so a
//! method or event that changes here without the schema fails this test and Rust/TS cannot drift.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use serde_json::Value;

fn src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

fn rs_files(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    out.sort();
    out
}

fn schema() -> Value {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../backend/crates/kor-core/schema/ipc.json");
    serde_json::from_str(&read(&p)).expect("ipc.json parses")
}

/// Namespace → file stem, from `ns::call` routing (`"a" | "b" => x::call`, `x::NS => x::call`) and every
/// `pub const NS` (modules the orchestrator has not routed yet still own their namespace).
fn owners() -> HashMap<String, String> {
    let ns_dir = src().join("ns");
    let mut owner = HashMap::new();
    for f in rs_files(&ns_dir) {
        let stem = f.file_stem().unwrap().to_string_lossy().into_owned();
        let body = read(&f);
        if let Some(ns) = body.lines().find(|l| l.starts_with("pub const NS: &str")).and_then(|l| l.split('"').nth(1)) {
            owner.insert(ns.to_owned(), stem.clone());
        }
        // Files that serve several namespaces match on `(ns, name)`: `("plan", "approve") =>`.
        for (i, _) in body.match_indices("(\"") {
            let rest = &body[i + 2..];
            if let Some((ns, tail)) = rest.split_once('"') {
                if tail.starts_with(", ") && ns.chars().all(|c| c.is_ascii_alphanumeric()) && !ns.is_empty() {
                    owner.entry(ns.to_owned()).or_insert_with(|| stem.clone());
                }
            }
        }
    }
    let mod_rs = read(&ns_dir.join("mod.rs"));
    let lines: Vec<&str> = mod_rs.lines().collect();
    for (i, line) in lines.iter().enumerate().filter(|(_, l)| l.contains("::call(ns, name")) {
        let (pat, target) = match line.split_once("=>") {
            Some(split) => split,
            None => match lines[..i].last().and_then(|p| p.split_once("=>")) {
                Some((pat, _)) => (pat, *line),
                None => continue,
            },
        };
        if pat.contains("::NS") {
            continue;
        }
        let file = target.trim().split("::").next().unwrap_or_default().to_owned();
        for n in pat.split('"').skip(1).step_by(2) {
            owner.insert(n.to_owned(), file.clone());
        }
    }
    owner
}

/// `"name"` is a match arm: followed by `=>`, `|`, `) =>` (tuple arm), the last of an `a | b` list, or a
/// `== "name"` / `!= "name"` guard (single-method modules return early for other names).
fn has_arm(body: &str, name: &str) -> bool {
    let lit = format!("\"{name}\"");
    body.match_indices(&lit).any(|(i, _)| {
        let after = body[i + lit.len()..].trim_start();
        let after = after.strip_prefix(')').map_or(after, str::trim_start);
        let before = body[..i].trim_end();
        after.starts_with("=>")
            || after.starts_with('|')
            || before.ends_with("==")
            || before.ends_with("!=")
            || before.ends_with('|')
    })
}

#[test]
fn every_schema_method_has_a_rust_handler() {
    let s = schema();
    let owner = owners();
    let root: String = rs_files(&src()).iter().map(|p| read(p)).collect();
    let mut missing = Vec::new();
    for method in s["methods"].as_object().expect("methods").keys() {
        // Sync helpers the renderer computes itself (`x-sync`) never reach Rust.
        if s["methods"][method]["x-sync"] == true {
            continue;
        }
        let ok = match method.rsplit_once('.') {
            Some((ns, name)) => owner.get(ns).is_some_and(|file| {
                let body = read(&src().join("ns").join(format!("{file}.rs")));
                has_arm(&body, name) || has_arm(&body, method)
            }),
            None => has_arm(&root, method),
        };
        if !ok {
            missing.push(method.clone());
        }
    }
    assert!(missing.is_empty(), "schema methods without a Rust handler: {missing:?}");
}

/// Every `"kor:…"` event the app emits (literal names next to `emit(`, and `EVENT` constants).
fn emitted() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut files = rs_files(&src());
    files.extend(rs_files(&src().join("ns")));
    for f in files {
        let text = read(&f);
        let lines: Vec<&str> = text.lines().collect();
        for (i, l) in lines.iter().enumerate() {
            let near = l.contains(".emit(") || l.contains("emit_to(") || l.contains("const EVENT");
            if !near {
                continue;
            }
            for w in &lines[i..lines.len().min(i + 3)] {
                for lit in w.split('"').skip(1).step_by(2) {
                    if lit.starts_with("kor:") && !lit.contains(' ') && lit.len() > 4 {
                        out.insert(lit.to_owned());
                    }
                }
            }
        }
    }
    out
}

#[test]
fn emitted_events_and_schema_events_match() {
    let s = schema();
    let schema_events: BTreeSet<String> = s["events"].as_object().expect("events").keys().cloned().collect();
    let emitted = emitted();
    let unknown: Vec<_> = emitted.difference(&schema_events).collect();
    assert!(unknown.is_empty(), "Rust emits events missing from ipc.json: {unknown:?}");
    let never: Vec<_> = schema_events.difference(&emitted).collect();
    assert!(never.is_empty(), "ipc.json events nothing in Rust emits (dead subscriptions): {never:?}");
}

#[test]
fn parity_desktop_methods_are_in_the_schema() {
    let s = schema();
    let parity = read(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity.toml"));
    let doc: toml_edit::DocumentMut = parity.parse().expect("parity.toml");
    let mut missing = Vec::new();
    for f in doc["feature"].as_array_of_tables().expect("features") {
        let Some(d) = f.get("desktop").and_then(toml_edit::Item::as_str) else { continue };
        if d != "-" && s["methods"].get(d).is_none() {
            missing.push(d.to_owned());
        }
    }
    assert!(missing.is_empty(), "parity.toml desktop methods missing from ipc.json: {missing:?}");
}

#[test]
fn agent_event_types_sent_by_the_app_are_in_the_schema() {
    let s = schema();
    // AgentEventBody variants are inline or behind `$ref`; the tagged `type` const is searched in every def.
    let text = serde_json::to_string(&s["$defs"]).expect("$defs");
    let mut missing = BTreeSet::new();
    for f in [src().join("chat.rs"), src().join("presence.rs")] {
        // Lines that build `kor:event` items; the Chrome side panel's broadcast has its own types.
        for line in read(&f).lines().filter(|l| !l.contains("broadcast_event")) {
            for (i, _) in line.match_indices("\"type\": \"") {
                let ty = line[i + 9..].split('"').next().unwrap_or_default().to_owned();
                if !text.contains(&format!("\"type\":{{\"const\":\"{ty}\"}}")) {
                    missing.insert(ty);
                }
            }
        }
    }
    assert!(missing.is_empty(), "`kor:event` types missing from AgentEventBody: {missing:?}");
}
