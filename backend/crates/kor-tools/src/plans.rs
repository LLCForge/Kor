//! Plans of the "stop and research" lock (Master Build Prompt §28.2, ADR 0050).
//!
//! A plan is `.kor/plans/<YYYY-MM-DD>-<slug>.md`: a small front matter (`title`, `status`,
//! `session`, `created`) and the Markdown body the model wrote. Only a user action approves one
//! (`kor plan approve`, `/go`, the desktop button); the model can only save drafts.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Per-session state of plan mode, shared by the tools and the agent loop.
#[derive(Debug, Default)]
pub struct Gate {
    /// `[plan] lock`: write tools stay off until a plan of this session is approved.
    pub lock: bool,
    /// `[auto] plan_intent`: "plan first / do not touch" in a message switches to plan mode.
    pub intent: bool,
    /// Session id plans are saved under (the transcript's file stem), if the session is saved.
    pub session: Option<String>,
    /// Unix seconds plan mode was last entered: only plans written since then can unlock it.
    pub since: AtomicU64,
    /// Files named by the approved plan; a write to any other file asks again.
    pub approved: Mutex<Option<Vec<String>>>,
}

impl Gate {
    /// Plan mode starts (again): earlier approvals no longer count.
    pub fn enter(&self) {
        self.since.store(now(), Ordering::Relaxed);
        *self
            .approved
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }

    /// The approved plan of this session, written since plan mode was entered.
    pub fn approved_plan(&self, root: &Path) -> Option<Plan> {
        let since = self.since.load(Ordering::Relaxed);
        list(root).into_iter().find(|p| {
            p.status == Status::Approved
                && p.created >= since
                && match (&self.session, &p.session) {
                    (Some(s), Some(ps)) => s == ps,
                    _ => true,
                }
        })
    }

    /// Remember the approved plan's files (once) and say whether a plan is approved.
    pub fn refresh(&self, root: &Path) -> bool {
        let mut a = self
            .approved
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if a.is_some() {
            return true;
        }
        match self.approved_plan(root) {
            Some(p) => {
                *a = Some(files_in(&p.body));
                true
            }
            None => false,
        }
    }

