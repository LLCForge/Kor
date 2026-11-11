//! The typed IPC contract between the core and its surfaces (spec §16.3, ADR 0061).
//!
//! `schema/ipc.json` (JSON Schema 2020-12) lists every desktop bridge method (`params`, `result`),
//! every `kor:*` event payload and the agent event stream (`$defs/AgentEvent`). The desktop's
//! TypeScript types are generated from it (`frontend/scripts/ipc-gen.ts`); the Rust side is checked
//! against it by tests and, at run time, by [`crate::ipc_trace`]. The validator below covers the
//! keywords the schema uses (`type`, `properties`, `required`, `additionalProperties`, `items`,
//! `prefixItems`, `minItems`, `maxItems`, `enum`, `const`, `anyOf`, `oneOf`, `allOf`, `$ref`,
//! `propertyNames`); `x-ts` nodes (TS-only shapes such as callbacks) accept any value.

use std::sync::OnceLock;

use serde_json::Value;

/// The contract, embedded so every binary validates against the schema it was built with.
pub const SCHEMA: &str = include_str!("../schema/ipc.json");

fn schema() -> &'static Value {
    static S: OnceLock<Value> = OnceLock::new();
    // The file is checked in and parsed by this crate's tests; an unparsable schema is a build bug.
    S.get_or_init(|| serde_json::from_str(SCHEMA).unwrap_or(Value::Null))
}

fn keys(section: &str) -> Vec<String> {
    schema()[section]
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}

/// Every bridge method: `ns.method` or a root name (`send`).
pub fn methods() -> Vec<String> {
    keys("methods")
}

/// Every event name: `kor:event`, `kor:<ns>/<event>`.
pub fn events() -> Vec<String> {
    keys("events")
}

