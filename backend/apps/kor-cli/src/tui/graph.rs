//! The TUI's graph panel (§15.18, Faz 3): the selected node's card (source line, community,
//! degree, callers / callees, churn, co-change), the hotspot list or a `/gfind` search, and the
//! files the agent touched this session marked with `●`. Reads the agent session's live graph
//! (`kor_agent::graph`) without ever waiting for its first build.

use std::collections::BTreeSet;

use kor_agent::graph::Session;
use kor_graph::{Graph, Link, NodeRef};
use kor_i18n::{Localizer, args};

use super::Screen;

const ITEMS: usize = 30;

/// Refresh `screen.graph` from the session (or explain why it is empty).
pub fn fill(screen: &mut Screen, l: &Localizer, session: Option<&Session>) {
    let Some(s) = session else {
        screen.graph = vec![l.t("tui-graph-off")];
        screen.graph_items = 0;
        return;
    };
    let Some(g) = s.ready() else {
        screen.graph = vec![l.t("tui-graph-building")];
        screen.graph_items = 0;
        return;
    };
    let touched: BTreeSet<String> = s.touched().into_iter().collect();
    let g = g.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let (lines, n) = lines(l, &g, &touched, &screen.graph_query, screen.graph_sel);
    screen.graph = lines;
    screen.graph_items = n;
    screen.graph_sel = screen.graph_sel.min(n.saturating_sub(1));
}

fn short(n: &NodeRef) -> String {
    match (&n.file, n.line) {
        (Some(f), _) if f == &n.name => format!("{} {}", n.kind.as_str(), n.name),
        (Some(f), Some(line)) => format!("{} {}  {f}:{line}", n.kind.as_str(), n.name),
        (Some(f), None) => format!("{} {}  {f}", n.kind.as_str(), n.name),
        _ => format!("{} {}", n.kind.as_str(), n.name),
    }
}

fn names(links: &[Link]) -> String {
    let mut v: Vec<&str> = links.iter().map(|k| k.node.name.as_str()).take(5).collect();
    v.dedup();
    let more = links.len().saturating_sub(5);
    if more > 0 {
        format!("{} +{more}", v.join(", "))
    } else {
        v.join(", ")
    }
}

/// Panel lines and the number of selectable rows. Rows start with `▸` when selected and carry
/// `●` when the agent touched their file this session.
pub fn lines(
    l: &Localizer,
    g: &Graph,
    touched: &BTreeSet<String>,
    query: &str,
    selected: usize,
) -> (Vec<String>, usize) {
    let items: Vec<NodeRef> = if query.is_empty() {
        g.hotspots(ITEMS).into_iter().map(|h| h.node).collect()
    } else {
        g.query(query, ITEMS).into_iter().map(|h| h.node).collect()
    };
    let mut out = Vec::new();
    let selected = selected.min(items.len().saturating_sub(1));
    if let Some(ex) = items.get(selected).and_then(|n| g.explain(&n.id)) {
        let n = &ex.node;
        out.push(format!("▣ {}", short(&NodeRef::from(n))));
        let community = ex.community.as_ref().map_or_else(
            || n.community.to_string(),
            |c| format!("{} {}", c.id, c.label),
        );
        out.push(l.t_args(
            "tui-graph-card",
            &args!("community" => community, "degree" => n.degree, "churn" => n.churn_30d),
        ));
        if let Some(first) = ex.source.as_deref().and_then(|s| s.lines().next()) {
            out.push(format!("  {}", first.trim()));
        }
        for (key, links) in [
            ("tui-graph-callers", &ex.callers),
            ("tui-graph-callees", &ex.callees),
            ("tui-graph-cochange", &ex.co_changes),
        ] {
            if !links.is_empty() {
                out.push(l.t_args(key, &args!("list" => names(links))));
            }
        }
    }
    out.push(if query.is_empty() {
        l.t("tui-graph-hotspots")
    } else {
        l.t_args("tui-graph-search", &args!("text" => query.to_owned()))
    });
    if items.is_empty() {
        out.push(l.t("tui-graph-empty"));
    }
    for (i, n) in items.iter().enumerate() {
        let hit = n.file.as_ref().is_some_and(|f| touched.contains(f));
        let mark = match (i == selected, hit) {
            (true, true) => "▸●",
            (true, false) => "▸ ",
            (false, true) => " ●",
            (false, false) => "  ",
        };
        out.push(format!("{mark} {}", short(n)));
    }
    if !touched.is_empty() {
        out.push(l.t_args("tui-graph-touched", &args!("count" => touched.len())));
        for f in touched.iter().take(10) {
            out.push(format!(" ● {f}"));
        }
    }
    out.push(l.t("tui-graph-help"));
    (out, items.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::keys::Action;
    use crate::tui::{Ev, Out, Panel, reduce};

    #[test]
    fn panel_lists_hotspots_marks_touched_and_searches() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("lib.rs"),
            "pub fn alpha() { beta(); gamma(); }\npub fn beta() { gamma(); }\npub fn gamma() {}\n",
        )
        .unwrap();
        let mut g = Graph::open(dir.path()).unwrap();
        g.update().unwrap();
        let l = Localizer::new(kor_i18n::Lang::En);
        let touched = BTreeSet::from(["lib.rs".to_owned()]);
        let (out, n) = lines(&l, &g, &touched, "", 0);
        assert!(n > 0);
        assert!(out[0].starts_with("▣ "), "{out:?}");
        assert!(out.iter().any(|s| s.starts_with("▸●")), "{out:?}");
        let (found, _) = lines(&l, &g, &touched, "gamma", 0);
        assert!(
            found.iter().any(|s| s.contains("function gamma")),
            "{found:?}"
        );

        // `/gfind` sets the search and focuses the panel; arrows move the selection.
        let mut s = Screen::new("default", "m");
        for ch in "/gfind c".chars() {
            reduce(&mut s, &l, Ev::Char(ch));
        }
        assert_eq!(reduce(&mut s, &l, Ev::Enter), Out::Graph);
        assert_eq!((s.graph_query.as_str(), s.focus), ("c", Some(Panel::Graph)));
        s.graph_items = 3;
        assert_eq!(reduce(&mut s, &l, Ev::Act(Action::Down)), Out::Graph);
        assert_eq!(s.graph_sel, 1);
        fill(&mut s, &l, None);
        assert_eq!(s.graph, vec![l.t("tui-graph-off")]);
    }
}