    /// After approval: `path` (project-relative) is not named by the plan, so it asks again.
    pub fn outside_plan(&self, path: &str) -> bool {
        let a = self
            .approved
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let norm = |s: &str| s.replace('\\', "/").trim_start_matches("./").to_owned();
        let path = norm(path);
        a.as_ref().is_some_and(|files| {
            !files
                .iter()
                .any(|f| path == norm(f) || path.ends_with(&format!("/{}", norm(f))))
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Draft,
    Approved,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Approved => "approved",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// File stem: `2026-11-10-add-login`.
    pub id: String,
    pub path: PathBuf,
    pub title: String,
    pub status: Status,
    pub session: Option<String>,
    /// Unix seconds.
    pub created: u64,
    pub body: String,
}

pub fn dir(root: &Path) -> PathBuf {
    root.join(".kor").join("plans")
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `YYYY-MM-DD` (UTC) of Unix seconds (Howard Hinnant's civil-from-days).
pub fn date(secs: u64) -> String {
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// ASCII slug of a title (Turkish and French letters folded), at most 48 characters.
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars().flat_map(char::to_lowercase) {
        let c = match c {
            'ç' => 'c',
            'ğ' => 'g',
            'ı' | 'î' | 'ï' => 'i',
            'ö' | 'ô' => 'o',
            'ş' => 's',
            'ü' | 'û' | 'ù' => 'u',
            'â' | 'à' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            c => c,
        };
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= 48 {
            break;
        }
    }
    let s = out.trim_end_matches('-').to_owned();
    if s.is_empty() { "plan".into() } else { s }
}

fn render(p: &Plan) -> String {
    let one_line = |s: &str| s.replace(['\r', '\n'], " ");
    format!(
        "---\ntitle: {}\nstatus: {}\nsession: {}\ncreated: {}\n---\n\n{}\n",
        one_line(&p.title),
        p.status.as_str(),
        p.session.as_deref().map(one_line).unwrap_or_default(),
        p.created,
        p.body.trim_end()
    )
}

fn parse(path: &Path, text: &str) -> Option<Plan> {
    let text = text.replace("\r\n", "\n");
    let rest = text.strip_prefix("---\n")?;
    let (head, body) = rest.split_once("\n---\n")?;
    let mut plan = Plan {
        id: path.file_stem()?.to_string_lossy().into_owned(),
        path: path.to_owned(),
        title: String::new(),
        status: Status::Draft,
        session: None,
        created: 0,
        body: body.trim().to_owned(),
    };
    for line in head.lines() {
        let Some((k, v)) = line.split_once(':') else { continue };
        let v = v.trim();
        match k.trim() {
            "title" => v.clone_into(&mut plan.title),
            "status" if v == "approved" => plan.status = Status::Approved,
            "session" if !v.is_empty() => plan.session = Some(v.to_owned()),
            "created" => plan.created = v.parse().unwrap_or(0),
            _ => {}
        }
    }
    Some(plan)
}

/// Save a new draft plan. The file name gets `-2`, `-3`… when one exists for the same day/slug.
///
/// # Errors
/// If `.kor/plans` cannot be created or written.
pub fn save(root: &Path, title: &str, body: &str, session: Option<&str>) -> io::Result<Plan> {
    let d = dir(root);
    std::fs::create_dir_all(&d)?;
    let created = now();
    let title = if title.trim().is_empty() {
        body.lines()
            .map(|l| l.trim_start_matches('#').trim())
            .find(|l| !l.is_empty())
            .unwrap_or("plan")
            .chars()
            .take(80)
            .collect()
    } else {
        title.trim().to_owned()
    };
    let base = format!("{}-{}", date(created), slug(&title));
    let mut id = base.clone();
    let mut n = 2;
    while d.join(format!("{id}.md")).exists() {
        id = format!("{base}-{n}");
        n += 1;
    }
    let plan = Plan {
        path: d.join(format!("{id}.md")),
        id,
        title,
        status: Status::Draft,
        session: session.map(str::to_owned),
        created,
        body: body.to_owned(),
    };
    std::fs::write(&plan.path, render(&plan))?;
    Ok(plan)
}

/// Every plan of the project, newest first.
pub fn list(root: &Path) -> Vec<Plan> {
    let mut out: Vec<Plan> = std::fs::read_dir(dir(root))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .filter_map(|p| parse(&p, &std::fs::read_to_string(&p).ok()?))
        .collect();
    out.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| b.id.cmp(&a.id)));
    out
}

/// A plan by id, or by a unique id prefix; `None` also when the prefix is ambiguous.
pub fn find(root: &Path, id: &str) -> Option<Plan> {
    let all = list(root);
    if let Some(p) = all.iter().find(|p| p.id == id) {
        return Some(p.clone());
    }
    let mut hits = all.into_iter().filter(|p| p.id.starts_with(id));
    let first = hits.next()?;
    hits.next().is_none().then_some(first)
}

/// Mark a plan approved (a user action only).
///
/// # Errors
/// `NotFound` for an unknown id, or the write error.
pub fn approve(root: &Path, id: &str) -> io::Result<Plan> {
    let mut p = find(root, id).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, id.to_owned()))?;
    p.status = Status::Approved;
    std::fs::write(&p.path, render(&p))?;
    Ok(p)
}

/// The newest draft of a session (what `/go` approves).
pub fn latest_draft(root: &Path, session: Option<&str>) -> Option<Plan> {
    list(root)
        .into_iter()
        .find(|p| p.status == Status::Draft && (session.is_none() || p.session.as_deref() == session))
}

/// Project paths a plan names: words with a `/` or a file extension, without backticks.
pub fn files_in(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for w in body.split(|c: char| c.is_whitespace() || matches!(c, '`' | '(' | ')' | ',' | ';' | '"' | '\'')) {
        let w = w.trim_end_matches([':', '.']).trim_start_matches("./");
        let ext = Path::new(w)
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| (1..=6).contains(&e.len()) && e.chars().all(|c| c.is_ascii_alphanumeric()));
        let looks = (w.contains('/') && !w.starts_with("http")) || ext;
        if looks && w.len() < 200 && !w.chars().all(|c| c.is_ascii_digit() || c == '.') && !out.iter().any(|o| o == w) {
            out.push(w.to_owned());
        }
    }
    out
}