/// Arguments of a call (positional, as `kor_call` receives them). Empty = valid.
pub fn check_call(method: &str, args: &[Value]) -> Vec<String> {
    let Some(m) = schema()["methods"].get(method) else {
        return vec![format!("unknown method `{method}`")];
    };
    let params = m["params"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut errors = Vec::new();
    if args.len() > params.len() {
        errors.push(format!(
            "{method}: {} arguments, at most {}",
            args.len(),
            params.len()
        ));
    }
    for (i, p) in params.iter().enumerate() {
        let name = p["name"].as_str().unwrap_or("?");
        match args.get(i) {
            // Optional parameters may be left out or passed as null (JS `undefined` → null).
            None | Some(Value::Null) if p["optional"] == true => {}
            None => errors.push(format!("{method}: missing argument `{name}`")),
            Some(v) => check(
                &p["schema"],
                v,
                &format!("{method}({name})"),
                &mut errors,
                0,
            ),
        }
    }
    errors
}

/// The value a call resolved with.
pub fn check_result(method: &str, result: &Value) -> Vec<String> {
    match schema()["methods"].get(method) {
        Some(m) => collect(&m["result"], result, &format!("{method} → result")),
        None => vec![format!("unknown method `{method}`")],
    }
}

/// An event payload.
pub fn check_event(name: &str, payload: &Value) -> Vec<String> {
    match schema()["events"].get(name) {
        // Events without a payload carry null.
        Some(e) => match e.get("payload") {
            Some(s) => collect(s, payload, name),
            None if payload.is_null() => Vec::new(),
            None => vec![format!("{name}: carries no payload")],
        },
        None => vec![format!("unknown event `{name}`")],
    }
}

/// A value against one `$defs` entry (`AgentEventBody`, `CostReport`…).
pub fn check_def(def: &str, value: &Value) -> Vec<String> {
    match schema()["$defs"].get(def) {
        Some(s) => collect(s, value, def),
        None => vec![format!("unknown definition `{def}`")],
    }
}

fn collect(s: &Value, v: &Value, at: &str) -> Vec<String> {
    let mut errors = Vec::new();
    check(s, v, at, &mut errors, 0);
    errors
}

fn type_ok(t: &str, v: &Value) -> bool {
    match t {
        "string" => v.is_string(),
        "number" => v.is_number(),
        "integer" => v.as_f64().is_some_and(|n| n.fract() == 0.0),
        "boolean" => v.is_boolean(),
        "null" => v.is_null(),
        "array" => v.is_array(),
        "object" => v.is_object(),
        _ => false,
    }
}

fn ok(s: &Value, v: &Value, depth: usize) -> bool {
    let mut e = Vec::new();
    check(s, v, "", &mut e, depth);
    e.is_empty()
}

/// `$ref` chains and recursive types are bounded; deeper values are accepted, not looped on.
const MAX_DEPTH: usize = 64;

#[allow(clippy::too_many_lines)] // one block per JSON Schema keyword
fn check(s: &Value, v: &Value, at: &str, errors: &mut Vec<String>, depth: usize) {
    if depth > MAX_DEPTH || !s.is_object() || s.get("x-ts").is_some() {
        return;
    }
    if let Some(r) = s["$ref"].as_str() {
        match r
            .strip_prefix("#/$defs/")
            .and_then(|n| schema()["$defs"].get(n))
        {
            Some(d) => check(d, v, at, errors, depth + 1),
            None => errors.push(format!("{at}: unresolved $ref {r}")),
        }
    }
    if let Some(t) = s.get("type") {
        let fits = match t {
            Value::String(t) => type_ok(t, v),
            Value::Array(ts) => ts.iter().filter_map(Value::as_str).any(|t| type_ok(t, v)),
            _ => true,
        };
        if !fits {
            errors.push(format!("{at}: expected {t}, got {}", kind(v)));
            return;
        }
    }
    if let Some(c) = s.get("const")
        && c != v
    {
        errors.push(format!("{at}: expected {c}"));
    }
    if let Some(e) = s["enum"].as_array()
        && !e.contains(v)
    {
        errors.push(format!(
            "{at}: {v} is not one of {}",
            Value::Array(e.clone())
        ));
    }
    for key in ["anyOf", "oneOf"] {
        if let Some(alts) = s[key].as_array()
            && !alts.iter().any(|a| ok(a, v, depth + 1))
        {
            // Name the closest alternative: the tagged variant (`type` const) when there is one.
            let tagged = alts.iter().find(|a| tag_matches(a, v, depth + 1));
            match tagged {
                Some(a) => check(a, v, at, errors, depth + 1),
                None => errors.push(format!("{at}: matches none of {} alternatives", alts.len())),
            }
        }
    }
    if let Some(all) = s["allOf"].as_array() {
        for a in all {
            check(a, v, at, errors, depth + 1);
        }
    }
    if let Value::Object(map) = v {
        let props = s["properties"].as_object();
        for r in s["required"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if !map.contains_key(r) {
                errors.push(format!("{at}: missing `{r}`"));
            }
        }
        for (k, val) in map {
            match props.and_then(|p| p.get(k)) {
                Some(ps) => check(ps, val, &format!("{at}.{k}"), errors, depth + 1),
                None => match s.get("additionalProperties") {
                    Some(Value::Bool(false)) => errors.push(format!("{at}: unexpected `{k}`")),
                    Some(ap @ Value::Object(_)) => {
                        check(ap, val, &format!("{at}.{k}"), errors, depth + 1);
                    }
                    _ => {}
                },
            }
            if let Some(pn) = s.get("propertyNames") {
                check(
                    pn,
                    &Value::String(k.clone()),
                    &format!("{at} key"),
                    errors,
                    depth + 1,
                );
            }
        }
    }
    if let Value::Array(items) = v {
        let prefix = s["prefixItems"].as_array();
        if let Some(min) = s["minItems"].as_u64()
            && (items.len() as u64) < min
        {
            errors.push(format!("{at}: at least {min} items"));
        }
        if let Some(max) = s["maxItems"].as_u64()
            && (items.len() as u64) > max
        {
            errors.push(format!("{at}: at most {max} items"));
        }
        for (i, item) in items.iter().enumerate() {
            let sub = prefix.and_then(|p| p.get(i)).or_else(|| s.get("items"));
            if let Some(sub) = sub {
                check(sub, item, &format!("{at}[{i}]"), errors, depth + 1);
            }
        }
    }
}

/// A variant whose `type` const equals the value's `type` (directly, in `allOf`, or behind `$ref`).
fn tag_matches(s: &Value, v: &Value, depth: usize) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    let Some(tag) = v.get("type") else {
        return false;
    };
    if s["properties"]["type"]["const"] == *tag {
        return true;
    }
    if let Some(r) = s["$ref"].as_str().and_then(|r| r.strip_prefix("#/$defs/"))
        && let Some(d) = schema()["$defs"].get(r)
    {
        return tag_matches(d, v, depth + 1);
    }
    s["allOf"]
        .as_array()
        .is_some_and(|a| a.iter().any(|x| tag_matches(x, v, depth + 1)))
}

fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn schema_parses_and_lists_the_surface() {
        let v: Value = serde_json::from_str(SCHEMA).expect("ipc.json");
        assert_eq!(v["$schema"], "https://json-schema.org/draft/2020-12/schema");
        assert!(methods().len() > 500, "{}", methods().len());
        assert!(methods().iter().any(|m| m == "cost.summary"));
        assert!(events().iter().any(|e| e == "kor:event"));
    }

    #[test]
    fn every_ref_resolves() {
        fn walk(n: &Value, bad: &mut Vec<String>) {
            match n {
                Value::Object(m) => {
                    if let Some(r) = m.get("$ref").and_then(Value::as_str)
                        && r.strip_prefix("#/$defs/")
                            .and_then(|d| schema()["$defs"].get(d))
                            .is_none()
                    {
                        bad.push(r.to_owned());
                    }
                    m.values().for_each(|v| walk(v, bad));
                }
                Value::Array(a) => a.iter().for_each(|v| walk(v, bad)),
                _ => {}
            }
        }
        let mut bad = Vec::new();
        walk(schema(), &mut bad);
        assert!(bad.is_empty(), "{bad:?}");
    }

    #[test]
    fn agent_events_validate() {
        let ok = json!({ "type": "status", "verb": "Közlüyor", "verbIndex": 1, "elapsedMs": 900, "tokens": 12, "sessionId": "s" });
        assert!(
            check_event("kor:event", &ok).is_empty(),
            "{:?}",
            check_event("kor:event", &ok)
        );
        assert!(
            check_event(
                "kor:event",
                &json!({ "type": "text", "delta": "a", "sessionId": "s" })
            )
            .is_empty()
        );
        let bad = json!({ "type": "text", "delta": 3, "sessionId": "s" });
        let e = check_event("kor:event", &bad);
        assert!(e.iter().any(|m| m.contains("delta")), "{e:?}");
        assert!(!check_event("kor:event", &json!({ "type": "nope", "sessionId": "s" })).is_empty());
        assert_eq!(
            check_event("kor:nope/x", &Value::Null),
            ["unknown event `kor:nope/x`"]
        );
    }

    #[test]
    fn calls_and_results() {
        assert!(check_call("cost.summary", &[json!(7)]).is_empty());
        assert!(
            check_call("cost.summary", &[]).is_empty(),
            "days is optional"
        );
        assert!(!check_call("cost.summary", &[json!("seven")]).is_empty());
        assert!(!check_call("cost.summary", &[json!(1), json!(2), json!(3)]).is_empty());
        assert!(!check_call("nope.x", &[]).is_empty());
        assert!(check_result("provider.remove", &json!(true)).is_empty());
        assert!(!check_result("provider.remove", &json!("yes")).is_empty());
    }

    #[test]
    fn defs_check_required_tuples_and_enums() {
        let row = json!({ "slot": 1, "masked": "…abcd" });
        assert!(check_result("provider.keys", &json!([row])).is_empty());
        assert!(!check_result("provider.keys", &json!([{ "slot": 1 }])).is_empty());
        assert!(check_def("PlanInfo", &json!({
            "id": "p", "title": "t", "status": "draft", "session": null, "created": 1, "body": "", "path": "x", "files": []
        })).is_empty());
        assert!(!check_def("PlanInfo", &json!({
            "id": "p", "title": "t", "status": "other", "session": null, "created": 1, "body": "", "path": "x", "files": []
        })).is_empty());
        assert_eq!(
            check_def("Nope", &Value::Null),
            ["unknown definition `Nope`"]
        );
    }
}
