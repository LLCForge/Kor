use std::path::{Path, PathBuf};
use std::sync::Mutex;

use async_trait::async_trait;
use kor_core::{Message, Role};

use super::*;

fn layout(d: &Path) -> Layout {
    Layout {
        root: d.join("home").join("Kor"),
        home: d.join("data"),
    }
}

fn session(l: &Layout, id: &str, text: &str) {
    let msgs = vec![
        Message::text(Role::User, text),
        Message::text(Role::Assistant, "tamam"),
    ];
    kor_agent::session::save(&l.home.join("sessions").join(format!("{id}.jsonl")), &msgs).unwrap();
}

#[test]
fn enable_creates_views_and_links_and_disable_removes_only_kors_files() {
    let d = tempfile::tempdir().unwrap();
    let l = layout(d.path());
    let proj = d.path().join("proj");
    std::fs::create_dir_all(proj.join(".kor/sessions/abc")).unwrap();
    std::fs::create_dir_all(proj.join("kor-media")).unwrap();
    std::fs::write(proj.join("kor-media/kedi.png"), b"png").unwrap();
    std::fs::create_dir_all(proj.join(".kor/perf/r1")).unwrap();
    session(&l, "20260101-000000-1", "auth sistemini düzelt");

    assert!(!status(&l).enabled);
    let r = enable(&l, Some(&proj)).unwrap();
    assert!(status(&l).enabled);
    let view = l.dir(SESSIONS).join("20260101-000000-1.md");
    let body = std::fs::read_to_string(&view).unwrap();
    assert!(
        body.contains("title: \"auth sistemini düzelt\"") && body.contains("tamam"),
        "{body}"
    );
    assert!(r.written.contains(&view));
    assert!(
        l.dir(RECORDS).join("proj").join("abc").is_dir(),
        "recordings linked"
    );
    assert!(
        l.dir(MEDIA).join("proj").join("kedi.png").is_file(),
        "media linked"
    );
    assert!(
        l.dir(REPORTS).join("proj").join("perf").join("r1").is_dir(),
        "reports linked"
    );
    assert!(
        std::fs::read_to_string(l.dir(MEDIA).join("Galeri.md"))
            .unwrap()
            .contains("kedi.png")
    );
    // Idempotent.
    let again = enable(&l, Some(&proj)).unwrap();
    assert!(
        again.written.is_empty() && again.linked.is_empty(),
        "{again:?}"
    );

    // A user file in Gelen and a user-edited view survive removal.
    std::fs::write(l.dir(INBOX).join("not.txt"), "benim").unwrap();
    std::fs::write(l.dir(SESSIONS).join("kendi.md"), "benim").unwrap();
    let rm = disable(&l).unwrap();
    assert!(!l.enabled());
    assert!(l.dir(INBOX).join("not.txt").is_file() && l.dir(SESSIONS).join("kendi.md").is_file());
    assert!(!view.exists(), "unchanged view removed");
    assert!(!l.dir(RECORDS).exists(), "empty folders removed");
    assert!(
        proj.join(".kor/sessions/abc").is_dir() && proj.join("kor-media/kedi.png").is_file(),
        "targets kept"
    );
    assert!(rm.kept.iter().any(|p| p.ends_with("not.txt")));
    // Nothing to do the second time.
    assert_eq!(disable(&l).unwrap(), RemoveReport::default());
}

