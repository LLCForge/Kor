//! Tool input against the tool's JSON Schema (spec 15.13), with no extra dependency.
//!
//! Covers the subset tool schemas use: `type` (one or a list), `properties`, `required`,
//! `additionalProperties`, `items`, `enum`, `const`, `minLength`/`maxLength`,
//! `minimum`/`maximum`, `minItems`/`maxItems`, `anyOf`/`oneOf`/`allOf`. Other keywords
//! (`$ref`, `pattern`, `format`, ...) are not checked; the tool still validates its own input.

use serde_json::{Map, Value};

/// First violation, with a JSON path (`edits[1].old_string`), or `Ok`.
///
/// # Errors
/// A message naming the field and what it should be.
pub fn validate(value: &Value, schema: &Value) -> Result<(), String> {
    check(value, schema, "")
}

fn at(path: &str) -> String {
    if path.is_empty() {
        "input".to_owned()
    } else {
        format!("`{path}`")
    }
}

fn type_matches(value: &Value, ty: &str) -> bool {
    match ty {
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "null" => value.is_null(),
        "number" => value.is_number(),
        "integer" => value.as_f64().is_some_and(|f| f.fract() == 0.0),
        _ => true,
    }
}

fn types(schema: &Value) -> Vec<&str> {
    match schema.get("type") {
        Some(Value::String(t)) => vec![t.as_str()],
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn usize_of(schema: &Value, key: &str) -> Option<usize> {
    schema
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
}

#[allow(clippy::too_many_lines)] // one branch per keyword, in schema order
fn check(value: &Value, schema: &Value, path: &str) -> Result<(), String> {
    let Some(s) = schema.as_object() else {
        return Ok(()); // `true` / `{}`-like schemas accept anything
    };
    let tys = types(schema);
    if !tys.is_empty() && !tys.iter().any(|t| type_matches(value, t)) {
        return Err(format!(
            "{} must be {}, got {}",
            at(path),
            tys.join(" or "),
            kind(value)
        ));
    }
    if let Some(Value::Array(options)) = s.get("enum")
        && !options.contains(value)
    {
        let list: Vec<String> = options.iter().map(Value::to_string).collect();
        return Err(format!("{} must be one of {}", at(path), list.join(", ")));
    }
    if let Some(c) = s.get("const")
        && c != value
    {
        return Err(format!("{} must be {c}", at(path)));
    }
    for key in ["anyOf", "oneOf"] {
        if let Some(Value::Array(alts)) = s.get(key)
            && !alts.is_empty()
            && !alts.iter().any(|a| check(value, a, path).is_ok())
        {
            // The first alternative's complaint is the most useful one to show.
            return check(value, &alts[0], path);
        }
    }
    if let Some(Value::Array(all)) = s.get("allOf") {
        for a in all {
            check(value, a, path)?;
        }
    }
    match value {
        Value::String(text) => {
            let n = text.chars().count();
            if let Some(min) = usize_of(schema, "minLength")
                && n < min
            {
                return Err(format!("{} must have at least {min} characters", at(path)));
            }
            if let Some(max) = usize_of(schema, "maxLength")
                && n > max
            {
                return Err(format!("{} must have at most {max} characters", at(path)));
            }
        }
        Value::Number(num) => {
            let f = num.as_f64().unwrap_or(0.0);
            if let Some(min) = s.get("minimum").and_then(Value::as_f64)
                && f < min
            {
                return Err(format!("{} must be at least {min}", at(path)));
            }
            if let Some(max) = s.get("maximum").and_then(Value::as_f64)
                && f > max
            {
                return Err(format!("{} must be at most {max}", at(path)));
            }
        }
        Value::Array(items) => {
            if let Some(min) = usize_of(schema, "minItems")
                && items.len() < min
            {
                return Err(format!("{} must have at least {min} items", at(path)));
            }
            if let Some(max) = usize_of(schema, "maxItems")
                && items.len() > max
            {
                return Err(format!("{} must have at most {max} items", at(path)));
            }
            if let Some(item_schema) = s.get("items") {
                for (i, item) in items.iter().enumerate() {
                    check(item, item_schema, &format!("{path}[{i}]"))?;
                }
            }
        }
        Value::Object(obj) => check_object(obj, s, path)?,
        Value::Bool(_) | Value::Null => {}
    }
    Ok(())
}

fn check_object(
    obj: &Map<String, Value>,
    s: &Map<String, Value>,
    path: &str,
) -> Result<(), String> {
    let join = |k: &str| {
        if path.is_empty() {
            k.to_owned()
        } else {
            format!("{path}.{k}")
        }
    };
    if let Some(Value::Array(req)) = s.get("required") {
        for r in req.iter().filter_map(Value::as_str) {
            if !obj.contains_key(r) {
                return Err(format!("missing required field `{}`", join(r)));
            }
        }
    }
    let props = s.get("properties").and_then(Value::as_object);
    for (k, v) in obj {
        match props.and_then(|p| p.get(k)) {
            Some(ps) => check(v, ps, &join(k))?,
            None => match s.get("additionalProperties") {
                Some(Value::Bool(false)) => {
                    let known: Vec<&str> = props
                        .map(|p| p.keys().map(String::as_str).collect())
                        .unwrap_or_default();
                    return Err(format!(
                        "unknown field `{}`; allowed: {}",
                        join(k),
                        known.join(", ")
                    ));
                }
                Some(extra @ Value::Object(_)) => check(v, extra, &join(k))?,
                _ => {}
            },
        }
    }
    Ok(())
}

/// Fix what a model commonly gets almost right, guided by the schema: field names in the
/// wrong case or with `-`/`_` swapped, `"3"` for 3, `"true"` for true, a JSON string where an
/// object or array belongs, a number where a string belongs. Returns whether anything changed.
pub fn coerce(value: &mut Value, schema: &Value) -> bool {
    let tys = types(schema);
    let mut changed = false;
    if !tys.is_empty()
        && !tys.iter().any(|t| type_matches(value, t))
        && let Some(fixed) = tys.iter().find_map(|t| convert(value, t))
    {
        *value = fixed;
        changed = true;
    }
    match value {
        Value::Object(obj) => {
            if let Some(props) = schema.get("properties").and_then(Value::as_object) {
                changed |= fix_keys(obj, props);
                for (k, ps) in props {
                    if let Some(v) = obj.get_mut(k) {
                        changed |= coerce(v, ps);
                    }
                }
            }
        }
        Value::Array(items) => {
            if let Some(is) = schema.get("items") {
                for item in items {
                    changed |= coerce(item, is);
                }
            }
        }
        _ => {}
    }
    changed
}

fn convert(value: &Value, ty: &str) -> Option<Value> {
    match (ty, value) {
        ("integer", Value::String(s)) => s.trim().parse::<i64>().ok().map(Value::from),
        ("integer", Value::Number(n)) => n
            .as_f64()
            .filter(|f| f.fract() == 0.0 && f.abs() < 9.0e15)
            // The guard above keeps the value inside i64 range and integral.
            .map(|f| {
                #[allow(clippy::cast_possible_truncation)]
                Value::from(f as i64)
            }),
        ("number", Value::String(s)) => s
            .trim()
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number),
        ("boolean", Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" => Some(Value::Bool(true)),
            "false" | "no" | "0" => Some(Value::Bool(false)),
            _ => None,
        },
        ("string", Value::Number(_) | Value::Bool(_)) => Some(Value::String(value.to_string())),
        ("object" | "array", Value::String(s)) => serde_json::from_str::<Value>(s)
            .ok()
            .filter(|v| type_matches(v, ty)),
        ("array", v) if !v.is_null() => Some(Value::Array(vec![v.clone()])),
        _ => None,
    }
}

fn squash(k: &str) -> String {
    k.chars()
        .filter(|c| *c != '_' && *c != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

/// `FilePath`, `file-path`, `filepath` → `file_path` when the schema declares `file_path`.
fn fix_keys(obj: &mut Map<String, Value>, props: &Map<String, Value>) -> bool {
    let wrong: Vec<(String, String)> = obj
        .keys()
        .filter(|k| !props.contains_key(*k))
        .filter_map(|k| {
            let sk = squash(k);
            props
                .keys()
                .find(|p| squash(p) == sk && !obj.contains_key(*p))
                .map(|p| (k.clone(), p.clone()))
        })
        .collect();
    for (from, to) in &wrong {
        if let Some(v) = obj.remove(from) {
            obj.insert(to.clone(), v);
        }
    }
    !wrong.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn edit_schema() -> Value {
        json!({"type": "object", "properties": {
            "file_path": {"type": "string", "maxLength": 20},
            "edits": {"type": "array", "minItems": 1, "items": {"type": "object", "properties": {
                "old_string": {"type": "string"}, "replace_all": {"type": "boolean"}
            }, "required": ["old_string"], "additionalProperties": false}},
            "mode": {"enum": ["a", "b"]},
            "limit": {"type": "integer", "minimum": 1}
        }, "required": ["file_path"], "additionalProperties": false})
    }

    #[test]
    fn reports_the_first_violation_with_its_path() {
        let s = edit_schema();
        assert!(validate(&json!({"file_path": "a"}), &s).is_ok());
        let e = |v: Value| validate(&v, &s).unwrap_err();
        assert_eq!(e(json!({})), "missing required field `file_path`");
        assert_eq!(
            e(json!({"file_path": 3})),
            "`file_path` must be string, got number"
        );
        assert!(e(json!({"file_path": "a", "x": 1})).starts_with("unknown field `x`"));
        assert!(e(json!({"file_path": "a".repeat(21)})).contains("at most 20 characters"));
        assert!(e(json!({"file_path": "a", "mode": "c"})).contains("one of \"a\", \"b\""));
        assert!(e(json!({"file_path": "a", "limit": 0})).contains("at least 1"));
        assert!(e(json!({"file_path": "a", "limit": 1.5})).contains("integer"));
        assert!(e(json!({"file_path": "a", "edits": []})).contains("at least 1 items"));
        assert_eq!(
            e(json!({"file_path": "a", "edits": [{"old_string": "x"}, {"old": "y"}]})),
            "missing required field `edits[1].old_string`"
        );
        assert!(
            validate(
                &json!(["x", null]),
                &json!({"type": "array", "items": {"type": ["string", "null"]}})
            )
            .is_ok()
        );
        assert!(
            validate(
                &json!(5),
                &json!({"anyOf": [{"type": "string"}, {"type": "integer"}]})
            )
            .is_ok()
        );
    }

    #[test]
    fn coerces_types_and_field_names_from_the_schema() {
        let s = edit_schema();
        let mut v = json!({"FilePath": "a", "limit": "3", "edits": "[{\"old_string\": \"x\", \"replace_all\": \"true\"}]"});
        assert!(coerce(&mut v, &s));
        assert_eq!(
            v,
            json!({"file_path": "a", "limit": 3, "edits": [{"old_string": "x", "replace_all": true}]})
        );
        assert!(validate(&v, &s).is_ok());

        let mut ok = json!({"file_path": "a"});
        assert!(!coerce(&mut ok, &s), "valid input is left alone");
        let mut bad = json!({"file_path": "a", "limit": "many"});
        coerce(&mut bad, &s);
        assert!(
            validate(&bad, &s).is_err(),
            "unconvertible values still fail"
        );
    }
}
