//! §15.13: a JSON Schema as a llama.cpp GBNF grammar, so a llama.cpp server constrains the reply
//! token by token even where it has no JSON-schema support. Covers `type` (object, array,
//! string, number, integer, boolean, null, or a list of them), `properties` + `required`,
//! `items`, `enum`, `const`, `anyOf`/`oneOf`. Anything else (`$ref`, patterns, formats) becomes
//! "any JSON value": still valid JSON, checked by the caller like every structured reply.

use serde_json::Value;
use std::fmt::Write as _;

/// Nesting beyond this is "any JSON value" (a schema is not a program).
const MAX_DEPTH: usize = 12;

const BASE: &str = r#"ws ::= [ \t\n]*
string ::= "\"" ( [^"\\\x7F\x00-\x1F] | "\\" ( ["\\/bfnrt] | "u" [0-9a-fA-F] [0-9a-fA-F] [0-9a-fA-F] [0-9a-fA-F] ) )* "\"" ws
number ::= "-"? ( [0-9] | [1-9] [0-9]* ) ( "." [0-9]+ )? ( [eE] [-+]? [0-9]+ )? ws
integer ::= "-"? ( [0-9] | [1-9] [0-9]* ) ws
boolean ::= ( "true" | "false" ) ws
null ::= "null" ws
value ::= object | array | string | number | boolean | null
object ::= "{" ws ( string ":" ws value ( "," ws string ":" ws value )* )? "}" ws
array ::= "[" ws ( value ( "," ws value )* )? "]" ws
"#;

/// The grammar; its start rule is `root`.
pub fn from_schema(schema: &Value) -> String {
    let mut g = Grammar::default();
    let root = g.rule(schema, 0);
    let mut out = format!("root ::= ws {root}\n");
    for (name, body) in &g.rules {
        let _ = writeln!(out, "{name} ::= {body}");
    }
    out.push_str(BASE);
    out
}

#[derive(Default)]
struct Grammar {
    rules: Vec<(String, String)>,
}

impl Grammar {
    fn add(&mut self, body: String) -> String {
        if let Some((name, _)) = self.rules.iter().find(|(_, b)| *b == body) {
            return name.clone();
        }
        let name = format!("r{}", self.rules.len());
        self.rules.push((name.clone(), body));
        name
    }

