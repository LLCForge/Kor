//! Two in-process peers over loopback TCP (ADR 0057): pairing, status, delegation with
//! approvals and the result diff, disconnect and reattach, handoff, memory merge, refusals.
//! Workers run a scripted agent in-process instead of a model.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use kor_agent::remote::wire::Command;
use kor_agent::{AgentEvent, Approver, PermissionReply, PermissionRequest};
use kor_providers::CancellationToken;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::client::{self, Workspace};
use super::noise::Link;
use super::{Book, Grant, Identity, Node, merge, pair, server, snapshot, standby};
use crate::remote::{TaskStore, TurnRunner, run_worker};

/// Says something, asks to write `note.txt`, writes it when allowed.
struct Scripted {
    root: PathBuf,
}

#[async_trait]
impl TurnRunner for Scripted {
    async fn turn(
        &mut self,
        text: String,
        events: &mpsc::UnboundedSender<AgentEvent>,
        approver: &dyn Approver,
        _cancel: CancellationToken,
    ) -> Result<Option<String>, String> {
        let _ = events.send(AgentEvent::Text(format!("working on {text}")));
        let req = PermissionRequest {
            id: "c1".into(),
            tool: "Write".into(),
            summary: "note.txt".into(),
            ..PermissionRequest::default()
        };
        if approver.review(&req).await.reply != PermissionReply::Deny {
            std::fs::write(self.root.join("note.txt"), "done by the peer\n")
                .map_err(|e| e.to_string())?;
        }
        Ok(None)
    }
}

fn node(dir: &Path, name: &str) -> Node {
    let mut b = Book::load(dir).unwrap();
    b.name = name.into();
    b.listen = "127.0.0.1:0".into();
    b.save(dir).unwrap();
    std::fs::create_dir_all(dir.join("home")).unwrap();
    let wdir = dir.to_path_buf();
    Node {
        dir: dir.to_path_buf(),
        identity: Identity::generate().unwrap(),
        sessions: dir.join("sessions"),
        knowledge_home: Some(dir.join("home")),
        spawn: Arc::new(move |peer: &str, id: &str| {
            let store = TaskStore {
                dir: wdir.join("tasks").join(peer),
            };
            let meta = server::read_meta(&store.task_dir(id)).ok_or("no meta")?;
            let id = id.to_owned();
            tokio::spawn(async move {
                let mut r = Scripted { root: meta.root };
                let _ = run_worker(&store, &id, &mut r).await;
            });
            Ok(())
        }),
        relay: None,
    }
}

async fn start(n: &Node) -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap().to_string();
    tokio::spawn(server::listen(Arc::new(n.clone()), l));
    addr
}