/// "Research first / plan / ask before / do not touch" in Turkish, English or French.
pub fn wants_plan(text: &str) -> bool {
    const PHRASES: &[&str] = &[
        // tr
        "önce araştır",
        "önce plan",
        "plan yap",
        "plan çıkar",
        "sonra sor",
        "dokunma",
        "değiştirme",
        "hiçbir şeyi değiştirme",
        "kod yazma",
        // en
        "research first",
        "plan first",
        "make a plan",
        "write a plan",
        "don't touch",
        "do not touch",
        "don't change anything",
        "do not change anything",
        "don't write",
        "do not write",
        "ask before",
        // fr
        "fais un plan",
        "d'abord un plan",
        "recherche d'abord",
        "ne touche pas",
        "ne modifie rien",
        "demande avant",
    ];
    let t = text.to_lowercase().replace('’', "'");
    t.trim() == "/plan" || PHRASES.iter().any(|p| t.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_list_find_approve_and_gate() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let p = save(root, "Giriş sayfası ekle", "1. `src/login.rs` yaz\n2. update docs/auth.md", Some("s1")).unwrap();
        assert!(p.id.ends_with("-giris-sayfasi-ekle"), "{}", p.id);
        assert!(p.path.starts_with(root.join(".kor/plans")));
        let q = save(root, "Giriş sayfası ekle", "again", Some("s1")).unwrap();
        assert!(q.id.ends_with("-2"));
        assert_eq!(list(root).len(), 2);
        assert!(find(root, "20").is_none(), "ambiguous prefix");
        assert_eq!(latest_draft(root, Some("s1")).unwrap().id, q.id);

        let gate = Gate { lock: true, session: Some("s1".into()), ..Gate::default() };
        assert!(!gate.refresh(root));
        assert!(!gate.outside_plan("anything.rs"), "nothing approved yet");
        approve(root, &p.id).unwrap();
        assert_eq!(find(root, &p.id).unwrap().status, Status::Approved);
        assert!(gate.refresh(root));
        assert!(!gate.outside_plan("src/login.rs"));
        assert!(!gate.outside_plan("./docs/auth.md"));
        assert!(gate.outside_plan("src/other.rs"));
        let other = Gate { lock: true, session: Some("s2".into()), ..Gate::default() };
        assert!(!other.refresh(root), "another session's plan does not unlock");
        let later = Gate { lock: true, session: Some("s1".into()), ..Gate::default() };
        later.since.store(u64::MAX, Ordering::Relaxed);
        assert!(!later.refresh(root), "an approval from before plan mode was entered does not count");
        assert!(approve(root, "missing").is_err());
    }

    #[test]
    fn intent_slug_and_date() {
        assert!(wants_plan("Önce araştır, sonra sor"));
        assert!(wants_plan("Please make a plan, don't touch the code"));
        assert!(wants_plan("Fais un plan d'abord"));
        assert!(wants_plan("/plan"));
        assert!(!wants_plan("fix the login bug"));
        assert_eq!(slug("Çok Önemli: Ödeme!"), "cok-onemli-odeme");
        assert_eq!(slug("!!!"), "plan");
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(1_731_196_800), "2024-11-10");
        assert_eq!(files_in("edit `a/b.rs` and README.md, see https://x.y/z"), ["a/b.rs", "README.md"]);
    }
}
