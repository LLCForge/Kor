//! SEO alerts (spec §29.6 anomalies, §29.10 competitor summary, §29.15 watch mode) through Kor's
//! notification channels (`[notify]`: Telegram, Slack, e-mail; ADR 0039). Nothing is sent in
//! privacy/offline mode (the daemon's notifier builder refuses).

use kor_runtime::triggers::{daemon, notify};

/// Send `title` / `text` to every configured channel; returns per-channel errors and notes.
pub async fn alert(
    l: &kor_i18n::Localizer,
    loaded: &kor_config::Loaded,
    title: &str,
    text: &str,
) -> Vec<String> {
    let mut notes = Vec::new();
    let secrets = daemon::vault_secrets(&loaded.config.vault);
    let ns = daemon::notifiers(l, &loaded.config.notify, &secrets, &mut notes);
    if ns.is_empty() {
        notes.push(
            "no notification channel configured ([notify] telegram_chat / slack / email_to)".into(),
        );
        return notes;
    }
    let note = notify::Note {
        title: title.to_owned(),
        text: text.to_owned(),
        ask: None,
    };
    notes.extend(notify::broadcast(&ns, &note).await);
    notes
}

/// Overall click/impression change between two periods of page rows, in percent (`None` when the
/// first period is empty). An anomaly when clicks fell by at least `threshold` percent.
pub fn anomaly(
    before: &[super::google::Row],
    now: &[super::google::Row],
    threshold: f64,
) -> Option<(f64, f64)> {
    let sum =
        |r: &[super::google::Row], f: fn(&super::google::Row) -> f64| r.iter().map(f).sum::<f64>();
    let (c0, c1) = (sum(before, |r| r.clicks), sum(now, |r| r.clicks));
    let (i0, i1) = (sum(before, |r| r.impressions), sum(now, |r| r.impressions));
    if c0 <= 0.0 {
        return None;
    }
    let dc = (c1 - c0) * 100.0 / c0;
    let di = if i0 > 0.0 {
        (i1 - i0) * 100.0 / i0
    } else {
        0.0
    };
    (dc <= -threshold).then_some((dc, di))
}

#[cfg(test)]
mod tests {
    use super::super::google::Row;
    use super::*;

    #[test]
    fn anomaly_threshold() {
        let r = |c: f64, i: f64| Row {
            clicks: c,
            impressions: i,
            ..Row::default()
        };
        assert_eq!(
            anomaly(&[r(100.0, 1000.0)], &[r(70.0, 900.0)], 20.0),
            Some((-30.0, -10.0))
        );
        assert_eq!(anomaly(&[r(100.0, 1000.0)], &[r(90.0, 900.0)], 20.0), None);
        assert_eq!(anomaly(&[], &[r(1.0, 1.0)], 20.0), None);
    }
}
