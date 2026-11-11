//! Louvain community detection, warm-started from the previous partition so community ids stay
//! stable across incremental updates, plus LLM-free labels (dominant directory + top names).

use crate::model::Edge;
use crate::model::{Community, EdgeKind, GraphNode, NodeId, NodeKind};
use crate::resolve::{EdgeKey, dir_of};
use std::collections::{BTreeMap, HashMap, HashSet};

const MAX_SWEEPS: usize = 20;
const MAX_LEVELS: usize = 10;

fn weight(kind: EdgeKind) -> f64 {
    match kind {
        EdgeKind::Calls | EdgeKind::Contains | EdgeKind::Implements | EdgeKind::Extends => 1.0,
        EdgeKind::CoChanges { ratio } => f64::from(ratio),
        EdgeKind::ReadsEnv => 0.3,
        _ => 0.5,
    }
}

/// One Louvain level graph: symmetric adjacency (no self entries) plus self-loop weights.
struct Level {
    adj: Vec<Vec<(usize, f64)>>,
    selfw: Vec<f64>,
}

impl Level {
    fn degrees(&self) -> Vec<f64> {
        self.adj
            .iter()
            .zip(&self.selfw)
            .map(|(a, s)| a.iter().map(|x| x.1).sum::<f64>() + 2.0 * s)
            .collect()
    }

    /// Local moving phase; returns true if any node changed community.
    fn local_move(&self, comm: &mut [usize]) -> bool {
        let k = self.degrees();
        let m2: f64 = k.iter().sum();
        if m2 <= 0.0 {
            return false;
        }
        let mut tot = vec![0.0; self.adj.len()];
        for (i, c) in comm.iter().enumerate() {
            tot[*c] += k[i];
        }
        let mut moved_any = false;
        let mut w_to: HashMap<usize, f64> = HashMap::new();
        for _ in 0..MAX_SWEEPS {
            let mut moved = false;
            for i in 0..self.adj.len() {
                let ci = comm[i];
                w_to.clear();
                for (j, w) in &self.adj[i] {
                    *w_to.entry(comm[*j]).or_default() += w;
                }
                tot[ci] -= k[i];
                let gain = |c: usize, w: f64| w - tot[c] * k[i] / m2;
                let mut best = (ci, gain(ci, w_to.get(&ci).copied().unwrap_or(0.0)));
                for (c, w) in &w_to {
                    let g = gain(*c, *w);
                    if g > best.1 + 1e-12
                        || ((g - best.1).abs() <= 1e-12 && *c < best.0 && best.0 != ci)
                    {
                        best = (*c, g);
                    }
                }
                tot[best.0] += k[i];
                if best.0 != ci {
                    comm[i] = best.0;
                    moved = true;
                    moved_any = true;
                }
            }
            if !moved {
                break;
            }
        }
        moved_any
    }

    fn aggregate(&self, comm: &[usize], n_comm: usize) -> Self {
        let mut adj: Vec<HashMap<usize, f64>> = vec![HashMap::new(); n_comm];
        let mut selfw = vec![0.0; n_comm];
        for (i, list) in self.adj.iter().enumerate() {
            selfw[comm[i]] += self.selfw[i];
            for (j, w) in list {
                if comm[i] == comm[*j] {
                    selfw[comm[i]] += w / 2.0;
                } else {
                    *adj[comm[i]].entry(comm[*j]).or_default() += w;
                }
            }
        }
        Self {
            adj: adj.into_iter().map(|m| m.into_iter().collect()).collect(),
            selfw,
        }
    }
}

fn compact(labels: &mut [usize]) -> usize {
    let mut map: HashMap<usize, usize> = HashMap::new();
    for l in labels.iter_mut() {
        let n = map.len();
        *l = *map.entry(*l).or_insert(n);
    }
    map.len()
}