async fn until(link: &mut Link, ty: &str) -> Vec<Value> {
    let mut seen = Vec::new();
    loop {
        let v = link.recv_within(Duration::from_secs(20)).await.unwrap();
        let done = v["type"] == ty;
        seen.push(v);
        if done {
            return seen;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::too_many_lines, clippy::many_single_char_names)]
async fn two_peers_pair_delegate_hand_off_and_merge() {
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (a, b) = (node(da.path(), "server"), node(db.path(), "laptop"));
    let addr = start(&a).await;

    // --- pairing: a code with a wrong secret fails, the right one pairs both ways ---
    let code = pair::create(&a.dir, vec![addr.clone()]).unwrap();
    let mut wrong = code.clone();
    wrong.secret = "0".repeat(16);
    assert!(pair::join(&b, &wrong, None).await.is_err());
    assert!(
        !pair::is_pending(&a.dir, code.id),
        "a failed attempt uses the code up"
    );
    let code = pair::create(&a.dir, vec![addr.clone()]).unwrap();
    let srv = pair::join(&b, &code, None).await.unwrap();
    assert_eq!(srv.name, "server");
    assert_eq!(srv.key_bytes(), Some(a.identity.public));
    tokio::time::sleep(Duration::from_millis(300)).await;
    let a_book = Book::load(&a.dir).unwrap();
    let lap = a_book
        .get("laptop")
        .expect("server stored the laptop")
        .clone();
    assert_eq!(lap.grant, Grant::default(), "read-only by default");
    // The code's address is where the laptop reaches the server.
    let mut bb = Book::load(&b.dir).unwrap();
    bb.peers[0].addrs = vec![addr.clone()];
    bb.save(&b.dir).unwrap();
    let srv = bb.peers[0].clone();

    // --- status ---
    let load = client::status(&b, &srv).await.unwrap();
    assert_eq!(load.name, "server");
    assert!(load.cpus >= 1);

    // --- an unpaired machine is refused at the handshake ---
    let dc = tempfile::tempdir().unwrap();
    let c = node(dc.path(), "stranger");
    assert!(client::status(&c, &srv).await.is_err());

    // --- delegation with a snapshot: events and approvals stream back, the diff returns ---
    let proj = tempfile::tempdir().unwrap();
    std::fs::write(proj.path().join("main.rs"), "fn main() {}\n").unwrap();
    let ws = Workspace::Snapshot {
        bundle: snapshot::collect(proj.path()).unwrap(),
    };
    let mut link = client::run(&b, &srv, "add a note", &ws).await.unwrap();
    let seen = until(&mut link, "confirm").await;
    let task = seen
        .iter()
        .find(|v| v["type"] == "attached")
        .and_then(|v| v["task"].as_str())
        .unwrap()
        .to_owned();
    assert!(seen.iter().any(|v| v["type"] == "text"));
    // --- disconnect while the task waits: it keeps waiting on the server ---
    drop(link);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let st = a.store("laptop").state(&task).unwrap();
    assert_eq!(st.state, "waiting");
    // --- reattach, replay, approve ---
    let mut link = client::attach(&b, &srv, Some(&task), 0).await.unwrap();
    until(&mut link, "confirm").await;
    link.send(
        serde_json::to_value(Command::Approve {
            id: "c1".into(),
            approval: "yes".into(),
            files: None,
        })
        .unwrap(),
    )
    .unwrap();
    until(&mut link, "done").await;
    drop(link);
    let res = client::result(&b, &srv, &task).await.unwrap();
    assert_eq!(res.changes.len(), 1);
    assert!(res.diff.contains("+done by the peer"), "{}", res.diff);
    let applied = snapshot::apply(proj.path(), &res.changes, None);
    assert_eq!(applied[0].status, "written");
    assert!(proj.path().join("note.txt").is_file());

    // --- permission denial: a directory the grant does not list ---
    let peer_dir = tempfile::tempdir().unwrap();
    let mut link = client::run(
        &b,
        &srv,
        "look around",
        &Workspace::Dir {
            path: peer_dir.path().display().to_string(),
        },
    )
    .await
    .unwrap();
    let v = until(&mut link, "error").await;
    assert!(
        v.last().unwrap()["message"]
            .as_str()
            .unwrap()
            .contains("does not grant")
    );
    // Granted now: the same request starts a task in that directory.
    let mut ab = Book::load(&a.dir).unwrap();
    ab.peers
        .iter_mut()
        .find(|p| p.name == "laptop")
        .unwrap()
        .grant
        .dirs = vec![peer_dir.path().display().to_string()];
    ab.save(&a.dir).unwrap();
    let mut link = client::run(
        &b,
        &srv,
        "look around",
        &Workspace::Dir {
            path: peer_dir.path().display().to_string(),
        },
    )
    .await
    .unwrap();
    until(&mut link, "confirm").await;
    link.send(serde_json::to_value(Command::Abort).unwrap())
        .unwrap();
    until(&mut link, "done").await;
    drop(link);

    // --- handoff: a mirrored, unfinished session continues on the server after its lease ---
    std::fs::create_dir_all(&b.sessions).unwrap();
    let tpath = b.sessions.join("s-1.jsonl");
    kor_agent::session::save(
        &tpath,
        &[kor_core::Message::text(kor_core::Role::User, "refactor")],
    )
    .unwrap();
    let mut m = standby::Mirror {
        session: "s-1".into(),
        transcript: tpath,
        project: proj.path().to_path_buf(),
        sent_digest: None,
    };
    assert_eq!(
        standby::renew(&b, &srv, &mut m).await.unwrap(),
        standby::Renewed::Held
    );
    assert!(m.sent_digest.is_some());
    assert!(standby::scan(&a).is_empty(), "the lease is still valid");
    // The laptop closes: no renewal; the lease runs out.
    let lease_file = a.dir.join("standby/laptop/s-1/lease.json");
    let mut lease: Value = serde_json::from_slice(&std::fs::read(&lease_file).unwrap()).unwrap();
    lease["expires"] = json!(0);
    std::fs::write(&lease_file, lease.to_string()).unwrap();
    let ids = standby::scan(&a);
    assert_eq!(ids.len(), 1);
    assert!(
        a.sessions.join(format!("{}.jsonl", ids[0])).is_file(),
        "checkpoint resumed"
    );
    // The laptop wakes up: the renewal learns where the session went.
    assert_eq!(
        standby::renew(&b, &srv, &mut m).await.unwrap(),
        standby::Renewed::TakenOver(ids[0].clone())
    );

    // --- shared memory: three-way merge both ways ---
    let rec = |task: &str, summary: &str| kor_memory::Record {
        ts: 1,
        project: "p".into(),
        kind: "task".into(),
        task: task.into(),
        summary: summary.into(),
        ..kor_memory::Record::default()
    };
    let kb = |n: &Node| {
        kor_memory::Knowledge::open(&kor_memory::knowledge_path(
            n.knowledge_home.as_deref().unwrap(),
        ))
        .unwrap()
    };
    {
        let (ka, kbb) = (kb(&a), kb(&b));
        ka.record(rec("shared", "one\ntwo")).unwrap();
        ka.record(rec("server-only", "s")).unwrap();
        kbb.record(rec("shared", "one\ntwo")).unwrap();
        kbb.record(rec("laptop-only", "l")).unwrap();
        ka.flush().unwrap();
        kbb.flush().unwrap();
    }
    // Read-only by default: the laptop gets everything, the server's base is untouched.
    let r = client::sync(&b, &srv).await.unwrap();
    assert!(!r.remote_applied);
    assert_eq!(r.records, 3);
    assert_eq!(
        merge::local_records(a.knowledge_home.as_deref().unwrap())
            .unwrap()
            .len(),
        2
    );
    // With memory granted, both sides converge.
    let mut ab = Book::load(&a.dir).unwrap();
    ab.peers
        .iter_mut()
        .find(|p| p.name == "laptop")
        .unwrap()
        .grant
        .memory = true;
    ab.save(&a.dir).unwrap();
    let r = client::sync(&b, &srv).await.unwrap();
    assert!(r.remote_applied && r.conflicts.is_empty());
    let sa = merge::snapshot(merge::local_records(a.knowledge_home.as_deref().unwrap()).unwrap());
    let sb = merge::snapshot(merge::local_records(b.knowledge_home.as_deref().unwrap()).unwrap());
    assert_eq!(sa, sb);
    // Both edit the same line of the shared record: a conflict, both versions kept.
    for (n, s) in [(&a, "one\nSERVER"), (&b, "one\nLAPTOP")] {
        let k = kb(n);
        let id = k
            .recent(None, 10)
            .unwrap()
            .into_iter()
            .find(|r| r.task == "shared")
            .unwrap()
            .id;
        k.forget(id).unwrap();
        k.record(rec("shared", s)).unwrap();
        k.flush().unwrap();
    }
    let r = client::sync(&b, &srv).await.unwrap();
    assert_eq!(r.conflicts.len(), 1);
    let merged = merge::local_records(b.knowledge_home.as_deref().unwrap()).unwrap();
    let shared = merged.iter().find(|r| r.task == "shared").unwrap();
    assert!(shared.summary.contains("SERVER") && shared.summary.contains("LAPTOP"));
}

fn material(e: &kor_server::store::Enrollment) -> super::Relay {
    use kor_secrets::SecretString;
    super::Relay {
        server: e.server.clone(),
        material: Arc::new(kor_agent::team_client::Material {
            ca: Some(e.ca.clone()),
            cert: Some(e.cert.clone()),
            key: Some(SecretString::from(e.key.clone())),
            token: Some(SecretString::from(e.token.clone())),
        }),
    }
}

/// §31.1: two paired machines with no direct, Tailscale or SSH route reach each other through
/// the team server, which forwards only the Noise ciphertext.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn peers_without_a_route_meet_through_the_team_server() {
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (mut a, mut b) = (node(da.path(), "server"), node(db.path(), "laptop"));
    let addr = start(&a).await;
    let code = pair::create(&a.dir, vec![addr]).unwrap();
    pair::join(&b, &code, None).await.unwrap();
    // Forget every direct route.
    let mut bb = Book::load(&b.dir).unwrap();
    bb.peers[0].addrs.clear();
    bb.save(&b.dir).unwrap();
    let srv = bb.peers[0].clone();
    assert!(
        client::status(&b, &srv).await.is_err(),
        "no route without the relay"
    );

    // An in-process team server; each machine joined as its own member.
    let sd = tempfile::tempdir().unwrap();
    let (_, admin) = kor_server::init(sd.path(), "acme", "localhost", "home@example.com", None)
        .await
        .unwrap();
    let app = kor_server::http::App::open(sd.path()).await.unwrap();
    let cfg = app.cfg.clone();
    let lay = app.layout.clone();
    let laptop = app
        .store
        .add_member(
            &cfg,
            &lay,
            "laptop@example.com",
            kor_server::Role::Member,
            "home@example.com",
        )
        .await
        .unwrap();
    let running = kor_server::http::start(app, "127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let url = format!("https://localhost:{}", running.addr.port());
    let mut ra = material(&admin);
    ra.server.clone_from(&url);
    let mut rb = material(&laptop);
    rb.server.clone_from(&url);
    a.relay = Some(ra.clone());
    b.relay = Some(rb);
    tokio::spawn(super::relay::serve(
        Arc::new(a.clone()),
        ra.server,
        ra.material,
    ));
    tokio::time::sleep(Duration::from_millis(500)).await;

    let load = client::status(&b, &srv).await.unwrap();
    assert_eq!(load.name, "server");
    running.stop();
}
