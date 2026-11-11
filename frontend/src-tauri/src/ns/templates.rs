//! `templates.*`: `kor templates list|show|new` ve `kor run template:<ad>` için ince köprü
//! (parity.toml, §15.11). Yükleme, doğrulama ve işleme `kor_compat::templates` içinde; burası
//! yalnızca JSON'a çevirir. `run` ajanı başlatmaz: doğrulanmış istemi döner, arayüz onu normal
//! sohbet yolundan (`send`) gönderir.

use kor_compat::templates::{self, InputError, Template};
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, b, root, s};
use crate::state::AppState;

/// Köprü ad alanı (`ns/mod.rs` bunu `templates::call`'a yönlendirir).
pub const NS: &str = "templates";

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    _app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    let m = format!("templates.{name}");
    let m = m.as_str();
    let root = match root(state) {
        Ok(r) => r,
        Err(e) => return Some(Err(e)),
    };
    Some(match name {
        "list" => Ok(list(&root)),
        // args: name
        "show" => s(args, 0, m).and_then(|n| find(&root, n)).map(|t| view(&t)),
        // args: name, inputs ({ ad: değer }) → { prompt, model }
        "run" => s(args, 0, m).and_then(|n| run(&root, n, args.get(1))),
        // args: name → varsayılanlarla, zorunlu girdiler `<ad>` olarak işlenmiş taslak
        "draft" => s(args, 0, m).and_then(|n| find(&root, n)).map(|t| {
            json!({ "prompt": templates::render(&t, &templates::draft_values(&t)), "model": t.model })
        }),
        // args: name, personal
        "new" => s(args, 0, m).and_then(|n| create(&root, n, b(args, 1))),
        _ => return None,
    })
}

fn source(t: &Template) -> &'static str {
    match &t.source {
        None => "builtin",
        Some(s) if s.scope == kor_compat::Scope::Project => "project",
        Some(s) => {
            if dirs::home_dir().is_some_and(|h| s.path.starts_with(templates::dir(&h))) {
                "personal"
            } else {
                "plugin"
            }
        }
    }
}

fn view(t: &Template) -> Value {
    json!({
        "name": t.name,
        "description": t.description,
        "source": source(t),
        "path": t.source.as_ref().map(|s| kor_tools::display_path(&s.path)),
        "model": t.model,
        "inputs": t.inputs,
        "steps": t.steps,
        "acceptance": t.acceptance,
    })
}

fn list(root: &std::path::Path) -> Value {
    let c = kor_runtime::load_compat(root);
    let warnings: Vec<&String> = c
        .warnings
        .iter()
        .filter(|w| w.contains("invalid template"))
        .collect();
    json!({
        "templates": templates::all(&c).iter().map(view).collect::<Vec<_>>(),
        "warnings": warnings,
    })
}

fn find(root: &std::path::Path, name: &str) -> Result<Template, String> {
    templates::find(&kor_runtime::load_compat(root), name)
        .ok_or_else(|| format!("{name} adında şablon yok."))
}

/// `{ "name": "users", "auth": false }` → `--name=users --auth=false`, sonra CLI ile aynı doğrulama.
fn run(root: &std::path::Path, name: &str, inputs: Option<&Value>) -> Res {
    let t = find(root, name)?;
    let words: Vec<String> = match inputs {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Object(map)) => map
            .iter()
            .map(|(k, v)| {
                let v = match v {
                    Value::String(s) => s.clone(),
                    Value::Bool(_) | Value::Number(_) => v.to_string(),
                    _ => {
                        return Err(format!(
                            "--{k} için metin, sayı ya da evet/hayır bekleniyordu."
                        ));
                    }
                };
                Ok(format!("--{k}={v}"))
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err("Şablon girdileri bir nesne olmalı.".into()),
    };
    let values = templates::values(&t, &words).map_err(|e| message(&t.name, &e))?;
    Ok(json!({ "prompt": templates::render(&t, &values), "model": t.model }))
}

fn create(root: &std::path::Path, name: &str, personal: bool) -> Res {
    let base = if personal {
        dirs::home_dir().ok_or("Ev dizini bulunamadı.")?
    } else {
        root.to_owned()
    };
    let dir = templates::dir(&base);
    match templates::create(&dir, name) {
        Ok(p) => Ok(json!({ "path": kor_tools::display_path(&p) })),
        Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => Err(format!(
            "Şablon adları 1-64 harf, rakam, - ya da _ olmalı: {name}"
        )),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(format!("{name}.md zaten var; hiçbir şey değiştirilmedi."))
        }
        Err(e) => Err(format!("Şablon oluşturulamadı: {e}")),
    }
}

fn message(template: &str, e: &InputError) -> String {
    match e {
        InputError::Unknown(n) => format!("{template} şablonunda --{n} girdisi yok."),
        InputError::Missing(n) => format!("{template} şablonu --{n} ister."),
        InputError::Duplicate(n) => format!("--{n} iki kez verildi."),
        InputError::Invalid { name, expected } => format!("--{name} şu olmalı: {expected}."),
        InputError::ExtraValue(v) => format!("Beklenmeyen değer: {v}"),
        InputError::Syntax => "Girdilerdeki bir tırnak kapatılmamış.".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_validates_inputs_like_the_cli() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let ok = run(
            root,
            "new-endpoint",
            Some(&json!({ "name": "users", "auth": false })),
        )
        .unwrap();
        let prompt = ok["prompt"].as_str().unwrap();
        assert!(
            prompt.contains("`users` resource")
                && prompt.contains("Authentication required: false")
        );
        let err = run(
            root,
            "new-endpoint",
            Some(&json!({ "method": "TRACE", "name": "x" })),
        )
        .unwrap_err();
        assert!(err.contains("--method"), "{err}");
        assert!(
            run(root, "new-endpoint", None)
                .unwrap_err()
                .contains("--name")
        );
        assert!(run(root, "new-endpoint", Some(&json!(["x"]))).is_err());
        assert!(find(root, "yok").is_err());

        let made = create(root, "mine", false).unwrap();
        assert!(made["path"].as_str().unwrap().ends_with("mine.md"));
        assert!(create(root, "mine", false).is_err());
        assert!(create(root, "../x", false).is_err());
        let names: Vec<String> = list(root)["templates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap_or_default().to_owned())
            .collect();
        assert!(names.contains(&"mine".to_owned()) && names.contains(&"test-from-bug".to_owned()));
    }
}