    /// A rule (or a base rule name) for `s`.
    fn rule(&mut self, s: &Value, depth: usize) -> String {
        if depth > MAX_DEPTH {
            return "value".into();
        }
        if let Some(c) = s.get("const") {
            return self.add(literal(c));
        }
        if let Some(e) = s.get("enum").and_then(Value::as_array) {
            let alts: Vec<String> = e.iter().map(literal).collect();
            return self.add(format!("( {} )", alts.join(" | ")));
        }
        for key in ["anyOf", "oneOf"] {
            if let Some(list) = s.get(key).and_then(Value::as_array) {
                let alts: Vec<String> = list.iter().map(|x| self.rule(x, depth + 1)).collect();
                return self.add(format!("( {} )", alts.join(" | ")));
            }
        }
        match s.get("type") {
            Some(Value::Array(types)) => {
                let alts: Vec<String> = types
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|t| {
                        let mut one = s.clone();
                        one["type"] = Value::String(t.to_owned());
                        self.rule(&one, depth + 1)
                    })
                    .collect();
                if alts.is_empty() {
                    "value".into()
                } else {
                    self.add(format!("( {} )", alts.join(" | ")))
                }
            }
            Some(Value::String(t)) => match t.as_str() {
                "object" => self.object(s, depth),
                "array" => {
                    let item = s
                        .get("items")
                        .map_or_else(|| "value".to_owned(), |i| self.rule(i, depth + 1));
                    self.add(format!(r#""[" ws ( {item} ( "," ws {item} )* )? "]" ws"#))
                }
                "string" | "number" | "integer" | "boolean" | "null" => t.clone(),
                _ => "value".into(),
            },
            _ => {
                if s.get("properties").is_some() {
                    self.object(s, depth)
                } else {
                    "value".into()
                }
            }
        }
    }

    /// Required properties in schema order, then the optional ones, each optional. Without any
    /// required property every property is written (still valid for the schema).
    fn object(&mut self, s: &Value, depth: usize) -> String {
        let Some(props) = s
            .get("properties")
            .and_then(Value::as_object)
            .filter(|p| !p.is_empty())
        else {
            return "object".into();
        };
        let required: Vec<&str> = s
            .get("required")
            .and_then(Value::as_array)
            .map(|r| r.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let none_required = !props.keys().any(|k| required.contains(&k.as_str()));
        let mut must = Vec::new();
        let mut may = Vec::new();
        for (k, v) in props {
            let pair = format!(
                "{} \":\" ws {}",
                literal(&Value::String(k.clone())).trim_end_matches(" ws"),
                self.rule(v, depth + 1)
            );
            let pair = pair.replacen("\" \":\"", "\" ws \":\"", 1);
            if none_required || required.contains(&k.as_str()) {
                must.push(pair);
            } else {
                may.push(pair);
            }
        }
        let mut body = format!(r#""{{" ws {}"#, must.join(r#" "," ws "#));
        for p in may {
            let _ = write!(body, r#" ( "," ws {p} )?"#);
        }
        body.push_str(r#" "}" ws"#);
        self.add(body)
    }
}

/// A JSON value as a GBNF literal followed by optional whitespace.
fn literal(v: &Value) -> String {
    let json = v.to_string();
    let escaped: String = json
        .chars()
        .flat_map(|c| match c {
            '"' => vec!['\\', '"'],
            '\\' => vec!['\\', '\\'],
            '\n' => vec!['\\', 'n'],
            c => vec![c],
        })
        .collect();
    format!("\"{escaped}\" ws")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_commit_split_schema_becomes_a_grammar() {
        let g = from_schema(&json!({
            "type": "object",
            "properties": {"commits": {"type": "array", "items": {
                "type": "object",
                "properties": {"files": {"type": "array", "items": {"type": "string"}}, "message": {"type": "string"}},
                "required": ["files", "message"]
            }}},
            "required": ["commits"]
        }));
        assert!(g.starts_with("root ::= ws r"), "{g}");
        assert!(g.contains(r#""\"commits\"" ws ":" ws"#), "{g}");
        assert!(
            g.contains(r#""\"files\"" ws ":" ws"#) && g.contains(r#""\"message\"" ws ":" ws"#),
            "{g}"
        );
        // Every referenced rule is defined.
        let defined: Vec<&str> = g.lines().filter_map(|l| l.split(" ::= ").next()).collect();
        for l in g.lines() {
            let body = l.split(" ::= ").nth(1).unwrap_or_default();
            for word in body
                .split(|c: char| !c.is_ascii_alphanumeric())
                .filter(|w| w.starts_with('r') && w[1..].parse::<u32>().is_ok())
            {
                assert!(defined.contains(&word), "{word} undefined in\n{g}");
            }
        }
    }

    #[test]
    fn enums_optional_properties_and_unions() {
        let g = from_schema(&json!({
            "type": "object",
            "properties": {
                "level": {"enum": ["low", "high"]},
                "note": {"type": ["string", "null"]},
                "n": {"type": "integer"}
            },
            "required": ["level"]
        }));
        assert!(g.contains(r#"( "\"low\"" ws | "\"high\"" ws )"#), "{g}");
        assert!(
            g.contains(r#"( "," ws "\"note\"" ws ":" ws"#),
            "optional: {g}"
        );
        assert!(g.contains("( string | null )"), "{g}");
        // Unknown keywords degrade to any JSON value, never to an invalid grammar.
        assert!(from_schema(&json!({"$ref": "#/defs/x"})).starts_with("root ::= ws value"));
    }
}
