//! Context Budget Manager views (§5.3) shared by `kor context`, the REPL's `/context` and status
//! line: localized text from [`kor_agent::context::Breakdown`]s. Every figure is ≈ (estimated).

use std::fmt::Write as _;

use kor_agent::context::Breakdown;
use kor_i18n::{Localizer, args};

/// `45k`, `1.2M`; small numbers as they are.
pub fn human(n: u64) -> String {
    match n {
        n if n >= 1_000_000 => format!("{}.{}M", n / 1_000_000, n % 1_000_000 / 100_000),
        n if n >= 1000 => format!("{}k", (n + 500) / 1000),
        n => n.to_string(),
    }
}

fn category(l: &Localizer, key: &str) -> String {
    l.t(&format!("context-cat-{key}"))
}

/// One line for the REPL after each request.
pub fn status(l: &Localizer, b: &Breakdown) -> String {
    let total = b.total();
    l.t_args(
        "context-status",
        &args!("used" => human(total), "window" => human(b.window), "percent" => b.percent(total)),
    )
}

/// The first-time "category dominates" warning carried by the breakdown, offering `/compact`.
pub fn warning(l: &Localizer, b: &Breakdown) -> Option<String> {
    let key = b.warning.as_deref()?;
    let tokens = b
        .parts()
        .iter()
        .find(|(k, _)| *k == key)
        .map_or(0, |(_, t)| *t);
    Some(l.t_args(
        "context-warn",
        &args!("category" => category(l, key), "percent" => b.percent(tokens)),
    ))
}

/// Full breakdown, as in the spec's example block.
pub fn render(l: &Localizer, b: &Breakdown) -> String {
    let total = b.total();
    let mut out = l.t_args(
        "context-head",
        &args!(
            "used" => human(total),
            "window" => human(b.window),
            "percent" => b.percent(total),
            "model" => b.model.clone()
        ),
    );
    out.push('\n');
    let rows: Vec<(String, u64)> = b
        .parts()
        .iter()
        .filter(|(_, t)| *t > 0)
        .map(|(k, t)| (category(l, k), *t))
        .collect();
    let width = rows
        .iter()
        .map(|(n, _)| n.chars().count())
        .max()
        .unwrap_or(0);
    for (name, t) in rows {
        let pad = " ".repeat(width - name.chars().count());
        let _ = writeln!(out, "  {name}{pad}  ≈{:>5}", human(t));
    }
    if let Some(w) = warning(l, b) {
        out.push_str(&w);
        out.push('\n');
    }
    out
}

/// `kor context [--history N]`: last breakdown from the local log, thresholds, recent totals.
pub fn report(l: &Localizer, cfg: &kor_config::ContextBudget, history: usize) -> String {
    let log = crate::context_log_path();
    let entries = log
        .as_deref()
        .map(|p| kor_agent::context::read_log(p, history.max(1)))
        .unwrap_or_default();
    let mut out = match entries.last() {
        Some(b) => render(l, b),
        None => format!("{}\n", l.t("context-none")),
    };
    let auto = if cfg.auto_summarize_percent == 0 {
        l.t("context-off")
    } else {
        format!("{}%", cfg.auto_summarize_percent)
    };
    let model = if cfg.summarize_model.trim().is_empty() {
        l.t("context-chat-model")
    } else {
        cfg.summarize_model.clone()
    };
    let _ = writeln!(
        out,
        "{}",
        l.t_args(
            "context-thresholds",
            &args!(
                "warn" => cfg.warn_percent,
                "auto" => auto,
                "model" => model,
                "map" => cfg.repo_map_tokens
            )
        )
    );
    if history > 1 {
        for b in &entries {
            let total = b.total();
            let _ = writeln!(
                out,
                "  ≈{:>6} / {:<6} {:>3}%  {}",
                human(total),
                human(b.window),
                b.percent(total),
                b.model
            );
        }
    }
    if let Some(p) = log {
        let _ = writeln!(out, "{}", kor_tools::display_path(&p));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_numbers() {
        assert_eq!(human(999), "999");
        assert_eq!(human(45_200), "45k");
        assert_eq!(human(187_600), "188k");
        assert_eq!(human(1_048_576), "1.0M");
    }

    #[test]
    fn renders_categories_and_the_warning() {
        let l = Localizer::new(kor_i18n::Lang::En);
        let b = Breakdown {
            model: "m".into(),
            window: 200_000,
            estimated: true,
            history: 89_000,
            rules: 30_000,
            warning: Some("history".into()),
            ..Breakdown::default()
        };
        let text = render(&l, &b);
        assert!(text.contains("≈119k / 200k (59%)"), "{text}");
        assert!(
            text.contains("Conversation history") && text.contains("≈  89k"),
            "{text}"
        );
        assert!(
            !text.contains("Repo map"),
            "empty categories are hidden: {text}"
        );
        assert!(
            text.contains("44% of the context window") && text.contains("/compact"),
            "{text}"
        );
        assert!(status(&l, &b).contains("≈119k / 200k"));
    }
}