#[test]
fn edited_front_matter_flows_back_and_the_file_wins() {
    let d = tempfile::tempdir().unwrap();
    let l = layout(d.path());
    let id = "20260102-000000-2";
    session(&l, id, "ilk mesaj");
    std::fs::create_dir_all(&l.home).unwrap();
    std::fs::write(
        l.home.join("desktop.json"),
        r#"{"version":1,"sessions":{},"theme":"dark"}"#,
    )
    .unwrap();
    enable(&l, None).unwrap();
    let view = l.dir(SESSIONS).join(format!("{id}.md"));
    let text = std::fs::read_to_string(&view).unwrap();
    let edited = text
        .replace("title: \"ilk mesaj\"", "title: \"Yeni başlık\"")
        .replace("pinned: false", "pinned: true")
        + "\nkendi notum\n";
    std::fs::write(&view, &edited).unwrap();
    // The transcript changes too: the file still wins this round.
    session(&l, id, "ilk mesaj (değişti)");
    let r = sync(&l).unwrap();
    assert_eq!(r.applied, vec![view.clone()]);
    assert_eq!(std::fs::read_to_string(&view).unwrap(), edited);
    let desk: serde_json::Value =
        serde_json::from_slice(&std::fs::read(l.home.join("desktop.json")).unwrap()).unwrap();
    assert_eq!(desk["sessions"][id]["customTitle"], "Yeni başlık");
    assert_eq!(desk["sessions"][id]["pinned"], true);
    assert_eq!(desk["theme"], "dark", "other settings kept");
    // Next round regenerates from the store (now carrying the edited title).
    sync(&l).unwrap();
    let regenerated = std::fs::read_to_string(&view).unwrap();
    assert!(regenerated.contains("Yeni başlık") && regenerated.contains("pinned: true"));
}

#[test]
fn damaged_desktop_json_is_never_overwritten() {
    let d = tempfile::tempdir().unwrap();
    let l = layout(d.path());
    std::fs::create_dir_all(&l.home).unwrap();
    std::fs::write(l.home.join("desktop.json"), "{bozuk").unwrap();
    let f = views::Front {
        title: Some("x".into()),
        pinned: false,
        archived: false,
    };
    assert!(!views::apply_front(&l, "a", &f).unwrap());
    assert_eq!(
        std::fs::read_to_string(l.home.join("desktop.json")).unwrap(),
        "{bozuk"
    );
}

#[test]
fn diffs_show_undo_journals() {
    let d = tempfile::tempdir().unwrap();
    let l = layout(d.path());
    let proj = d.path().join("p");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(proj.join("a.txt"), "eski\n").unwrap();
    let mut j =
        kor_tools::undo::Journal::open(&l.home.join("undo"), "s1", std::slice::from_ref(&proj))
            .unwrap();
    j.record(&proj, "a.txt").unwrap();
    std::fs::write(proj.join("a.txt"), "yeni\n").unwrap();
    enable(&l, None).unwrap();
    let diff = std::fs::read_to_string(l.dir(DIFFS).join("s1.diff")).unwrap();
    assert!(diff.contains("eski") && diff.contains("yeni"), "{diff}");
    // Undone journals disappear from the view.
    kor_tools::undo::undo(&l.home.join("undo").join("s1")).unwrap();
    sync(&l).unwrap();
    assert!(!l.dir(DIFFS).join("s1.diff").exists());
}

#[test]
fn inbox_files_move_once_and_become_context() {
    let d = tempfile::tempdir().unwrap();
    let l = layout(d.path());
    assert!(take_inbox(&l).is_empty(), "off: nothing");
    enable(&l, None).unwrap();
    std::fs::write(l.dir(INBOX).join("not.md"), "önemli bağlam").unwrap();
    std::fs::write(l.dir(INBOX).join("indiriliyor.crdownload"), "x").unwrap();
    std::fs::write(l.dir(INBOX).join(".gizli"), "x").unwrap();
    std::fs::create_dir_all(l.dir(INBOX).join(INBOX_DONE)).unwrap();
    std::fs::write(l.dir(INBOX).join(INBOX_DONE).join("not.md"), "eski").unwrap();
    assert_eq!(status(&l).inbox, 1);
    let taken = take_inbox(&l);
    assert_eq!(
        taken,
        vec![l.dir(INBOX).join(INBOX_DONE).join("not (2).md")],
        "numbered, never overwritten"
    );
    let block = inbox_block(&taken);
    assert!(block.contains("önemli bağlam"));
    assert!(take_inbox(&l).is_empty());
    assert!(l.dir(INBOX).join("indiriliyor.crdownload").is_file());
}

struct Scripted {
    seen: Mutex<Vec<FileTask>>,
}

#[async_trait]
impl TaskRunner for Scripted {
    async fn run(&self, task: &FileTask) -> Result<String, String> {
        self.seen.lock().unwrap().push(task.clone());
        if task.prompt.contains("patla") {
            Err("model hatası".into())
        } else {
            Ok(format!("yanıt: {}", task.prompt))
        }
    }
}

