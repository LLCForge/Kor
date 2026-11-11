//! Ajanın dosya değişiklikleri: araç yazmadan önce içerik alınır, bitince karşılaştırılıp `fileChanged`
//! olayı yayılır (değişiklik kutusu, fark kartları). Sohbetteki ilk içerik saklanır; `revertFile` dosyayı
//! o hâline döndürür (sohbette oluşturulduysa siler). Yalnızca açık klasörün içindeki ve bu sohbette
//! değişmiş dosyalara dokunulur.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde_json::{Value, json};

/// Bundan büyük metin dosyaları takip edilmez (bellek); ikili dosyalar da.
const MAX_TRACKED: u64 = 2 * 1024 * 1024;
/// Bundan büyük içerik olayla gönderilmez; arayüz `getFileChange` ile ister.
const LARGE: usize = 256 * 1024;
const FILE_TOOLS: &[&str] = &["Write", "Edit", "MultiEdit", "NotebookEdit", "Delete"];

#[derive(Default)]
struct Store {
    /// oturum → dosya → sohbetteki ilk içerik (`None`: dosya yoktu).
    originals: HashMap<String, HashMap<PathBuf, Option<String>>>,
    /// araç kimliği → (dosya, araçtan önceki içerik).
    pending: HashMap<String, (PathBuf, Option<String>)>,
}

fn store() -> std::sync::MutexGuard<'static, Store> {
    static S: OnceLock<Mutex<Store>> = OnceLock::new();
    S.get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Kök içindeki yol; `..` ile kaçan ya da kök dışındaki mutlak yol `None`.
pub fn inside(root: &Path, raw: &str) -> Option<PathBuf> {
    let raw = raw.trim();
    if raw.is_empty() || raw.contains('\0') {
        return None;
    }
    let joined = root.join(raw);
    let mut out = PathBuf::new();
    for c in joined.components() {
        match c {
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out.starts_with(root).then_some(out)
}

/// Metin içeriği; yoksa `Some(None)`, takip edilemezse (büyük/ikili/okunamaz) `None`.
fn snapshot(path: &Path) -> Option<Option<String>> {
    match std::fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(None),
        Err(_) => None,
        Ok(m) if !m.is_file() || m.len() > MAX_TRACKED => None,
        Ok(_) => {
            let bytes = std::fs::read(path).ok()?;
            if bytes.iter().take(8192).any(|b| *b == 0) {
                return None;
            }
            String::from_utf8(bytes).ok().map(Some)
        }
    }
}

/// Dosya aracı başlarken çağrılır: önceki içerik ve (ilk kez görülüyorsa) sohbetteki ilk içerik saklanır.
pub fn before_tool(session: &str, root: &Path, tool_id: &str, name: &str, target: &str) {
    if !FILE_TOOLS.contains(&name) {
        return;
    }
    let Some(path) = inside(root, target) else {
        return;
    };
    let Some(previous) = snapshot(&path) else {
        return;
    };
    let mut s = store();
    s.originals
        .entry(session.to_owned())
        .or_default()
        .entry(path.clone())
        .or_insert_with(|| previous.clone());
    s.pending.insert(tool_id.to_owned(), (path, previous));
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Satır bazında kaba ekleme/silme sayısı (büyük dosyalar için özet).
fn line_stats(before: &str, after: &str) -> (usize, usize) {
    let mut counts: HashMap<&str, i64> = HashMap::new();
    for l in before.lines() {
        *counts.entry(l).or_default() -= 1;
    }
    for l in after.lines() {
        *counts.entry(l).or_default() += 1;
    }
    let added = counts
        .values()
        .filter(|v| **v > 0)
        .map(|v| *v as usize)
        .sum();
    let removed = counts
        .values()
        .filter(|v| **v < 0)
        .map(|v| (-*v) as usize)
        .sum();
    (added, removed)
}

/// Araç bitince: içerik değiştiyse `fileChanged` olayı.
pub fn after_tool(session: &str, root: &Path, tool_id: &str, is_error: bool) -> Option<Value> {
    let (path, previous) = store().pending.remove(tool_id)?;
    if is_error {
        return None;
    }
    let after = snapshot(&path)?;
    // Dosya önce de sonra da yok: araç başka bir yola yazdı (ya da özet yolu tutmadı); olay uydurulmaz.
    if previous.is_none() && after.is_none() {
        return None;
    }
    let after = after.unwrap_or_default();
    if previous.as_deref() == Some(after.as_str()) {
        return None;
    }
    let before = store()
        .originals
        .get(session)
        .and_then(|m| m.get(&path))
        .cloned()
        .flatten();
    let mut ev = json!({
        "type": "fileChanged",
        "path": rel(root, &path),
        "before": before,
        "previous": previous,
        "after": after,
        "toolId": tool_id,
    });
    if after.len() > LARGE || previous.as_ref().is_some_and(|p| p.len() > LARGE) {
        let (added, removed) = line_stats(previous.as_deref().unwrap_or(""), &after);
        ev["before"] = Value::Null;
        ev["previous"] = Value::Null;
        ev["after"] = json!("");
        ev["large"] = json!({ "added": added, "removed": removed, "bytes": after.len() });
    }
    Some(ev)
}

/// `getFileChange`: sohbetteki ilk içerik ve şimdiki içerik.
pub fn file_change(session: &str, root: &Path, rel_path: &str) -> Option<Value> {
    let path = inside(root, rel_path)?;
    let before = store().originals.get(session)?.get(&path)?.clone();
    let after = snapshot(&path)?.unwrap_or_default();
    Some(json!({ "before": before, "after": after }))
}

/// `revertFile`: dosyayı sohbetteki ilk hâline döndürür.
pub fn revert(session: &str, root: &Path, rel_path: &str) -> Result<(), String> {
    let path = inside(root, rel_path).ok_or("Yol açık klasörün dışında")?;
    let original = store()
        .originals
        .get(session)
        .and_then(|m| m.get(&path))
        .cloned()
        .ok_or("Bu dosya bu sohbette değişmedi")?;
    match original {
        Some(text) => {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            std::fs::write(&path, text).map_err(|e| format!("Geri alınamadı: {e}"))?;
        }
        None => match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("Geri alınamadı: {e}")),
        },
    }
    if let Some(m) = store().originals.get_mut(session) {
        m.remove(&path);
    }
    Ok(())
}

