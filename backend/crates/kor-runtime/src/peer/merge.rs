//! Shared memory between peers (spec §31.1 + §31.3, ADR 0057): knowledge records sync both
//! ways with a **three-way merge** against the state both sides agreed on at the last sync (the
//! base), never last-writer-wins.
//!
//! A record's identity across machines is its project, kind, time and task ([`key`]); local
//! database ids differ per machine. Per record: added on one side → added on the other; deleted
//! on one side and unchanged on the other → deleted; deleted on one side and edited on the other
//! → kept with the edit (a conflict is reported). Edited on both sides → field by field: lists
//! (files, decisions, errors, commands, rules) take both sides' additions and removals; the
//! summary is merged line by line (diff3), and overlapping edits keep both versions between
//! conflict markers.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use kor_memory::Record;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// Lines per side above which a text conflict is not split further.
const MAX_LINES: usize = 2_000;

/// Identity of a record on every machine.
pub fn key(r: &Record) -> String {
    let mut h = Sha256::new();
    for part in [r.project.as_str(), r.kind.as_str(), &r.ts.to_string(), r.task.as_str()] {
        h.update(part.as_bytes());
        h.update([0]);
    }
    super::hex(&h.finalize()[..16])
}

fn norm(mut r: Record) -> Record {
    r.id = 0;
    r
}

pub type Snapshot = BTreeMap<String, Record>;

pub fn snapshot(records: impl IntoIterator<Item = Record>) -> Snapshot {
    records.into_iter().map(|r| (key(&r), norm(r))).collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub key: String,
    pub task: String,
    /// `summary`, or `deleted` (one side deleted what the other edited).
    pub field: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    pub merged: Snapshot,
    pub conflicts: Vec<Conflict>,
}

/// diff3 of three texts by lines; `Err(text)` holds both versions between markers.
#[allow(clippy::many_single_char_names)] // o/a/b and i/j/k: the usual diff3 names
pub fn text3(base: &str, ours: &str, theirs: &str) -> Result<String, String> {
    if ours == theirs || theirs == base {
        return Ok(ours.to_owned());
    }
    if ours == base {
        return Ok(theirs.to_owned());
    }
    let (o, a, b): (Vec<&str>, Vec<&str>, Vec<&str>) = (
        base.lines().collect(),
        ours.lines().collect(),
        theirs.lines().collect(),
    );
    let marked = || format!("<<<<<<< ours\n{ours}\n=======\n{theirs}\n>>>>>>> theirs");
    if o.len() > MAX_LINES || a.len() > MAX_LINES || b.len() > MAX_LINES {
        return Err(marked());
    }
    let ma = matches(&o, &a);
    let mb = matches(&o, &b);
    let mut out: Vec<&str> = Vec::new();
    let mut conflict = false;
    let (mut i, mut j, mut k) = (0, 0, 0);
    loop {
        // Next base line kept by both sides at or after the current positions.
        let sync = (i..o.len()).find(|&x| {
            ma[x].is_some_and(|y| y >= j) && mb[x].is_some_and(|y| y >= k)
        });
        let (i1, j1, k1) = match sync {
            Some(x) => (x, ma[x].unwrap_or(a.len()), mb[x].unwrap_or(b.len())),
            None => (o.len(), a.len(), b.len()),
        };
        let (co, ca, cb) = (&o[i..i1], &a[j..j1], &b[k..k1]);
        if ca == co {
            out.extend_from_slice(cb);
        } else if cb == co || ca == cb {
            out.extend_from_slice(ca);
        } else {
            conflict = true;
            out.push("<<<<<<< ours");
            out.extend_from_slice(ca);
            out.push("=======");
            out.extend_from_slice(cb);
            out.push(">>>>>>> theirs");
        }
        let Some(x) = sync else { break };
        out.push(o[x]);
        (i, j, k) = (x + 1, j1 + 1, k1 + 1);
    }
    let joined = out.join("\n");
    if conflict { Err(joined) } else { Ok(joined) }
}

