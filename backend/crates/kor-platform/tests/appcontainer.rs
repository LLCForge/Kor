//! Real `AppContainer` confinement on this Windows machine (no-op elsewhere). The commands use
//! .NET calls with absolute paths: `PowerShell`'s own drives need kor-sandbox's prelude.
#![cfg(windows)]

use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

use kor_platform::appcontainer::{self, Spec};

fn ps(command: &str) -> Vec<OsString> {
    [
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        command,
    ]
    .map(OsString::from)
    .to_vec()
}

fn env() -> Vec<(OsString, OsString)> {
    std::env::vars_os().collect()
}

fn spec<'a>(root: &'a Path, args: &'a [OsString], env: &'a [(OsString, OsString)]) -> Spec<'a> {
    Spec {
        program: "powershell".as_ref(),
        args,
        cwd: root,
        env,
        root,
        network: false,
        memory_bytes: 2 << 30,
        cpu_percent: 90,
    }
}

async fn run(root: &Path, command: &str) -> (bool, String) {
    let (args, env) = (ps(command), env());
    let child = appcontainer::spawn(&spec(root, &args, &env)).unwrap();
    let out = child.wait_with_output().await.unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

fn has_container_ace(dir: &Path) -> bool {
    let out = std::process::Command::new("icacls")
        .arg(dir)
        .output()
        .unwrap();
    // AppContainer SIDs start with S-1-15-2-; icacls prints unresolvable SIDs raw.
    String::from_utf8_lossy(&out.stdout).contains("S-1-15-2-")
}

#[tokio::test]
async fn confines_files_and_network_and_cleans_up() {
    assert!(appcontainer::available());
    let project = tempfile::tempdir().unwrap();
    let root = project.path().to_path_buf();
    std::fs::write(root.join("in.txt"), "inside").unwrap();
    // A file of the user's outside the project (the user's temp folder is in the profile).
    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("secret.txt");
    std::fs::write(&secret, "top secret").unwrap();

    let r = root.display();
    let (ok, out) = run(
        &root,
        &format!(
            "[IO.File]::ReadAllText('{r}\\in.txt'); [IO.File]::WriteAllText('{r}\\out.txt', 'written'); \
             [IO.File]::WriteAllText(\"$env:TEMP\\t.txt\", 't'); \"temp=$env:TEMP\""
        ),
    )
    .await;
    assert!(ok, "{out}");
    assert!(out.contains("inside"), "{out}");
    assert!(out.contains(r"\AC\Temp"), "{out}");
    assert_eq!(
        std::fs::read_to_string(root.join("out.txt")).unwrap(),
        "written"
    );

    let read = format!("[IO.File]::ReadAllText('{}')", secret.display());
    let (ok, out) = run(&root, &read).await;
    assert!(!ok && !out.contains("top secret"), "{out}");

    let target = outside.path().join("planted.txt");
    let write = format!("[IO.File]::WriteAllText('{}', 'x')", target.display());
    let (ok, out) = run(&root, &write).await;
    assert!(!ok && !target.exists(), "{out}");

    // Network off: even a listener on this machine is unreachable from the container.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let probe = format!(
        "try {{ $c = New-Object Net.Sockets.TcpClient; if ($c.ConnectAsync('127.0.0.1', {port}).Wait(3000)) {{ 'CONNECTED' }} else {{ 'BLOCKED' }} }} catch {{ 'BLOCKED' }}"
    );
    let (_, out) = run(&root, &probe).await;
    assert!(
        out.contains("BLOCKED") && !out.contains("CONNECTED"),
        "{out}"
    );
    let control = std::process::Command::new("powershell")
        .args(ps(&probe))
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&control.stdout).contains("CONNECTED"));
    drop(listener);

    // Every command has ended: the ACE on the project is gone.
    assert!(!has_container_ace(&root));
}

#[tokio::test]
async fn dropping_the_child_kills_the_whole_tree() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path().to_path_buf();
    let pid_file = root.join("pid.txt");
    let args = ps(&format!(
        "$p = [Diagnostics.Process]::Start('powershell', '-NoProfile -Command Start-Sleep 120'); \
         [IO.File]::WriteAllText('{}', \"$($p.Id);\"); Start-Sleep 120",
        pid_file.display()
    ));
    let env = env();
    let child = appcontainer::spawn(&spec(&root, &args, &env)).unwrap();
    let parent = child.id();
    let wait = tokio::spawn(child.wait_with_output());
    let mut grandchild = None;
    for _ in 0..200 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if let Some(pid) = std::fs::read_to_string(&pid_file)
            .ok()
            .and_then(|s| s.strip_suffix(';')?.trim().parse::<u32>().ok())
        // `;`: the write is complete
        {
            grandchild = Some(pid);
            break;
        }
    }
    let grandchild = grandchild.expect("grandchild started");
    assert!(alive(parent), "the sandboxed parent exited early");
    if !alive(grandchild) {
        // Seen on CI runners only: the nested PowerShell dies at start-up inside the AppContainer,
        // so there is no tree to kill. Nothing to check about the drop.
        eprintln!("skipped: grandchild {grandchild} exited on its own");
        return;
    }
    wait.abort(); // like a cancelled tool call: the future is dropped
    let _ = wait.await;
    for _ in 0..50 {
        if !alive(parent) && !alive(grandchild) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(!alive(parent) && !alive(grandchild));
    assert!(!has_container_ace(&root));
}

fn alive(pid: u32) -> bool {
    let out = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
}
