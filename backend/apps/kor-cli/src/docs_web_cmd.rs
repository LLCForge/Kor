//! `kor docs use <url> "<question>"` and `kor docs cache` (§27.11, ADR 0058): a documentation page
//! answered for this project, the CLI side of the browser side panel's "Bu sayfayı projemde
//! kullan". These two subcommands join the `kor docs` command of ADR 0063; logic in
//! `kor_runtime::browser_assist::docs`.

use kor_i18n::{Localizer, args};
use kor_runtime::browser_assist::docs;

use crate::Ctx;

#[derive(Debug, clap::Args)]
pub struct UseArgs {
    pub url: String,
    /// The question (default: how to use the page in this project).
    pub question: Vec<String>,
    /// Read the page in your own browser (pages behind a login), through the Kor extension.
    #[arg(long)]
    pub browser: bool,
    #[arg(long)]
    pub model: Option<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, clap::Args)]
pub struct CacheArgs {
    /// Empty the cache.
    #[arg(long)]
    pub clear: bool,
    #[arg(long)]
    pub json: bool,
}

pub fn localize_use(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-docs-use"))
        .mut_arg("url", |a| a.help(l.t("arg-docs-url")))
        .mut_arg("question", |a| a.help(l.t("arg-docs-question")))
        .mut_arg("browser", |a| a.help(l.t("arg-docs-web-browser")))
        .mut_arg("model", |a| a.help(l.t("arg-model")))
        .mut_arg("json", |a| a.help(l.t("arg-json")))
}

pub fn localize_cache(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-docs-cache"))
        .mut_arg("clear", |a| a.help(l.t("arg-docs-cache-clear")))
        .mut_arg("json", |a| a.help(l.t("arg-json")))
}

pub fn use_page(ctx: &Ctx, a: &UseArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let loaded = ctx.config.as_ref().map_err(|e| anyhow::anyhow!("{e}"))?;
    let root = kor_runtime::osint::project_root(&std::env::current_dir()?);
    let (provider, model) = kor_runtime::oneshot_provider(l, loaded, a.model.as_deref())
        .map_err(|e| anyhow::anyhow!(e))?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let question = a.question.join(" ");
    let cached = docs::cache_dir().and_then(|d| docs::cache_get(&d, &a.url));
    let page = if let Some(c) = cached {
        println!("{}", l.t("docs-use-cached"));
        c.page
    } else {
        println!(
            "{}",
            l.t_args("docs-use-reading", &args!("url" => a.url.clone()))
        );
        let (title, text) = if a.browser {
            rt.block_on(kor_tools::chrome::read_page(&a.url))
                .map_err(|e| anyhow::anyhow!(e))?
        } else {
            let (text, title) = rt
                .block_on(kor_runtime::docs::extract(&a.url, false))
                .map_err(|e| anyhow::anyhow!(e))?;
            (title, text)
        };
        docs::Page {
            url: a.url.clone(),
            title,
            text,
            selection: String::new(),
        }
    };
    let lang = l.lang();
    let answer = rt
        .block_on(docs::answer(
            &provider, &model, &root, page, &question, lang,
        ))
        .map_err(|e| anyhow::anyhow!(e))?;
    if a.json {
        println!("{}", serde_json::to_string_pretty(&answer)?);
        return Ok(true);
    }
    if let Some(lib) = &answer.library {
        println!(
            "{}",
            l.t_args(
                "docs-use-library",
                &args!("name" => lib.name.clone(), "page" => or_dash(&lib.page_version), "project" => or_dash(&lib.project_version))
            )
        );
        if lib.mismatch {
            println!(
                "{}",
                l.t_args("docs-use-mismatch", &args!("page" => lib.page_version.clone(), "project" => lib.project_version.clone()))
            );
        }
    }
    println!("\n{}", answer.answer.trim());
    if !answer.places.is_empty() {
        println!(
            "\n{}",
            l.t_args(
                "docs-use-places",
                &args!("places" => answer.places.join(", "))
            )
        );
    }
    Ok(true)
}

fn or_dash(s: &str) -> String {
    if s.is_empty() {
        "—".into()
    } else {
        s.to_owned()
    }
}

pub fn cache(ctx: &Ctx, a: &CacheArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let dir = docs::cache_dir().ok_or_else(|| anyhow::anyhow!(l.t("system-no-home")))?;
    if a.clear {
        println!(
            "{}",
            l.t_args(
                "docs-cache-cleared",
                &args!("count" => docs::cache_clear(&dir))
            )
        );
        return Ok(true);
    }
    let list = docs::cache_list(&dir);
    if a.json {
        let rows: Vec<serde_json::Value> = list
            .iter()
            .map(|c| serde_json::json!({"url": c.page.url, "title": c.page.title, "fetchedAt": c.fetched_at, "chars": c.page.text.chars().count()}))
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(true);
    }
    if list.is_empty() {
        println!("{}", l.t("docs-cache-empty"));
    }
    for c in list {
        println!("{}  {}", c.page.url, c.page.title);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::*;

    #[derive(Debug, clap::Parser)]
    enum T {
        Use(UseArgs),
        Cache(CacheArgs),
    }

    #[test]
    fn parses() {
        let T::Use(u) = T::parse_from([
            "t",
            "use",
            "https://docs.rs/serde",
            "how",
            "do",
            "I",
            "derive?",
            "--browser",
        ]) else {
            panic!("use")
        };
        assert_eq!(
            (u.url.as_str(), u.question.join(" ").as_str(), u.browser),
            ("https://docs.rs/serde", "how do I derive?", true)
        );
        assert!(matches!(
            T::parse_from(["t", "cache", "--clear"]),
            T::Cache(CacheArgs { clear: true, .. })
        ));
        let l = Localizer::new(kor_i18n::Lang::Fr);
        let cmd = <T as clap::CommandFactory>::command();
        let _ = cmd
            .mut_subcommand("use", |c| localize_use(c, &l))
            .mut_subcommand("cache", |c| localize_cache(c, &l));
    }
}