#[tokio::test]
async fn dropped_task_files_run_once_in_safe_mode() {
    let d = tempfile::tempdir().unwrap();
    let l = layout(d.path());
    enable(&l, None).unwrap();
    let proj = d.path().join("proje");
    std::fs::create_dir_all(&proj).unwrap();
    let t = l.dir(TASKS);
    std::fs::write(
        t.join("özet.md"),
        format!(
            "---\nmodel: ollama/qwen3\nproject: {}\n---\nREADME'yi özetle\n",
            proj.display().to_string().replace('\\', "/")
        ),
    )
    .unwrap();
    std::fs::write(t.join("hata.md"), "patla").unwrap();
    std::fs::write(t.join("tehlike.md"), "---\nmode: auto\n---\nher şeyi sil\n").unwrap();
    std::fs::write(t.join("notlar.txt"), "görev değil").unwrap();
    assert_eq!(tasks_pending(&l).len(), 3);
    let runner = Scripted {
        seen: Mutex::new(Vec::new()),
    };
    let out = run_pending(&l, &runner).await;
    assert_eq!(out.len(), 3);
    let seen = runner.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2, "the auto-mode task never ran");
    let ozet = seen.iter().find(|s| s.name == "özet").unwrap();
    assert_eq!(ozet.mode, kor_core::PermissionMode::Plan);
    assert_eq!(ozet.model.as_deref(), Some("ollama/qwen3"));
    assert_eq!(
        ozet.root,
        PathBuf::from(proj.display().to_string().replace('\\', "/"))
    );
    assert_eq!(
        std::fs::read_to_string(t.join("özet.sonuç.md")).unwrap(),
        "yanıt: README'yi özetle"
    );
    assert!(
        std::fs::read_to_string(t.join("tehlike.sonuç.md"))
            .unwrap()
            .contains("auto")
    );
    assert!(
        std::fs::read_to_string(t.join("hata.sonuç.md"))
            .unwrap()
            .contains("model hatası")
    );
    assert!(t.join(TASKS_DONE).join("özet.md").is_file());
    assert!(t.join("notlar.txt").is_file());
    // Results are not tasks; nothing runs twice.
    assert!(tasks_pending(&l).is_empty());
    assert!(run_pending(&l, &runner).await.is_empty());
}

#[test]
fn task_front_matter_rules() {
    let root = Path::new("/r");
    let p = Path::new("/r/x.md");
    assert!(tasks::parse(p, "x", "", root).is_err());
    let t = tasks::parse(p, "x", "---\nmode: default\n---\nselam", root).unwrap();
    assert_eq!(
        (t.mode, t.prompt.as_str(), t.root.as_path()),
        (kor_core::PermissionMode::Default, "selam", root)
    );
    assert!(tasks::parse(p, "x", "---\nproject: göreli/yol\n---\nx", root).is_err());
    assert!(tasks::parse(p, "x", "---\nmode: accept-edits\n---\nx", root).is_err());
    // No front matter: the whole text is the prompt.
    assert_eq!(
        tasks::parse(p, "x", "--- değil\nmetin", root)
            .unwrap()
            .prompt,
        "--- değil\nmetin"
    );
}

#[test]
fn dav_paths_are_confined_and_writes_only_in_drop_boxes() {
    let d = tempfile::tempdir().unwrap();
    let l = layout(d.path());
    enable(&l, None).unwrap();
    let tree = dav::Tree {
        root: l.root.clone(),
        token: "t0k".into(),
    };
    assert_eq!(tree.resolve("/yanlis/Gelen"), Err(dav::Refused::NotFound));
    assert_eq!(tree.resolve("/t0k/../x"), Err(dav::Refused::Forbidden));
    assert_eq!(tree.resolve("/t0k/%2e%2e/x"), Err(dav::Refused::Forbidden));
    assert_eq!(tree.resolve("/t0k/C:/x"), Err(dav::Refused::Forbidden));
    assert_eq!(tree.resolve("/t0kx/a"), Err(dav::Refused::NotFound));
    let (p, segs) = tree.resolve("/t0k/G%C3%B6revler/a.md").unwrap();
    assert_eq!(p, l.dir(TASKS).join("a.md"));
    assert!(dav::Tree::writable(&segs));
    assert!(!dav::Tree::writable(
        &tree.resolve("/t0k/Oturumlar/a.md").unwrap().1
    ));
    assert!(
        !dav::Tree::writable(&tree.resolve("/t0k/Gelen").unwrap().1),
        "not the folder itself"
    );
    let (root, rs) = tree.resolve("/t0k/").unwrap();
    let xml = tree.propfind(&root, &rs, 1).unwrap();
    assert!(
        xml.contains("<D:href>/t0k/G%C3%B6revler/</D:href>") && xml.contains("<D:collection/>"),
        "{xml}"
    );
    assert!(
        tree.propfind(&l.root.join("yok"), &["yok".into()], 0)
            .is_none()
    );
}

