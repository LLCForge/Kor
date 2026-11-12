//! Ad alanlı köprü yöntemleri (`kor.mcp.list` → `"mcp.list"`). Her dosya birkaç ad alanına sahiptir;
//! sözleşmeler `frontend/src/desktop/ipc-*.ts`.

use serde_json::Value;
use tauri::AppHandle;

use crate::api::Res;
use crate::state::AppState;

mod acp;
mod analysis;
mod batch;
mod bench;
pub(crate) mod catalog;
mod chrome;
mod compat;
mod components;
mod context;
mod core;
mod cost;
mod db;
mod deploy;
mod deps;
mod devenv;
mod diagrams;
mod docs;
mod eco;
mod estimate;
mod files;
mod fim;
mod find;
mod forge;
mod forks;
mod gitflow;
mod graph;
mod hooks;
mod intent;
mod know;
mod korfolder;
mod local;
mod mcpdev;
mod mcpgen;
mod media;
mod memory;
mod metrics;
mod offline;
pub(crate) mod osextra;
mod osint;
mod packages;
mod peer;
mod perf;
mod prefs;
mod prereq;
mod privacycard;
mod projmem;
mod provider;
mod providers;
mod proxy;
mod remote;
mod replay;
mod rules;
mod security;
mod seo;
mod shell;
mod shellui;
mod system;
mod tasks;
mod team;
mod templates;
mod tools;
mod triggers;
mod visual;
mod watch;
mod web;
mod webview;

pub(crate) use files::session_prompt_block;
pub(crate) use providers::shutdown;
pub(crate) use security::g12_sandbox;

/// Uygulama açılışında: zamanlanmış görevler.
pub fn start(app: AppHandle) {
    korfolder::start(app.clone());
    osextra::start(app.clone());
    system::start(app.clone());
    local::start_scheduler(app);
}

pub async fn call(method: &str, args: &[Value], app: &AppHandle, state: &AppState) -> Res {
    let Some((ns, name)) = method.split_once('.') else {
        return Err(format!("`{method}` henüz Kor arka ucunda yok"));
    };
    let out = match ns {
        "mcp" | "plugins" | "agents" | "commands" | "teams" => {
            eco::call(ns, name, args, app, state).await
        }
        "git" | "terminal" | "logs" | "routines" | "notes" | "tags" | "connections" | "g10" => {
            local::call(ns, name, args, app, state).await
        }
        "g4" | "g13" | "g30" | "g14" => files::call(ns, name, args, app, state).await,
        "g9" | "g35" | "preview" | "abrowser" => providers::call(ns, name, args, app, state).await,
        "chromeExtension" => chrome::call(ns, name, args, app, state).await,
        "core" => core::call(ns, name, args, app, state).await,
        "compat" => compat::call(ns, name, args, app, state).await,
        "catalog" => catalog::call(ns, name, args, app, state).await,
        "provider" => provider::call(ns, name, args, app, state).await,
        "components" => components::call(ns, name, args, app, state).await,
        "media" => media::call(ns, name, args, app, state).await,
        proxy::NS => proxy::call(ns, name, args, app, state).await,
        bench::NS => bench::call(ns, name, args, app, state).await,
        context::NS => context::call(ns, name, args, app, state).await,
        cost::NS => cost::call(ns, name, args, app, state).await,
        gitflow::NS => gitflow::call(ns, name, args, app, state).await,
        shell::NS => shell::call(ns, name, args, app, state).await,
        templates::NS => templates::call(ns, name, args, app, state).await,
        memory::NS => memory::call(ns, name, args, app, state).await,
        packages::NS => packages::call(ns, name, args, app, state).await,
        graph::NS => graph::call(ns, name, args, app, state).await,
        db::NS => db::call(ns, name, args, app, state).await,
        projmem::NS => projmem::call(ns, name, args, app, state).await,
        "vault" | "audit" | "sandbox" => security::call(ns, name, args, app, state).await,
        team::NS => team::call(ns, name, args, app, state).await,
        offline::NS => offline::call(ns, name, args, app, state).await,
        analysis::NS => analysis::call(ns, name, args, app, state).await,
        deps::NS => deps::call(ns, name, args, app, state).await,
        metrics::NS => metrics::call(ns, name, args, app, state).await,
        perf::NS => perf::call(ns, name, args, app, state).await,
        visual::NS => visual::call(ns, name, args, app, state).await,
        tasks::NS => tasks::call(ns, name, args, app, state).await,
        mcpgen::NS => mcpgen::call(ns, name, args, app, state).await,
        forge::NS => forge::call(ns, name, args, app, state).await,
        triggers::NS => triggers::call(ns, name, args, app, state).await,
        deploy::NS => deploy::call(ns, name, args, app, state).await,
        watch::NS => watch::call(ns, name, args, app, state).await,
        find::NS => find::call(ns, name, args, app, state).await,
        replay::NS => replay::call(ns, name, args, app, state).await,
        devenv::NS => devenv::call(ns, name, args, app, state).await,
        remote::NS => remote::call(ns, name, args, app, state).await,
        "prefs" | "plan" | "sessions" => prefs::call(ns, name, args, app, state).await,
        "system" | "computer" | "layout" => system::call(ns, name, args, app, state).await,
        webview::NS => webview::call(ns, name, args, app, state).await,
        acp::NS => acp::call(ns, name, args, app, state).await,
        batch::NS => batch::call(ns, name, args, app, state).await,
        diagrams::NS => diagrams::call(ns, name, args, app, state).await,
        docs::NS => docs::call(ns, name, args, app, state).await,
        estimate::NS => estimate::call(ns, name, args, app, state).await,
        fim::NS => fim::call(ns, name, args, app, state).await,
        forks::NS => forks::call(ns, name, args, app, state).await,
        hooks::NS => hooks::call(ns, name, args, app, state).await,
        intent::NS => intent::call(ns, name, args, app, state).await,
        know::NS => know::call(ns, name, args, app, state).await,
        korfolder::NS => korfolder::call(ns, name, args, app, state).await,
        mcpdev::NS => mcpdev::call(ns, name, args, app, state).await,
        osextra::NS => osextra::call(ns, name, args, app, state).await,
        osint::NS => osint::call(ns, name, args, app, state).await,
        peer::NS => peer::call(ns, name, args, app, state).await,
        prereq::NS => prereq::call(ns, name, args, app, state).await,
        privacycard::NS => privacycard::call(ns, name, args, app, state).await,
        rules::NS => rules::call(ns, name, args, app, state).await,
        seo::NS => seo::call(ns, name, args, app, state).await,
        shellui::NS => shellui::call(ns, name, args, app, state).await,
        tools::NS => tools::call(ns, name, args, app, state).await,
        web::NS => web::call(ns, name, args, app, state).await,
        _ => None,
    };
    out.unwrap_or_else(|| Err(format!("`{method}` henüz Kor arka ucunda yok")))
}

