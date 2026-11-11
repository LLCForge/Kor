//! `kor config explain <section>` (ADR 0050): `auto` lists every automatic behaviour with its
//! value, source and the older keys it governs (§28.14); `keys` shows the key layout, the
//! `keymap.toml` bindings and their conflicts (§28.10); any other section filters the settings by
//! prefix (`ui`, `tui`, `context`…). Logic: `kor_runtime::prefs`.

use kor_config::Layer;
use kor_i18n::{Localizer, args};

use crate::Ctx;

fn source(l: &Localizer, layer: Layer, origin: &str) -> String {
    match layer {
        Layer::Default => l.t("config-source-default"),
        Layer::Env => l.t_args("config-source-env", &args!("origin" => origin.to_owned())),
        Layer::Cli => l.t("config-source-cli"),
        Layer::System | Layer::User | Layer::Project => {
            l.t_args("config-source-file", &args!("origin" => origin.to_owned()))
        }
    }
}

pub fn explain(ctx: &Ctx, section: &str) -> bool {
    let l = &ctx.l;
    let loaded = match &ctx.config {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}", crate::config_error(l, e));
            return false;
        }
    };
    match section {
        "auto" => {
            println!("{}", l.t("config-auto-header"));
            for r in kor_runtime::prefs::auto_rows(loaded) {
                let layer = loaded.settings.get(&format!("auto.{}", r.key)).map_or(Layer::Default, |s| s.layer);
                println!(
                    "  {}",
                    l.t_args(
                        "config-auto-line",
                        &args!(
                            "key" => format!("auto.{}", r.key),
                            "value" => if r.value { "on" } else { "off" },
                            "about" => l.t(&format!("auto-{}", r.key.replace('_', "-"))),
                            "source" => source(l, layer, &r.source)
                        )
                    )
                );
                for (k, v) in &r.related {
                    println!("      {k} = {v}");
                }
            }
            println!("{}", l.t("config-auto-footer"));
            true
        }
        "keys" | "keymap" => {
            let root = crate::graph_cmd::root().unwrap_or_default();
            let km = kor_runtime::prefs::load_keymap(&root);
            let mode = km.mode.unwrap_or_else(|| loaded.config.key_mode());
            println!("{}", l.t_args("config-keys-mode", &args!("mode" => format!("{mode:?}").to_lowercase())));
            for p in kor_runtime::prefs::keymap_paths(&root) {
                let state = if km.files.contains(&p) { "✓" } else { "-" };
                println!("  {state} {}", p.display());
            }
            for (k, a) in &km.tui {
                println!("  [tui] {k} = {a}");
            }
            for (k, a) in &km.composer {
                println!("  [composer] {k} = {a}");
            }
            for w in &km.warnings {
                println!("  ! {w}");
            }
            km.warnings.is_empty()
        }
        prefix => {
            let p = format!("{prefix}.");
            let mut any = false;
            for (key, s) in loaded.settings.iter().filter(|(k, _)| k.starts_with(&p) || *k == prefix) {
                any = true;
                println!(
                    "  {}",
                    l.t_args(
                        "config-explain-line",
                        &args!("key" => key.clone(), "value" => s.value.to_string(), "source" => source(l, s.layer, &s.origin))
                    )
                );
            }
            if !any {
                eprintln!("{}", l.t_args("config-explain-none", &args!("section" => prefix.to_owned())));
            }
            any
        }
    }
}