pub fn forget(session: &str) {
    store().originals.remove(session);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_stay_inside_the_root() {
        let root = Path::new(r"C:\proj");
        assert_eq!(
            inside(root, "src/a.rs").unwrap(),
            root.join("src").join("a.rs")
        );
        assert!(inside(root, "../x").is_none());
        assert!(inside(root, r"C:\other\x").is_none());
        assert!(inside(root, "").is_none());
    }

    #[test]
    fn change_event_then_revert_restores_the_first_version() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "bir\n").unwrap();

        before_tool("s1", root, "t1", "Edit", "a.txt");
        std::fs::write(root.join("a.txt"), "iki\n").unwrap();
        let ev = after_tool("s1", root, "t1", false).unwrap();
        assert_eq!(ev["path"], "a.txt");
        assert_eq!(ev["before"], "bir\n");
        assert_eq!(ev["previous"], "bir\n");
        assert_eq!(ev["after"], "iki\n");

        before_tool("s1", root, "t2", "Edit", "a.txt");
        std::fs::write(root.join("a.txt"), "üç\n").unwrap();
        let ev = after_tool("s1", root, "t2", false).unwrap();
        assert_eq!(
            ev["before"], "bir\n",
            "the first version of the conversation"
        );
        assert_eq!(ev["previous"], "iki\n");

        before_tool("s1", root, "t3", "Write", "new.txt");
        std::fs::write(root.join("new.txt"), "yeni").unwrap();
        assert!(after_tool("s1", root, "t3", false).unwrap()["before"].is_null());

        assert_eq!(file_change("s1", root, "a.txt").unwrap()["after"], "üç\n");
        revert("s1", root, "a.txt").unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "bir\n"
        );
        revert("s1", root, "new.txt").unwrap();
        assert!(!root.join("new.txt").exists());
        assert!(
            revert("s1", root, "a.txt").is_err(),
            "nothing left to revert"
        );
        assert!(revert("s1", root, "../x").is_err());
    }

    #[test]
    fn failed_or_unchanged_tools_emit_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("b.txt"), "x").unwrap();
        before_tool("s2", root, "u1", "Edit", "b.txt");
        assert!(after_tool("s2", root, "u1", false).is_none());
        before_tool("s2", root, "u2", "Edit", "b.txt");
        std::fs::write(root.join("b.txt"), "y").unwrap();
        assert!(after_tool("s2", root, "u2", true).is_none());
        before_tool("s2", root, "u3", "Read", "b.txt");
        assert!(after_tool("s2", root, "u3", false).is_none());
    }

    #[test]
    fn large_changes_send_stats_only() {
        let (a, r) = line_stats("a\nb\nc\n", "a\nc\nd\ne\n");
        assert_eq!((a, r), (2, 1));
    }
}