#[cfg(test)]
mod tests {
    /// Every `desktop` surface in `parity.toml` names a bridge method this app implements.
    #[test]
    fn parity_desktop_surfaces_exist() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let read_dir = |d: std::path::PathBuf| -> String {
            std::fs::read_dir(d)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "rs"))
                .map(|e| std::fs::read_to_string(e.path()).unwrap_or_default())
                .collect()
        };
        let root = read_dir(src.clone());
        let ns_dir = src.join("ns");
        let read_ns = |file: &str| {
            std::fs::read_to_string(ns_dir.join(format!("{file}.rs"))).unwrap_or_default()
        };
        // Ad alanı → onu işleyen dosya, `call` içindeki yönlendirme kollarından (`"a" | "b" => x::call`, `x::NS => x::call`).
        let mut owner = std::collections::HashMap::<String, String>::new();
        let mod_rs = read_ns("mod");
        let lines: Vec<&str> = mod_rs.lines().collect();
        for (i, line) in lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.contains("::call(ns, name"))
        {
            // Uzun kol rustfmt ile bölünür: desen bir önceki satırda (`"a" | "b" => {`).
            let (pat, target) = match line.split_once("=>") {
                Some(split) => split,
                None => match lines[..i].last().and_then(|p| p.split_once("=>")) {
                    Some((pat, _)) => (pat, *line),
                    None => continue,
                },
            };
            let file = target
                .trim()
                .split("::")
                .next()
                .unwrap_or_default()
                .to_owned();
            let names: Vec<String> = if pat.contains("::NS") {
                let body = read_ns(&file);
                let decl = body
                    .lines()
                    .find(|l| l.starts_with("pub const NS: &str"))
                    .unwrap_or_default();
                decl.split('"')
                    .nth(1)
                    .into_iter()
                    .map(str::to_owned)
                    .collect()
            } else {
                pat.split('"')
                    .skip(1)
                    .step_by(2)
                    .map(str::to_owned)
                    .collect()
            };
            for n in names {
                owner.insert(n, file.clone());
            }
        }
        // `"name"` gerçek bir eşleşme kolu mu: ardından `=>`, `|` ya da `) =>` (demet kolu) gelir,
        // `== "name"` / `!= "name"` korumasıdır ya da `matches!(name, …)` izin listesindedir.
        let has_arm = |body: &str, name: &str| {
            let lit = format!("\"{name}\"");
            body.match_indices(&lit).any(|(i, _)| {
                let before = &body[..i];
                let after = body[i + lit.len()..].trim_start();
                let after = after.strip_prefix(')').map_or(after, str::trim_start);
                let in_matches = before
                    .rfind("matches!(")
                    .is_some_and(|m| !before[m..].contains(')'));
                after.starts_with("=>")
                    || after.starts_with('|')
                    || before.trim_end().ends_with("==")
                    || before.trim_end().ends_with("!=")
                    || in_matches
            })
        };
        let text = std::fs::read_to_string(src.join("../../../parity.toml")).unwrap();
        let doc: toml_edit::DocumentMut = text.parse().unwrap();
        let mut missing = Vec::new();
        for f in doc["feature"].as_array_of_tables().unwrap() {
            let Some(d) = f.get("desktop").and_then(toml_edit::Item::as_str) else {
                continue;
            };
            if d == "-" {
                continue;
            }
            let ok = match d.split_once('.') {
                Some((space, name)) => owner
                    .get(space)
                    .is_some_and(|file| has_arm(&read_ns(file), name)),
                None => root.contains(&format!("\"{d}\"")),
            };
            if !ok {
                missing.push(d.to_owned());
            }
        }
        assert!(
            missing.is_empty(),
            "parity.toml names unknown bridge methods: {missing:?}"
        );
    }
}
