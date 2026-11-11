//! Picture formats of live diagrams (§27.14, ADR 0063): an SVG for READMEs (`svg=<path>` on a
//! marked block) and `.excalidraw` drawings whose Kor-made shapes are regenerated while the
//! user's own shapes, positions and colours stay.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde_json::{Value, json};

use crate::diagram_sync::Model;

const W: f64 = 180.0;
const H: f64 = 40.0;
const GAP_X: f64 = 60.0;
const GAP_Y: f64 = 70.0;

/// Layer of each node: longest path from a source (cycles broken by visiting order).
fn layers(m: &Model) -> BTreeMap<String, usize> {
    let mut level: BTreeMap<String, usize> = m.nodes.keys().map(|k| (k.clone(), 0)).collect();
    // Bellman-Ford style relaxation, bounded so cycles terminate.
    for _ in 0..m.nodes.len().min(50) {
        let mut changed = false;
        for (a, b, _) in &m.edges {
            if a == b {
                continue;
            }
            let la = level.get(a).copied().unwrap_or(0);
            if let Some(lb) = level.get_mut(b)
                && *lb < la + 1
                && la + 1 < m.nodes.len()
            {
                *lb = la + 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    level
}

/// `(x, y)` of every node: one row per layer.
pub fn positions(m: &Model) -> BTreeMap<String, (f64, f64)> {
    let level = layers(m);
    let mut rows: BTreeMap<usize, Vec<&String>> = BTreeMap::new();
    for (id, l) in &level {
        rows.entry(*l).or_default().push(id);
    }
    let mut out = BTreeMap::new();
    for (l, ids) in rows {
        for (i, id) in ids.into_iter().enumerate() {
            #[allow(clippy::cast_precision_loss)] // small counts
            out.insert(
                id.clone(),
                (20.0 + i as f64 * (W + GAP_X), 20.0 + l as f64 * (H + GAP_Y)),
            );
        }
    }
    out
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// A self-contained SVG of `m` (boxes and arrows, no external fonts or scripts).
pub fn svg(m: &Model) -> String {
    let pos = positions(m);
    let max_x = pos.values().map(|p| p.0).fold(0.0_f64, f64::max) + W + 20.0;
    let max_y = pos.values().map(|p| p.1).fold(0.0_f64, f64::max) + H + 20.0;
    let mut o = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{max_x:.0}\" height=\"{max_y:.0}\" viewBox=\"0 0 {max_x:.0} {max_y:.0}\" font-family=\"sans-serif\" font-size=\"13\">\n<defs><marker id=\"a\" viewBox=\"0 0 10 10\" refX=\"10\" refY=\"5\" markerWidth=\"8\" markerHeight=\"8\" orient=\"auto\"><path d=\"M0,0 L10,5 L0,10 z\" fill=\"#555\"/></marker></defs>\n<rect width=\"100%\" height=\"100%\" fill=\"#fff\"/>\n"
    );
    for (a, b, label) in &m.edges {
        let (Some(pa), Some(pb)) = (pos.get(a), pos.get(b)) else {
            continue;
        };
        let (x1, y1) = (pa.0 + W / 2.0, pa.1 + H);
        let (x2, y2) = (pb.0 + W / 2.0, pb.1);
        let _ = writeln!(
            o,
            "<line x1=\"{x1:.0}\" y1=\"{y1:.0}\" x2=\"{x2:.0}\" y2=\"{y2:.0}\" stroke=\"#555\" marker-end=\"url(#a)\"/>"
        );
        let l = label.split_once('|').map_or(label.as_str(), |(_, n)| n);
        if !l.is_empty() {
            let _ = writeln!(
                o,
                "<text x=\"{:.0}\" y=\"{:.0}\" fill=\"#555\" font-size=\"11\">{}</text>",
                (x1 + x2) / 2.0 + 4.0,
                (y1 + y2) / 2.0,
                esc(l)
            );
        }
    }
    for (id, label) in &m.nodes {
        let Some((x, y)) = pos.get(id) else { continue };
        let _ = writeln!(
            o,
            "<rect x=\"{x:.0}\" y=\"{y:.0}\" width=\"{W:.0}\" height=\"{H:.0}\" rx=\"6\" fill=\"#f4f4f5\" stroke=\"#333\"/><text x=\"{:.0}\" y=\"{:.0}\" text-anchor=\"middle\" dominant-baseline=\"middle\">{}</text>",
            x + W / 2.0,
            y + H / 2.0,
            esc(&label.chars().take(26).collect::<String>())
        );
    }
    o.push_str("</svg>\n");
    o
}

// ---------------------------------------------------------------------------------------------
// Excalidraw
// ---------------------------------------------------------------------------------------------

/// The marker text element of an Excalidraw drawing: `kor:diagram kind=… …`.
pub fn excalidraw_marker(doc: &Value) -> Option<String> {
    doc["elements"].as_array()?.iter().find_map(|e| {
        let t = e["text"].as_str()?;
        (e["type"] == "text" && e["isDeleted"] != true)
            .then(|| {
                t.trim()
                    .strip_prefix("kor:diagram")
                    .map(|r| r.trim().to_owned())
            })
            .flatten()
    })
}

fn kor_tag(e: &Value) -> Option<&str> {
    e["customData"]["kor"].as_str()
}

/// The Kor-made structure of a drawing as sorted lines (`node:<id>|<label>`, `edge:<a>-><b>`).
pub fn excalidraw_structure(doc: &Value) -> String {
    let mut lines = BTreeSet::new();
    for e in doc["elements"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| e["isDeleted"] != true)
    {
        match kor_tag(e) {
            Some(t) if t.starts_with("label:") => {
                lines.insert(format!(
                    "node:{}|{}",
                    &t[6..],
                    e["text"].as_str().unwrap_or("")
                ));
            }
            Some(t) if t.starts_with("edge:") => {
                lines.insert(t.to_owned());
            }
            _ => {}
        }
    }
    lines.into_iter().collect::<Vec<_>>().join("\n")
}

/// The structure `m` would give, in the form of [`excalidraw_structure`].
pub fn model_structure(m: &Model) -> String {
    let mut lines = BTreeSet::new();
    for (id, l) in &m.nodes {
        lines.insert(format!("node:{id}|{l}"));
    }
    for (a, b, _) in &m.edges {
        lines.insert(format!("edge:{a}->{b}"));
    }
    lines.into_iter().collect::<Vec<_>>().join("\n")
}

fn seed(s: &str) -> u64 {
    s.bytes().fold(0x811c_9dc5_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0193)
    }) % 2_000_000_000
}

fn base(id: &str, kind: &str, x: f64, y: f64, w: f64, h: f64, tag: &str) -> Value {
    json!({
        "id": id, "type": kind, "x": x, "y": y, "width": w, "height": h, "angle": 0,
        "strokeColor": "#1e1e1e", "backgroundColor": "transparent", "fillStyle": "solid",
        "strokeWidth": 1, "strokeStyle": "solid", "roughness": 0, "opacity": 100,
        "groupIds": [], "frameId": null, "roundness": null, "seed": seed(id), "version": 1,
        "versionNonce": seed(&format!("{id}#")), "isDeleted": false, "boundElements": [],
        "updated": 1, "link": null, "locked": false, "customData": { "kor": tag },
    })
}

/// Regenerate the Kor-made shapes of `doc` from `m`: shapes of nodes that still exist keep their
/// position, size and colours (the user may have moved or restyled them); labels are updated;
/// new nodes are laid out below; removed nodes and every Kor arrow are rebuilt. Shapes without a
/// Kor tag (the user's own) are untouched.
pub fn excalidraw_apply(doc: &mut Value, m: &Model) {
    let elements = doc["elements"].as_array().cloned().unwrap_or_default();
    let mut keep: Vec<Value> = Vec::new();
    let mut rects: BTreeMap<String, Value> = BTreeMap::new();
    for e in elements {
        match kor_tag(&e).map(str::to_owned) {
            Some(t) if t.starts_with("node:") => {
                rects.insert(t[5..].to_owned(), e);
            }
            Some(_) => {} // labels and arrows are rebuilt
            None => keep.push(e),
        }
    }
    let pos = positions(m);
    let below = rects
        .values()
        .filter_map(|r| {
            r["y"]
                .as_f64()
                .map(|y| y + r["height"].as_f64().unwrap_or(H))
        })
        .fold(0.0_f64, f64::max);
    let mut out = keep;
    let mut boxes: BTreeMap<String, (f64, f64, f64, f64)> = BTreeMap::new();
    for (id, label) in &m.nodes {
        let rect_id = format!("kor-n-{id}");
        let text_id = format!("kor-t-{id}");
        let mut r = rects.remove(id).unwrap_or_else(|| {
            let (x, y) = pos.get(id).copied().unwrap_or((20.0, 20.0));
            let mut r = base(
                &rect_id,
                "rectangle",
                x,
                y + if below > 0.0 { below + GAP_Y } else { 0.0 },
                W,
                H,
                &format!("node:{id}"),
            );
            r["roundness"] = json!({ "type": 3 });
            r
        });
        let (x, y) = (
            r["x"].as_f64().unwrap_or(0.0),
            r["y"].as_f64().unwrap_or(0.0),
        );
        let (w, h) = (
            r["width"].as_f64().unwrap_or(W),
            r["height"].as_f64().unwrap_or(H),
        );
        r["boundElements"] = json!([{ "type": "text", "id": text_id }]);
        boxes.insert(id.clone(), (x, y, w, h));
        let mut t = base(
            &text_id,
            "text",
            x + 8.0,
            y + h / 2.0 - 10.0,
            w - 16.0,
            20.0,
            &format!("label:{id}"),
        );
        let o = t.as_object_mut().expect("object");
        o.insert("text".into(), json!(label));
        o.insert("originalText".into(), json!(label));
        o.insert("fontSize".into(), json!(16));
        o.insert("fontFamily".into(), json!(1));
        o.insert("textAlign".into(), json!("center"));
        o.insert("verticalAlign".into(), json!("middle"));
        o.insert("containerId".into(), json!(r["id"]));
        o.insert("lineHeight".into(), json!(1.25));
        o.insert("autoResize".into(), json!(true));
        out.push(r);
        out.push(t);
    }
    for (a, b, _) in &m.edges {
        let (Some(pa), Some(pb)) = (boxes.get(a), boxes.get(b)) else {
            continue;
        };
        let (x1, y1) = (pa.0 + pa.2 / 2.0, pa.1 + pa.3);
        let (x2, y2) = (pb.0 + pb.2 / 2.0, pb.1);
        let id = format!("kor-e-{a}-{b}");
        let mut e = base(
            &id,
            "arrow",
            x1,
            y1,
            (x2 - x1).abs(),
            (y2 - y1).abs(),
            &format!("edge:{a}->{b}"),
        );
        let o = e.as_object_mut().expect("object");
        o.insert("points".into(), json!([[0.0, 0.0], [x2 - x1, y2 - y1]]));
        o.insert(
            "startBinding".into(),
            json!({ "elementId": format!("kor-n-{a}"), "focus": 0, "gap": 4 }),
        );
        o.insert(
            "endBinding".into(),
            json!({ "elementId": format!("kor-n-{b}"), "focus": 0, "gap": 4 }),
        );
        o.insert("startArrowhead".into(), Value::Null);
        o.insert("endArrowhead".into(), json!("arrow"));
        o.insert("lastCommittedPoint".into(), Value::Null);
        out.push(e);
    }
    if !doc.is_object() {
        *doc = json!({});
    }
    let d = doc.as_object_mut().expect("object");
    d.entry("type").or_insert_with(|| json!("excalidraw"));
    d.entry("version").or_insert_with(|| json!(2));
    d.entry("source").or_insert_with(|| json!("kor"));
    d.insert("elements".into(), Value::Array(out));
    d.entry("appState")
        .or_insert_with(|| json!({ "viewBackgroundColor": "#ffffff" }));
    d.entry("files").or_insert_with(|| json!({}));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> Model {
        let mut m = Model::default();
        m.nodes.insert("a".into(), "src/a".into());
        m.nodes.insert("b".into(), "src/b".into());
        m.edges.insert(("a".into(), "b".into(), String::new()));
        m
    }

    #[test]
    fn svg_layers_boxes_and_arrows() {
        let s = svg(&model());
        assert!(s.starts_with("<svg") && s.contains(">src/a</text>") && s.contains("<line"));
        let p = positions(&model());
        assert!(p["b"].1 > p["a"].1, "b is below its dependency source");
        let mut cyc = model();
        cyc.edges.insert(("b".into(), "a".into(), String::new()));
        assert_eq!(positions(&cyc).len(), 2, "cycles terminate");
    }

    #[test]
    fn excalidraw_keeps_user_shapes_and_moved_nodes() {
        let mut doc = json!({ "type": "excalidraw", "elements": [
            { "id": "m", "type": "text", "text": "kor:diagram kind=architecture depth=2" },
            { "id": "note", "type": "rectangle", "x": 900, "y": 900, "width": 50, "height": 50 }
        ]});
        assert_eq!(
            excalidraw_marker(&doc).as_deref(),
            Some("kind=architecture depth=2")
        );
        excalidraw_apply(&mut doc, &model());
        assert_eq!(excalidraw_structure(&doc), model_structure(&model()));
        // The user moves and recolours node a; a new node c appears in the code.
        for e in doc["elements"].as_array_mut().unwrap() {
            if e["id"] == "kor-n-a" {
                e["x"] = json!(500);
                e["backgroundColor"] = json!("#ffc9c9");
            }
        }
        let mut m = model();
        m.nodes.insert("c".into(), "src/c".into());
        assert_ne!(excalidraw_structure(&doc), model_structure(&m));
        excalidraw_apply(&mut doc, &m);
        let els = doc["elements"].as_array().unwrap();
        let a = els.iter().find(|e| e["id"] == "kor-n-a").unwrap();
        assert_eq!(
            (a["x"].as_f64(), a["backgroundColor"].as_str()),
            (Some(500.0), Some("#ffc9c9"))
        );
        assert!(els.iter().any(|e| e["id"] == "note"), "user shape kept");
        assert!(els.iter().any(|e| e["id"] == "m"), "marker kept");
        assert!(els.iter().any(|e| e["id"] == "kor-n-c"));
        assert_eq!(excalidraw_structure(&doc), model_structure(&m));
    }
}
