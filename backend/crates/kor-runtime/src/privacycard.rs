//! Provider privacy cards for the CLI table (`kor providers privacy`) and the desktop model
//! picker (spec §31.2, ADR 0057). The cited data lives in `kor_providers::privacy_card`; this
//! module localizes it and applies the `privacy.require` filter in force.

use std::fmt::Write as _;

use kor_i18n::{Localizer, args};
use kor_providers::privacy_card::{self, Card, Region, Retention, Training, Zdr};
use serde::Serialize;
use serde_json::{Value, json};

/// One provider's card with its localized texts and the requirements it misses.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub card: Card,
    pub unmet: Vec<String>,
    pub text: Text,
}

/// Localized cell texts (unknown → "bilinmiyor", never a guess).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Text {
    pub training: String,
    pub retention: String,
    pub region: String,
    pub zdr: String,
    pub sub_processor: String,
    pub law: String,
}

pub fn text(l: &Localizer, c: &Card) -> Text {
    let unknown = || l.t("privcard-unknown");
    let training = match c.training {
        Training::No => l.t("privcard-training-no"),
        Training::OptOut => l.t("privcard-training-opt-out"),
        Training::Yes => l.t("privcard-training-yes"),
        Training::Unknown => unknown(),
    };
    let retention = match c.retention {
        Retention::Days(d) => l.t_args("privcard-retention-days", &args!("days" => i64::from(d))),
        Retention::Indefinite => l.t("privcard-retention-indefinite"),
        Retention::Unknown => unknown(),
    };
    let mut region = match c.region {
        Region::Eu => l.t("privcard-region-eu"),
        Region::Us => l.t("privcard-region-us"),
        Region::Tr => l.t("privcard-region-tr"),
        Region::Cn => l.t("privcard-region-cn"),
        Region::Other => l.t("privcard-region-other"),
        Region::Local => l.t("privcard-region-local"),
        Region::Unknown => unknown(),
    };
    if c.eu_residency == Some(true) {
        let _ = write!(region, " ({})", l.t("privcard-eu-option"));
    }
    let zdr = match c.zdr {
        Zdr::Required => l.t("privcard-zdr-required"),
        Zdr::NotNeeded => l.t("privcard-zdr-not-needed"),
        Zdr::Unknown => unknown(),
    };
    let sub_processor = match (&c.sub_processor, c.gateway) {
        (Some(s), _) => s.clone(),
        (None, true) => l.t("privcard-sub-routed"),
        (None, false) => "-".into(),
    };
    Text {
        training,
        retention,
        region,
        zdr,
        sub_processor,
        law: l.t(c.law_note_key()),
    }
}

fn row(l: &Localizer, card: Card) -> Row {
    let unmet = card
        .unmet(&crate::privacy_requirements())
        .into_iter()
        .map(|r| r.as_str().to_owned())
        .collect();
    Row {
        text: text(l, &card),
        card,
        unmet,
    }
}

/// Every known provider (built-in and the user's), sorted by id.
pub fn rows(l: &Localizer) -> Vec<Row> {
    let Ok(reg) = crate::registry(l, &mut Vec::new()) else {
        return Vec::new();
    };
    let mut out: Vec<Row> = reg
        .iter()
        .map(|m| row(l, privacy_card::card(m, None)))
        .collect();
    out.sort_by(|a, b| a.card.provider.cmp(&b.card.provider));
    out
}

/// The card of one `provider/model` reference; `None` for an unknown provider.
pub fn for_model(l: &Localizer, model_ref: &str) -> Option<Row> {
    let (provider, model) = model_ref.split_once('/').unwrap_or((model_ref, ""));
    let reg = crate::registry(l, &mut Vec::new()).ok()?;
    let m = reg.get(provider)?;
    Some(row(l, privacy_card::card(m, Some(model))))
}

/// Desktop payload: the requirements in force and every provider's row.
pub fn payload(l: &Localizer) -> Value {
    let req: Vec<&str> = crate::privacy_requirements()
        .iter()
        .map(|r| r.as_str())
        .collect();
    json!({ "require": req, "providers": rows(l) })
}

fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\n', '\r'], " ")
}

/// `kor providers privacy`: one Markdown table row per provider, then the sources.
pub fn table(l: &Localizer, rows: &[Row]) -> String {
    let req = crate::privacy_requirements();
    let mut out = String::new();
    if req.is_empty() {
        let _ = writeln!(out, "{}\n", l.t("privcard-require-none"));
    } else {
        let list: Vec<&str> = req.iter().map(|r| r.as_str()).collect();
        let _ = writeln!(
            out,
            "{}\n",
            l.t_args("privcard-require", &args!("list" => list.join(", ")))
        );
    }
    let cols = [
        "privcard-col-provider",
        "privcard-col-training",
        "privcard-col-retention",
        "privcard-col-region",
        "privcard-col-zdr",
        "privcard-col-sub",
        "privcard-col-status",
    ];
    let head: Vec<String> = cols.iter().map(|k| l.t(k)).collect();
    let _ = writeln!(out, "| {} |", head.join(" | "));
    let _ = writeln!(out, "|{}", "---|".repeat(cols.len()));
    for r in rows {
        let status = if req.is_empty() {
            "-".to_owned()
        } else if r.unmet.is_empty() {
            l.t("privcard-ok")
        } else {
            l.t_args("privcard-unmet", &args!("unmet" => r.unmet.join(", ")))
        };
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} | {} | {} |",
            cell(&r.card.provider),
            cell(&r.text.training),
            cell(&r.text.retention),
            cell(&r.text.region),
            cell(&r.text.zdr),
            cell(&r.text.sub_processor),
            cell(&status),
        );
    }
    let _ = writeln!(out, "\n{}", l.t("privcard-sources"));
    for r in rows.iter().filter(|r| !r.card.sources.is_empty()) {
        for s in &r.card.sources {
            let _ = writeln!(out, "- {}: {} ({})", r.card.provider, s.url, s.retrieved);
        }
        for n in &r.card.notes {
            let _ = writeln!(out, "  - {n}");
        }
    }
    let _ = writeln!(out, "\n{}", l.t("privcard-law-footer"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texts_say_unknown_and_never_guess() {
        let l = Localizer::new(kor_i18n::Lang::Tr);
        let c = privacy_card::card_for("no-such", false, None);
        let t = text(&l, &c);
        let unknown = l.t("privcard-unknown");
        assert_eq!(t.training, unknown);
        assert_eq!(t.retention, unknown);
        assert_eq!(t.region, unknown);
        assert_eq!(t.zdr, unknown);
        let a = text(&l, &privacy_card::card_for("anthropic", false, None));
        assert!(a.retention.contains("30"), "{}", a.retention);
        let rows = vec![Row {
            text: a,
            unmet: vec![],
            card: privacy_card::card_for("anthropic", false, None),
        }];
        let md = table(&l, &rows);
        assert!(md.contains("anthropic.com/legal/commercial-terms"), "{md}");
        assert!(md.contains("2026-11-10"));
    }
}