/// For each line of `o`, the line of `x` it is matched to by the longest common subsequence.
#[allow(clippy::many_single_char_names)] // LCS table indices
fn matches(o: &[&str], x: &[&str]) -> Vec<Option<usize>> {
    let (n, m) = (o.len(), x.len());
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if o[i] == x[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut res = vec![None; n];
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if o[i] == x[j] {
            res[i] = Some(j);
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    res
}

/// Both sides' additions and removals relative to `base`, in a stable order.
fn list3(base: &[String], ours: &[String], theirs: &[String]) -> Vec<String> {
    let removed: BTreeSet<&String> = base
        .iter()
        .filter(|x| !ours.contains(x) || !theirs.contains(x))
        .collect();
    let mut out: Vec<String> = Vec::new();
    for x in ours.iter().chain(theirs) {
        if !removed.contains(x) && !out.contains(x) {
            out.push(x.clone());
        }
    }
    out
}

fn record3(base: &Record, ours: &Record, theirs: &Record) -> (Record, bool) {
    let mut r = ours.clone();
    let (summary, conflict) = match text3(&base.summary, &ours.summary, &theirs.summary) {
        Ok(s) => (s, false),
        Err(s) => (s, true),
    };
    r.summary = summary;
    r.files = list3(&base.files, &ours.files, &theirs.files);
    r.decisions = list3(&base.decisions, &ours.decisions, &theirs.decisions);
    r.errors = list3(&base.errors, &ours.errors, &theirs.errors);
    r.commands = list3(&base.commands, &ours.commands, &theirs.commands);
    r.rules = list3(&base.rules, &ours.rules, &theirs.rules);
    (r, conflict)
}

/// Merge `ours` and `theirs` against `base` (all keyed by [`key`]).
#[allow(clippy::many_single_char_names)] // b/o/t: base, ours, theirs
pub fn merge(base: &Snapshot, ours: &Snapshot, theirs: &Snapshot) -> Outcome {
    let keys: BTreeSet<&String> = base.keys().chain(ours.keys()).chain(theirs.keys()).collect();
    let mut out = Outcome::default();
    let conflict = |k: &str, r: &Record, field: &str| Conflict {
        key: k.to_owned(),
        task: r.task.clone(),
        field: field.to_owned(),
    };
    for k in keys {
        let (b, o, t) = (base.get(k), ours.get(k), theirs.get(k));
        let pick = match (b, o, t) {
            (None, Some(o), None) => Some(o.clone()),
            (None, None, Some(t)) => Some(t.clone()),
            (None, Some(o), Some(t)) => {
                let empty = Record {
                    summary: String::new(),
                    ..o.clone()
                };
                let empty = Record {
                    files: vec![],
                    decisions: vec![],
                    errors: vec![],
                    commands: vec![],
                    rules: vec![],
                    ..empty
                };
                let (r, c) = record3(&empty, o, t);
                if c {
                    out.conflicts.push(conflict(k, o, "summary"));
                }
                Some(r)
            }
            // Deleted on one side: gone if the other side left it as it was.
            (Some(b), Some(o), None) | (Some(b), None, Some(o)) => {
                if o == b {
                    None
                } else {
                    out.conflicts.push(conflict(k, o, "deleted"));
                    Some(o.clone())
                }
            }
            (Some(_) | None, None, None) => None,
            (Some(b), Some(o), Some(t)) => {
                let (r, c) = record3(b, o, t);
                if c {
                    out.conflicts.push(conflict(k, o, "summary"));
                }
                Some(r)
            }
        };
        if let Some(r) = pick {
            out.merged.insert(k.clone(), r);
        }
    }
    out
}

/// The base of the last sync with `peer` (`<peer dir>/sync/<peer>.json`); empty if none.
pub fn load_base(dir: &Path, peer: &str) -> Snapshot {
    std::fs::read(dir.join("sync").join(format!("{peer}.json")))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// # Errors
/// On I/O errors.
pub fn save_base(dir: &Path, peer: &str, s: &Snapshot) -> Result<(), String> {
    let d = dir.join("sync");
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    let tmp = d.join(format!("{peer}.json.tmp"));
    std::fs::write(&tmp, serde_json::to_vec(s).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, d.join(format!("{peer}.json"))).map_err(|e| e.to_string())
}

/// Every record in the knowledge base of `home`.
///
/// # Errors
/// When the database cannot be opened or read.
pub fn local_records(home: &Path) -> Result<Vec<Record>, String> {
    let kb = kor_memory::Knowledge::open(&kor_memory::knowledge_path(home))
        .map_err(|e| e.to_string())?;
    let n = usize::try_from(kb.count().map_err(|e| e.to_string())?).unwrap_or(usize::MAX);
    kb.recent(None, n.max(1)).map_err(|e| e.to_string())
}

/// Counts of what applying a merge did to the local base.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Applied {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
}

/// Make the knowledge base of `home` equal to `merged`.
///
/// # Errors
/// On database errors.
pub fn apply(home: &Path, merged: &Snapshot) -> Result<Applied, String> {
    let kb = kor_memory::Knowledge::open(&kor_memory::knowledge_path(home))
        .map_err(|e| e.to_string())?;
    let n = usize::try_from(kb.count().map_err(|e| e.to_string())?).unwrap_or(usize::MAX);
    let local = kb.recent(None, n.max(1)).map_err(|e| e.to_string())?;
    let mut a = Applied::default();
    let mut seen = BTreeSet::new();
    let mut new = Vec::new();
    for r in local {
        let k = key(&r);
        let id = r.id;
        match merged.get(&k) {
            None => {
                kb.forget(id).map_err(|e| e.to_string())?;
                a.removed += 1;
            }
            Some(m) if *m != norm(r) => {
                kb.forget(id).map_err(|e| e.to_string())?;
                new.push(m.clone());
                a.updated += 1;
            }
            Some(_) => {}
        }
        seen.insert(k);
    }
    for (k, r) in merged {
        if !seen.contains(k) {
            new.push(r.clone());
            a.added += 1;
        }
    }
    if !new.is_empty() {
        kb.record_all(new).map_err(|e| e.to_string())?;
    }
    kb.flush().map_err(|e| e.to_string())?;
    Ok(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(task: &str, summary: &str, decisions: &[&str]) -> Record {
        Record {
            ts: 100,
            project: "p".into(),
            kind: "task".into(),
            task: task.into(),
            summary: summary.into(),
            decisions: decisions.iter().map(|s| (*s).to_owned()).collect(),
            ..Record::default()
        }
    }

    #[test]
    fn text_merge_takes_both_sides_and_marks_overlaps() {
        let base = "a\nb\nc\nd";
        assert_eq!(text3(base, "A\nb\nc\nd", "a\nb\nc\nD").unwrap(), "A\nb\nc\nD");
        let e = text3(base, "a\nX\nc\nd", "a\nY\nc\nd").unwrap_err();
        assert!(e.contains("<<<<<<< ours\nX\n=======\nY\n>>>>>>> theirs"), "{e}");
        assert_eq!(text3("x", "x", "y").unwrap(), "y");
    }

    #[test]
    fn three_way_not_last_writer_wins() {
        let shared = rec("t1", "line one\nkeep\nline two", &["use sqlite"]);
        let gone = rec("t2", "old", &[]);
        let base = snapshot([shared.clone(), gone.clone()]);
        // Ours: edits line one, adds a decision; deletes t2 untouched by them.
        let mut o1 = shared.clone();
        o1.summary = "LINE ONE\nkeep\nline two".into();
        o1.decisions.push("wal mode".into());
        // Theirs: edits line two, removes "use sqlite", adds t3. (Edits to adjacent lines with
        // no unchanged line between them conflict, as in diff3.)
        let mut t1 = shared.clone();
        t1.summary = "line one\nkeep\nLINE TWO".into();
        t1.decisions.clear();
        let t3 = rec("t3", "from the server", &[]);
        let ours = snapshot([o1]);
        let theirs = snapshot([t1, gone.clone(), t3.clone()]);
        let out = merge(&base, &ours, &theirs);
        assert!(out.conflicts.is_empty(), "{:?}", out.conflicts);
        let m = &out.merged[&key(&shared)];
        assert_eq!(m.summary, "LINE ONE\nkeep\nLINE TWO");
        assert_eq!(m.decisions, ["wal mode"]);
        assert!(!out.merged.contains_key(&key(&gone)));
        assert!(out.merged.contains_key(&key(&t3)));

        // Delete vs edit keeps the edit and reports it.
        let mut edited = gone.clone();
        edited.summary = "new".into();
        let out = merge(&base, &snapshot([shared.clone()]), &snapshot([shared.clone(), edited]));
        assert_eq!(out.merged[&key(&gone)].summary, "new");
        assert_eq!(out.conflicts[0].field, "deleted");

        // Both edited the same line: both kept, conflict reported.
        let mut a = shared.clone();
        a.summary = "mine\nkeep\nline two".into();
        let mut b = shared.clone();
        b.summary = "theirs\nkeep\nline two".into();
        let out = merge(&base, &snapshot([a]), &snapshot([b]));
        assert_eq!(out.conflicts.len(), 1);
        assert!(out.merged[&key(&shared)].summary.contains("mine"));
        assert!(out.merged[&key(&shared)].summary.contains("theirs"));
    }

    #[test]
    fn apply_makes_the_base_equal_to_the_merge() {
        let home = tempfile::tempdir().unwrap();
        let kb = kor_memory::Knowledge::open(&kor_memory::knowledge_path(home.path())).unwrap();
        kb.record(rec("a", "x", &[])).unwrap();
        kb.record(rec("b", "y", &[])).unwrap();
        kb.flush().unwrap();
        drop(kb);
        let mut b2 = rec("b", "y2", &[]);
        b2.files.push("f.rs".into());
        let merged = snapshot([b2.clone(), rec("c", "z", &[])]);
        let a = apply(home.path(), &merged).unwrap();
        assert_eq!(a, Applied { added: 1, updated: 1, removed: 1 });
        let now = snapshot(local_records(home.path()).unwrap());
        assert_eq!(now, merged);
    }
}