/// Multi-level Louvain. `init` is the starting partition of level 0 (previous communities).
fn louvain(level0: Level, init: Vec<usize>) -> Vec<usize> {
    let mut membership = init;
    compact(&mut membership);
    let mut level = level0;
    let mut comm = membership.clone();
    for depth in 0..MAX_LEVELS {
        let moved = level.local_move(&mut comm);
        let n = compact(&mut comm);
        if depth == 0 {
            membership.clone_from(&comm);
        } else {
            for m in &mut membership {
                *m = comm[*m];
            }
        }
        if !moved {
            break;
        }
        level = level.aggregate(&comm, n);
        comm = (0..n).collect();
    }
    membership
}

#[allow(clippy::implicit_hasher)]
/// Assign `community` on every node and return the labelled communities, largest first.
pub fn detect(
    nodes: &mut BTreeMap<NodeId, GraphNode>,
    edges: &BTreeMap<EdgeKey, Edge>,
    prev: &HashMap<NodeId, u16>,
) -> Vec<Community> {
    let ids: Vec<&NodeId> = nodes.keys().collect();
    let pos: HashMap<&str, usize> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();
    let mut adj: Vec<HashMap<usize, f64>> = vec![HashMap::new(); ids.len()];
    for e in edges.values() {
        let (Some(&a), Some(&b)) = (pos.get(e.source.as_str()), pos.get(e.target.as_str())) else {
            continue;
        };
        if a != b {
            let w = weight(e.kind);
            *adj[a].entry(b).or_default() += w;
            *adj[b].entry(a).or_default() += w;
        }
    }
    let n = ids.len();
    let init: Vec<usize> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| prev.get(id.as_str()).map_or(n + i, |c| usize::from(*c)))
        .collect();
    let level = Level {
        adj: adj.into_iter().map(|m| m.into_iter().collect()).collect(),
        selfw: vec![0.0; n],
    };
    let membership = louvain(level, init);

    // Stable ids: a new community keeps the previous id most of its members had.
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, c) in membership.iter().enumerate() {
        groups.entry(*c).or_default().push(i);
    }
    let mut order: Vec<(usize, Vec<usize>)> = groups.into_iter().collect();
    order.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.1[0].cmp(&b.1[0])));
    let mut used: HashSet<u16> = HashSet::new();
    let mut assigned: Vec<Option<u16>> = vec![None; order.len()];
    for (k, (_, members)) in order.iter().enumerate() {
        let mut votes: HashMap<u16, usize> = HashMap::new();
        for m in members {
            if let Some(p) = prev.get(ids[*m].as_str()) {
                *votes.entry(*p).or_default() += 1;
            }
        }
        let mut votes: Vec<(u16, usize)> = votes.into_iter().collect();
        votes.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        if let Some((id, _)) = votes.into_iter().find(|(id, _)| !used.contains(id)) {
            used.insert(id);
            assigned[k] = Some(id);
        }
    }
    let mut free = (0..=u16::MAX).filter(|i| !used.contains(i));
    let final_ids: Vec<u16> = assigned
        .iter()
        .map(|a| a.or_else(|| free.next()).unwrap_or(u16::MAX))
        .collect();

    let mut out = Vec::with_capacity(order.len());
    let owned_ids: Vec<NodeId> = ids.iter().map(|s| (*s).clone()).collect();
    for ((_, members), cid) in order.iter().zip(final_ids) {
        for m in members {
            if let Some(node) = nodes.get_mut(&owned_ids[*m]) {
                node.community = cid;
            }
        }
        let label = label(members.iter().filter_map(|m| nodes.get(&owned_ids[*m])));
        out.push(Community {
            id: cid,
            label,
            size: u32::try_from(members.len()).unwrap_or(u32::MAX),
        });
    }
    out
}

