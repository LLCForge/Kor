//! `find.*`: `kor find` (§8.9, ADR 0042) için ince köprü. Arama `kor_agent::find` içinde;
//! burası yalnızca JSON'a çevirir.
//!
//! - `find.run(question: string, limit?: number, explain?: bool, model?: string)` →
//!   `{ found: { question, hits, links, text_index, graph_nodes, notes }, explanation, provider,
//!   explainError }`. `explain` kapalıyken model çağrılmaz; liste tek başına sonuçtur.

use std::path::Path;

use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, b, opt_s, root, s};
use crate::state::AppState;

/// Köprü ad alanı (`ns/mod.rs` bunu `find::call`'a yönlendirir).
pub const NS: &str = "find";

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    _app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    Some(match name {
        "run" => match (s(args, 0, "find.run"), limit(args.get(1)), root(state)) {
            (Ok(q), Ok(n), Ok(dir)) => run_at(&dir, q, n, b(args, 2), opt_s(args, 3)).await,
            (Err(e), ..) | (_, Err(e), _) | (.., Err(e)) => Err(e),
        },
        _ => return None,
    })
}

fn limit(v: Option<&Value>) -> Result<usize, String> {
    match v {
        None | Some(Value::Null) => Ok(10),
        Some(v) => v
            .as_u64()
            .filter(|n| (1..=50).contains(n))
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| "Sonuç sayısı 1-50 olmalı".into()),
    }
}

async fn run_at(
    dir: &Path,
    question: &str,
    limit: usize,
    explain: bool,
    model: Option<&str>,
) -> Res {
    let mut ctx = kor_tools::ToolCtx::new(dir).map_err(|e| format!("Proje klasörü: {e}"))?;
    ctx.load_ai_ignore();
    // Yerel modelin dizini `file_search` ile aynıdır; hiçbir şey makineden çıkmaz.
    let semantic = kor_agent::semantic::local_embedder()
        .zip(kor_runtime::kor_home())
        .map(|(m, home)| kor_agent::semantic::FileSearch::new(m, &home.join("index"), &ctx.root));
    let found = kor_agent::find::search(&ctx, question, limit, semantic.as_ref())
        .await
        .map_err(|e| format!("Arama yapılamadı: {e}"))?;
    let (mut explanation, mut provider_name, mut explain_error) = (None, None, None);
    if explain && !found.hits.is_empty() {
        // Gizlilik modu: yerel olmayan sağlayıcıyı oneshot_provider reddeder.
        let picked = crate::bridge::load_config(dir).and_then(|loaded| {
            let l = kor_i18n::Localizer::new(loaded.config.ui.language.resolve());
            kor_runtime::oneshot_provider(&l, &loaded, model)
        });
        match picked {
            Err(e) => explain_error = Some(e),
            Ok((provider, model)) => {
                provider_name = Some(provider.manifest().name.clone());
                match kor_agent::find::explain(&provider, &model, &ctx.root, &found).await {
                    Ok(t) => explanation = Some(t),
                    Err(e) => explain_error = Some(format!("Model yanıtı kullanılamadı: {e}")),
                }
            }
        }
    }
    Ok(json!({
        "found": found,
        "explanation": explanation,
        "provider": provider_name,
        "explainError": explain_error,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_defaults_and_bounds() {
        assert_eq!(limit(None), Ok(10));
        assert_eq!(limit(Some(&json!(25))), Ok(25));
        assert!(limit(Some(&json!(0))).is_err() && limit(Some(&json!("x"))).is_err());
    }

    #[test]
    fn lists_places_without_a_model() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join("auth.rs"),
            "pub fn login(user: &str) -> bool {\n    check_password(user)\n}\nfn check_password(_u: &str) -> bool { true }\n",
        )
        .unwrap();
        let v = tauri::async_runtime::block_on(run_at(
            d.path(),
            "how does login check the password",
            5,
            false,
            None,
        ))
        .unwrap();
        let hits = v["found"]["hits"].as_array().unwrap();
        assert!(hits.iter().any(|h| h["file"] == "auth.rs"), "{v}");
        assert!(v["explanation"].is_null() && v["explainError"].is_null());
    }
}
