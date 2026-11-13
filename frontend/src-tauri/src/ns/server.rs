//! `server.*`: `kor server init|add-member|status|start` için ince köprü (§11.1, parity.toml).
//! Takım sunucusu masaüstüne gömülmez; masaüstüyle gelen `kor` CLI'ı `--json` ile çalıştırılır,
//! böylece iki yüzey aynı kodu kullanır. Üye tarafı (`join`, `login`) `team.*` içindedir.
//!
//! - `server.init(domain, admin, team?)` → `{ dir, domain, listen, admin, bundle }`
//! - `server.addMember(email, role?, remove?)` → `{ email, role, bundle, expires }` | `{ removed }`
//! - `server.status()` → `{ server, member }`
//! - `server.start()` → `{ started: true }` (arka planda)

use kor_platform::NoWindow as _;
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, b, opt_s, s};
use crate::state::AppState;

pub const NS: &str = "server";

/// `kor <args> --json`, ilk satırı JSON olarak; hata olursa stderr.
async fn run_json(args: Vec<String>) -> Res {
    let exe = crate::kor_cli::find().ok_or_else(|| crate::kor_cli::MISSING.to_owned())?;
    let out = tokio::task::spawn_blocking(move || {
        std::process::Command::new(exe)
            .args(&args)
            .no_window()
            .stdin(std::process::Stdio::null())
            .output()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        return Err(if err.is_empty() {
            "kor server başarısız oldu".into()
        } else {
            err
        });
    }
    stdout
        .lines()
        .find_map(|l| serde_json::from_str::<Value>(l).ok())
        .ok_or_else(|| "kor server JSON yanıtı vermedi".into())
}

fn arg(v: &str, max: usize) -> Result<String, String> {
    let v = v.trim();
    if v.is_empty() || v.len() > max || v.starts_with('-') || v.chars().any(char::is_control) {
        return Err("Geçersiz değer".into());
    }
    Ok(v.to_owned())
}

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    _app: &AppHandle,
    _state: &AppState,
) -> Option<Res> {
    let a = |v: &[&str]| v.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
    Some(match name {
        "init" => {
            let (Ok(domain), Ok(admin)) = (s(args, 0, "server.init"), s(args, 1, "server.init"))
            else {
                return Some(Err("`server.init(domain, admin, team?)`".into()));
            };
            match (
                arg(domain, 253),
                arg(admin, 254),
                arg(opt_s(args, 2).unwrap_or("team"), 64),
            ) {
                (Ok(d), Ok(m), Ok(t)) => {
                    run_json(a(&[
                        "server", "init", "--domain", &d, "--admin", &m, "--team", &t, "--json",
                    ]))
                    .await
                }
                _ => Err("Geçersiz alan adı, e-posta ya da takım adı".into()),
            }
        }
        "addMember" => match s(args, 0, "server.addMember").and_then(|e| arg(e, 254)) {
            Ok(email) => {
                let role = opt_s(args, 1).unwrap_or("member");
                if !matches!(role, "admin" | "member" | "dev" | "viewer") {
                    return Some(Err("Rol: admin, member ya da viewer".into()));
                }
                let mut v = a(&["server", "add-member", &email, "--role", role, "--json"]);
                if b(args, 2) {
                    v.push("--remove".into());
                }
                run_json(v).await
            }
            Err(e) => Err(e),
        },
        "status" => run_json(a(&["server", "status", "--json"])).await,
        "start" => {
            let exe = match crate::kor_cli::find() {
                Some(e) => e,
                None => return Some(Err(crate::kor_cli::MISSING.to_owned())),
            };
            let r = std::process::Command::new(exe)
                .args(["server", "start", "--background"])
                .no_window()
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            match r {
                Ok(st) if st.success() => Ok(json!({ "started": true })),
                Ok(_) => Err("Takım sunucusu başlatılamadı (kor server status)".into()),
                Err(e) => Err(e.to_string()),
            }
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn options_cannot_be_smuggled_in_as_values() {
        assert!(super::arg("kor.example.com", 253).is_ok());
        assert!(super::arg("--db=postgres://x", 253).is_err());
        assert!(super::arg("", 253).is_err());
        assert!(super::arg("a\nb", 253).is_err());
    }
}
