//! `kor notifications [list|read|clear]` (§27.2, ADR 0051): the same store as the desktop bell.

use kor_i18n::{Localizer, args};
use kor_runtime::presence::notifications::{Category, Notification, Store};

use crate::Ctx;

#[derive(Debug, clap::Subcommand)]
pub enum NotifyCmd {
    /// Newest first (the default).
    List {
        /// Only unread ones.
        #[arg(long)]
        unread: bool,
        /// Only this category (approval, question, done, error, budget, update, health, component, quota).
        #[arg(long)]
        category: Option<String>,
        #[arg(short = 'n', long, default_value_t = 30)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// Mark notifications read (all when no id is given).
    Read { ids: Vec<String> },
    /// Remove notifications (all when no id is given).
    Clear { ids: Vec<String> },
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-notifications"))
        .mut_subcommand("list", |s| s.about(l.t("cmd-notifications-list")))
        .mut_subcommand("read", |s| s.about(l.t("cmd-notifications-read")))
        .mut_subcommand("clear", |s| s.about(l.t("cmd-notifications-clear")))
}

fn store() -> anyhow::Result<Store> {
    Store::standard().ok_or_else(|| anyhow::anyhow!("no Kor data folder"))
}

fn line(l: &Localizer, n: &Notification) -> String {
    let when = kor_runtime::cost::fmt_time(n.at);
    let mut s = format!(
        "{} {}  {}  [{}] {}",
        if n.read { " " } else { "•" },
        n.id,
        when,
        l.t(&format!("notify-cat-{}", n.category.key())),
        n.title
    );
    if let Some(c) = n.cost_usd {
        s.push_str("  ");
        s.push_str(&kor_runtime::cost::fmt_usd(c, n.cost_estimated));
    }
    if !n.body.is_empty() {
        s.push_str("\n      ");
        s.push_str(n.body.lines().next().unwrap_or_default());
    }
    if let Some(a) = &n.approval {
        s.push_str("\n      kor approve ");
        s.push_str(a);
    }
    s
}

pub fn run(ctx: &Ctx, cmd: Option<NotifyCmd>) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let st = store()?;
    let ids = |v: &[String]| (!v.is_empty()).then(|| v.to_vec());
    match cmd.unwrap_or(NotifyCmd::List {
        unread: false,
        category: None,
        limit: 30,
        json: false,
    }) {
        NotifyCmd::List {
            unread,
            category,
            limit,
            json,
        } => {
            let cat = match category.as_deref() {
                None => None,
                Some(c) => Some(Category::parse(c).ok_or_else(|| {
                    anyhow::anyhow!(l.t_args("notify-bad-category", &args!("name" => c.to_owned())))
                })?),
            };
            let list: Vec<Notification> = st
                .list()
                .into_iter()
                .filter(|n| !unread || !n.read)
                .filter(|n| cat.is_none_or(|c| n.category == c))
                .take(limit)
                .collect();
            if json {
                println!("{}", serde_json::to_string_pretty(&list)?);
            } else if list.is_empty() {
                println!("{}", l.t("notifications-none"));
            } else {
                println!(
                    "{}",
                    l.t_args(
                        "notifications-unread",
                        &args!("n" => i64::try_from(st.unread()).unwrap_or(0))
                    )
                );
                for n in &list {
                    println!("{}", line(l, n));
                }
            }
        }
        NotifyCmd::Read { ids: v } => {
            let n = st
                .mark_read(ids(&v).as_deref())
                .map_err(anyhow::Error::msg)?;
            println!(
                "{}",
                l.t_args(
                    "notifications-read",
                    &args!("n" => i64::try_from(n).unwrap_or(0))
                )
            );
        }
        NotifyCmd::Clear { ids: v } => {
            let n = st.clear(ids(&v).as_deref()).map_err(anyhow::Error::msg)?;
            println!(
                "{}",
                l.t_args(
                    "notifications-cleared",
                    &args!("n" => i64::try_from(n).unwrap_or(0))
                )
            );
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Debug, clap::Parser)]
    struct T {
        #[command(subcommand)]
        cmd: Option<NotifyCmd>,
    }

    #[test]
    fn parses() {
        assert!(T::try_parse_from(["kor"]).unwrap().cmd.is_none());
        assert!(matches!(
            T::try_parse_from(["kor", "list", "--unread", "--category", "budget", "-n", "5"])
                .unwrap()
                .cmd,
            Some(NotifyCmd::List {
                unread: true,
                limit: 5,
                ..
            })
        ));
        assert!(matches!(
            T::try_parse_from(["kor", "clear"]).unwrap().cmd,
            Some(NotifyCmd::Clear { ids }) if ids.is_empty()
        ));
        assert!(matches!(
            T::try_parse_from(["kor", "read", "n1", "n2"]).unwrap().cmd,
            Some(NotifyCmd::Read { ids }) if ids.len() == 2
        ));
        let l = Localizer::new(kor_i18n::Lang::En);
        let _ = localize(<T as clap::CommandFactory>::command(), &l);
    }

    #[test]
    fn every_category_has_a_label() {
        for lang in [kor_i18n::Lang::En, kor_i18n::Lang::Tr, kor_i18n::Lang::Fr] {
            let l = Localizer::new(lang);
            for c in Category::ALL {
                let key = format!("notify-cat-{}", c.key());
                assert_ne!(l.t(&key), key, "{lang:?} {key}");
            }
        }
    }
}
