//! `shell.*`: kabuk entegrasyonu için ince köprü (§15.4, parity.toml). İş mantığı
//! `kor_agent::shellint` ve `kor_runtime::shell` içinde (CLI ile ortak); burası JSON'a çevirir.
//!
//! - `shell.install(shell?: string, confirm?: bool)` / `shell.uninstall(...)` → `{ shell, path,
//!   block, changed, written }`. `confirm` olmadan yalnızca önizleme döner, hiçbir şey yazılmaz.
//! - `shell.status()` → `{ shells: [{ shell, path, installed, present, isDefault }], record, last? }`;
//!   `present`: kabuğun kendisi bu bilgisayarda var (`installed` yalnızca Kor bloğu demektir).
//! - `shell.capture(command: string[])` → `{ code, output }`; başarısızsa kayıt tutulur.
//! - `shell.why(model?)` → `{ failure, explanation }`.
//! - `shell.fix(model?)` → `{ failure, explanation, command? }`. Hiçbir şey çalıştırılmaz.
//! - `shell.runFix(command: string)` → `{ code, output }`. Arayüzdeki onay düğmesi onaydır.

use std::process::Stdio;

use kor_agent::shellint::{self, Failure};
use kor_platform::NoWindow as _;
use kor_runtime::shell::{self as core, Plan, RunError, ShellError};
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, b, opt_s, root, s};
use crate::state::AppState;

/// Köprü ad alanı (`ns/mod.rs` bunu `shell::call`'a yönlendirir).
pub const NS: &str = "shell";

const MAX_COMMAND: usize = 8_192;

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    _app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    Some(match name {
        "install" => change(opt_s(args, 0), b(args, 1), true),
        "uninstall" => change(opt_s(args, 0), b(args, 1), false),
        "status" => Ok(status()),
        "capture" => capture(state, args.first()).await,
        "why" => explain(state, opt_s(args, 0), true).await,
        "fix" => explain(state, opt_s(args, 0), false).await,
        "runFix" => match s(args, 0, "shell.runFix") {
            Ok(c) => run_fix(c.to_owned()).await,
            Err(e) => Err(e),
        },
        _ => return None,
    })
}

fn plan_err(e: ShellError) -> String {
    match e {
        ShellError::NoHome => "Kor veri klasörü bulunamadı.".into(),
        ShellError::RcUnknown(k) => format!("{} başlangıç dosyasının yeri bilinmiyor.", k.name()),
        ShellError::Io(p, e) => format!("{}: {e}", kor_tools::display_path(&p)),
    }
}

fn change(shell: Option<&str>, confirm: bool, install: bool) -> Res {
    let k = core::kind(shell.filter(|s| !s.trim().is_empty()))
        .ok_or_else(|| "Kabuk tanınmadı. bash, zsh, fish veya powershell belirtin.".to_owned())?;
    let plan: Plan = if install {
        core::plan_install(k)
    } else {
        core::plan_uninstall(k)
    }
    .map_err(plan_err)?;
    let shown = kor_tools::display_path(&plan.rc);
    let changed = plan.new.is_some();
    if confirm && changed {
        core::apply(&plan).map_err(|e| format!("{shown}: {e}"))?;
    }
    Ok(json!({
        "shell": k.name(),
        "path": shown,
        "block": plan.block,
        "changed": changed,
        "written": confirm && changed,
    }))
}

fn status() -> Value {
    let default = core::default_kind();
    let shells: Vec<Value> = core::status()
        .into_iter()
        .map(|(k, rc, on)| {
            json!({
                "shell": k.name(),
                "path": kor_tools::display_path(&rc),
                "installed": on,
                "present": core::present(k),
                "isDefault": k == default,
            })
        })
        .collect();
    json!({
        "shells": shells,
        "record": core::record_path().map(|p| kor_tools::display_path(&p)),
        "last": core::load_failure(),
    })
}

async fn capture(state: &AppState, arg: Option<&Value>) -> Res {
    let command: Vec<String> = arg
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let total: usize = command.iter().map(String::len).sum();
    if command.first().is_none_or(|p| p.trim().is_empty()) || total > MAX_COMMAND {
        return Err(format!(
            "`shell.capture`: komut boş olmayan bir metin dizisi olmalı (en çok {MAX_COMMAND} karakter)"
        ));
    }
    let dir = root(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let Some((program, rest)) = command.split_first() else {
            return Err("`shell.capture`: komut boş".to_owned());
        };
        let (code, output) = shellint::capture_command(
            std::process::Command::new(program)
                .args(rest)
                .current_dir(&dir)
                .stdin(Stdio::null())
                .no_window(),
        )
        .map_err(|e| format!("{program}: {e}"))?;
        if code != 0 {
            core::record_failure(&command, code, &dir, output.clone());
        }
        Ok(json!({ "code": code, "output": output }))
    })
    .await
    .map_err(|e| format!("Komut çalıştırılamadı: {e}"))?
}

fn no_record() -> String {
    "Kayıtlı başarısız komut yok. Kabuk entegrasyonunu kurun ya da `shell.capture` kullanın.".into()
}

async fn explain(state: &AppState, model: Option<&str>, explain_only: bool) -> Res {
    let f: Failure = core::load_failure().ok_or_else(no_record)?;
    let loaded = crate::bridge::load_config(&root(state)?)?;
    let l = kor_i18n::Localizer::new(loaded.config.ui.language.resolve());
    // Gizlilik modu: yerel olmayan sağlayıcıyı oneshot_provider reddeder.
    let (provider, model) = kor_runtime::oneshot_provider(&l, &loaded, model)?;
    // fix_prompt kaydı göndermeden önce gizli değerleri maskeler.
    let prompt = shellint::fix_prompt(&f, std::env::consts::OS, explain_only);
    let reply = kor_agent::gitflow::ask(&provider, &model, prompt, 1024)
        .await
        .map_err(|e| format!("Model yanıtı kullanılamadı: {e}"))?;
    if explain_only {
        return Ok(json!({ "failure": f, "explanation": reply.trim() }));
    }
    let fix = shellint::parse_fix(&reply);
    Ok(json!({ "failure": f, "explanation": fix.explanation, "command": fix.command }))
}

async fn run_fix(cmd: String) -> Res {
    if cmd.trim().is_empty() || cmd.len() > MAX_COMMAND {
        return Err(format!("Komut 1-{MAX_COMMAND} karakter olmalı"));
    }
    let f = core::load_failure().ok_or_else(no_record)?;
    tauri::async_runtime::spawn_blocking(move || {
        match core::run_fix(&f, &cmd, |c| {
            shellint::capture_command(c.stdin(Stdio::null()).no_window())
        }) {
            Ok((code, output)) => Ok(json!({ "code": code, "output": output })),
            Err(RunError::Io(e)) => Err(format!("Komut çalıştırılamadı: {e}")),
            Err(RunError::ShellMissing(k)) => {
                Err(format!("{} bulunamadı (PATH'te yok).", k.name()))
            }
        }
    })
    .await
    .map_err(|e| format!("Komut çalıştırılamadı: {e}"))?
}