fn label<'a>(members: impl Iterator<Item = &'a GraphNode>) -> String {
    let mut dirs: HashMap<String, usize> = HashMap::new();
    let mut symbols: Vec<&GraphNode> = Vec::new();
    let mut any: Option<&GraphNode> = None;
    for n in members {
        if let Some(f) = &n.file {
            let f = f.to_string_lossy();
            *dirs.entry(dir_of(&f).to_owned()).or_default() += 1;
        }
        if matches!(
            n.kind,
            NodeKind::Function | NodeKind::Struct | NodeKind::Trait
        ) {
            symbols.push(n);
        }
        if any.is_none_or(|a| n.degree > a.degree) {
            any = Some(n);
        }
    }
    let dir = dirs
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|d| if d.0.is_empty() { ".".to_owned() } else { d.0 });
    symbols.sort_by(|a, b| b.degree.cmp(&a.degree).then(a.name.cmp(&b.name)));
    symbols.dedup_by(|a, b| a.name == b.name);
    let mut names: Vec<&str> = symbols.iter().take(3).map(|n| n.name.as_str()).collect();
    if names.is_empty() {
        names.extend(any.map(|n| n.name.as_str()));
    }
    match dir {
        Some(d) => format!("{d}: {}", names.join(", ")),
        None => names.join(", "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Confidence;
    use std::path::PathBuf;

    fn node(id: &str, file: &str) -> GraphNode {
        GraphNode {
            id: id.into(),
            kind: NodeKind::Function,
            name: id.into(),
            file: Some(PathBuf::from(file)),
            span: None,
            community: 0,
            degree: 1,
            churn_30d: 0,
            text: String::new(),
        }
    }

    fn graph() -> (BTreeMap<NodeId, GraphNode>, BTreeMap<EdgeKey, Edge>) {
        let mut nodes = BTreeMap::new();
        let mut edges = BTreeMap::new();
        // Two 4-cliques joined by one edge.
        for (grp, dir) in [("a", "src/a"), ("b", "src/b")] {
            for i in 0..4 {
                let id = format!("{grp}{i}");
                nodes.insert(id.clone(), node(&id, &format!("{dir}/x.rs")));
                for j in 0..i {
                    let other = format!("{grp}{j}");
                    edges.insert(
                        (id.clone(), other.clone(), "calls"),
                        Edge {
                            source: id.clone(),
                            target: other,
                            kind: EdgeKind::Calls,
                            confidence: Confidence::Extracted,
                        },
                    );
                }
            }
        }
        edges.insert(
            ("a0".into(), "b0".into(), "calls"),
            Edge {
                source: "a0".into(),
                target: "b0".into(),
                kind: EdgeKind::Calls,
                confidence: Confidence::Extracted,
            },
        );
        (nodes, edges)
    }

    #[test]
    fn finds_two_communities_and_keeps_ids() {
        let (mut nodes, edges) = graph();
        let comms = detect(&mut nodes, &edges, &HashMap::new());
        assert_eq!(comms.len(), 2);
        assert_eq!(nodes["a1"].community, nodes["a3"].community);
        assert_ne!(nodes["a1"].community, nodes["b1"].community);
        assert!(comms.iter().any(|c| c.label.starts_with("src/a: ")));
        // Warm start with swapped ids keeps them.
        let prev: HashMap<NodeId, u16> = nodes
            .iter()
            .map(|(k, n)| (k.clone(), if n.community == 0 { 7 } else { 9 }))
            .collect();
        let mut nodes2 = nodes.clone();
        nodes2.insert("a4".into(), node("a4", "src/a/x.rs"));
        let mut edges2 = edges.clone();
        edges2.insert(
            ("a4".into(), "a1".into(), "calls"),
            Edge {
                source: "a4".into(),
                target: "a1".into(),
                kind: EdgeKind::Calls,
                confidence: Confidence::Extracted,
            },
        );
        detect(&mut nodes2, &edges2, &prev);
        assert_eq!(nodes2["a1"].community, prev["a1"]);
        assert_eq!(nodes2["b2"].community, prev["b2"]);
        assert_eq!(nodes2["a4"].community, prev["a1"]);
    }
}