#[tokio::test]
async fn dav_server_lists_reads_and_takes_drops_only() {
    let d = tempfile::tempdir().unwrap();
    let l = layout(d.path());
    enable(&l, None).unwrap();
    std::fs::write(l.dir(SESSIONS).join("a.md"), "merhaba").unwrap();
    let server = dav::serve(l.root.clone()).await.unwrap();
    let url = server.url.clone();
    let host = url
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap()
        .to_owned();
    let addr = host.clone();
    let req = |method: &str, path: &str, body: &str, host: &str| {
        let (method, path, body, host, addr) = (
            method.to_owned(),
            path.to_owned(),
            body.to_owned(),
            host.to_owned(),
            addr.clone(),
        );
        async move {
            use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
            let mut s = tokio::net::TcpStream::connect(&addr).await.unwrap();
            let msg = format!(
                "{method} {path} HTTP/1.1\r\nHost: {host}\r\nDepth: 1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            s.write_all(msg.as_bytes()).await.unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).await.unwrap();
            out
        }
    };
    let base = url
        .trim_start_matches("http://")
        .splitn(2, '/')
        .nth(1)
        .map(|p| format!("/{p}"))
        .unwrap();
    let r = req("PROPFIND", &base, "", &host).await;
    assert!(
        r.starts_with("HTTP/1.1 207") && r.contains("Oturumlar"),
        "{r}"
    );
    let r = req("GET", &format!("{base}Oturumlar/a.md"), "", &host).await;
    assert!(r.ends_with("merhaba"), "{r}");
    let r = req("PUT", &format!("{base}Gelen/yeni.txt"), "veri", &host).await;
    assert!(r.starts_with("HTTP/1.1 201"), "{r}");
    assert_eq!(
        std::fs::read_to_string(l.dir(INBOX).join("yeni.txt")).unwrap(),
        "veri"
    );
    let r = req("PUT", &format!("{base}Oturumlar/b.md"), "x", &host).await;
    assert!(r.starts_with("HTTP/1.1 403"), "{r}");
    let r = req("DELETE", &format!("{base}Gelen/yeni.txt"), "", &host).await;
    assert!(r.starts_with("HTTP/1.1 403"), "{r}");
    let r = req("PUT", &format!("{base}Gelen/yeni.txt"), "üzerine", &host).await;
    assert!(r.starts_with("HTTP/1.1 403"), "no overwrite: {r}");
    // DNS rebinding: a foreign Host header is refused.
    let port = host.rsplit(':').next().unwrap();
    let r = req(
        "GET",
        &format!("{base}Oturumlar/a.md"),
        "",
        &format!("evil.example:{port}"),
    )
    .await;
    assert!(r.starts_with("HTTP/1.1 403"), "{r}");
    drop(server);
}

#[test]
fn unique_paths_number_clashes() {
    let d = tempfile::tempdir().unwrap();
    let a = d.path().join("a.txt");
    assert_eq!(crate::batch::unique_path(&a, &[]), a);
    std::fs::write(&a, "").unwrap();
    assert_eq!(
        crate::batch::unique_path(&a, &[]),
        d.path().join("a (2).txt")
    );
    assert_eq!(
        crate::batch::unique_path(&a, &[d.path().join("a (2).txt")]),
        d.path().join("a (3).txt")
    );
}
