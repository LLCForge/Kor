//! GraphQL schema (introspection from the endpoint, an introspection JSON file, or an SDL file)
//! → one tool per `Query` field and, with writes enabled, one per `Mutation` field. Arguments
//! become JSON Schema; the call builds a single operation with variables and a default selection
//! set (scalar fields, two levels deep) that the model may replace with `selection`.

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use super::{AddRequest, Definition, GenTool, Kind, Plan, Secrets, serve::Backend};

const MAX_FIELDS: usize = 30;

/// The standard introspection query (types, fields, arguments, input fields, enum values).
pub const INTROSPECTION: &str = "query IntrospectionQuery { __schema { queryType { name } mutationType { name } types { kind name description fields(includeDeprecated: false) { name description args { name description type { ...T } } type { ...T } } inputFields { name description type { ...T } } enumValues(includeDeprecated: false) { name } } } } fragment T on __Type { kind name ofType { kind name ofType { kind name ofType { kind name ofType { kind name ofType { kind name ofType { kind name } } } } } } }";

#[derive(Debug, Clone, PartialEq)]
enum Ty {
    Named(String),
    List(Box<Ty>),
    NonNull(Box<Ty>),
}

impl Ty {
    fn base(&self) -> &str {
        match self {
            Self::Named(n) => n,
            Self::List(t) | Self::NonNull(t) => t.base(),
        }
    }

    fn sdl(&self) -> String {
        match self {
            Self::Named(n) => n.clone(),
            Self::List(t) => format!("[{}]", t.sdl()),
            Self::NonNull(t) => format!("{}!", t.sdl()),
        }
    }

    fn from_json(v: &Value) -> Option<Self> {
        match v["kind"].as_str()? {
            "NON_NULL" => Some(Self::NonNull(Box::new(Self::from_json(&v["ofType"])?))),
            "LIST" => Some(Self::List(Box::new(Self::from_json(&v["ofType"])?))),
            _ => Some(Self::Named(v["name"].as_str()?.to_owned())),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct Field {
    name: String,
    description: String,
    args: Vec<(String, Ty)>,
    ty: Option<Ty>,
}

#[derive(Debug, Clone, Default)]
struct TypeDef {
    /// `OBJECT`, `INTERFACE`, `UNION`, `INPUT_OBJECT`, `ENUM`, `SCALAR`.
    kind: String,
    fields: Vec<Field>,
    values: Vec<String>,
}

#[derive(Debug, Default)]
struct Schema {
    query: Option<String>,
    mutation: Option<String>,
    types: HashMap<String, TypeDef>,
}

fn from_introspection(v: &Value) -> Result<Schema, String> {
    let s = v
        .pointer("/data/__schema")
        .or_else(|| v.get("__schema"))
        .ok_or("not a GraphQL introspection result")?;
    let mut out = Schema {
        query: s["queryType"]["name"].as_str().map(str::to_owned),
        mutation: s["mutationType"]["name"].as_str().map(str::to_owned),
        types: HashMap::new(),
    };
    for t in s["types"].as_array().into_iter().flatten() {
        let Some(name) = t["name"].as_str() else {
            continue;
        };
        let field = |f: &Value| Field {
            name: f["name"].as_str().unwrap_or_default().to_owned(),
            description: f["description"].as_str().unwrap_or_default().to_owned(),
            args: f["args"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|a| Some((a["name"].as_str()?.to_owned(), Ty::from_json(&a["type"])?)))
                .collect(),
            ty: Ty::from_json(&f["type"]),
        };
        let fields = t["fields"]
            .as_array()
            .or_else(|| t["inputFields"].as_array())
            .into_iter()
            .flatten()
            .map(field)
            .collect();
        out.types.insert(
            name.to_owned(),
            TypeDef {
                kind: t["kind"].as_str().unwrap_or_default().to_owned(),
                fields,
                values: t["enumValues"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|e| e["name"].as_str().map(str::to_owned))
                    .collect(),
            },
        );
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// SDL
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Name(String),
    Str(String),
    P(char),
}

fn tokenize(src: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut it = src.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '#' => while it.next_if(|&c| c != '\n').is_some() {},
            '"' => {
                let block = it.next_if_eq(&'"').is_some();
                let mut s = String::new();
                if block {
                    if it.next_if_eq(&'"').is_none() {
                        out.push(Tok::Str(String::new())); // `""`
                        continue;
                    }
                    for c in it.by_ref() {
                        s.push(c);
                        if s.ends_with("\"\"\"") {
                            s.truncate(s.len() - 3);
                            break;
                        }
                    }
                } else {
                    while let Some(c) = it.next() {
                        match c {
                            '\\' => {
                                if let Some(n) = it.next() {
                                    s.push(n);
                                }
                            }
                            '"' | '\n' => break,
                            c => s.push(c),
                        }
                    }
                }
                out.push(Tok::Str(s));
            }
            c if c.is_whitespace() || c == ',' => {}
            c if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+') => {
                let mut s = c.to_string();
                while let Some(n) =
                    it.next_if(|n| n.is_ascii_alphanumeric() || matches!(n, '_' | '-' | '.' | '+'))
                {
                    s.push(n);
                }
                out.push(Tok::Name(s));
            }
            c => out.push(Tok::P(c)),
        }
    }
    out
}

struct Parser {
    toks: Vec<Tok>,
    i: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.i).cloned();
        self.i += 1;
        t
    }
    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(&Tok::P(c)) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn name(&mut self) -> Option<String> {
        match self.peek() {
            Some(Tok::Name(n)) => {
                let n = n.clone();
                self.i += 1;
                Some(n)
            }
            _ => None,
        }
    }
    /// A balanced `(...)`, `[...]` or `{...}` group, or one token.
    fn skip_value(&mut self) {
        let Some(Tok::P(open @ ('(' | '[' | '{'))) = self.next() else {
            return;
        };
        let close = match open {
            '(' => ')',
            '[' => ']',
            _ => '}',
        };
        let mut depth = 1;
        while let Some(t) = self.next() {
            if t == Tok::P(open) {
                depth += 1;
            } else if t == Tok::P(close) {
                depth -= 1;
                if depth == 0 {
                    return;
                }
            }
        }
    }
    fn skip_directives(&mut self) {
        while self.eat('@') {
            self.name();
            if self.peek() == Some(&Tok::P('(')) {
                self.skip_value();
            }
        }
    }
    fn ty(&mut self) -> Option<Ty> {
        let t = if self.eat('[') {
            let inner = self.ty()?;
            self.eat(']');
            Ty::List(Box::new(inner))
        } else {
            Ty::Named(self.name()?)
        };
        Some(if self.eat('!') {
            Ty::NonNull(Box::new(t))
        } else {
            t
        })
    }
    fn fields(&mut self) -> Vec<Field> {
        let mut out = Vec::new();
        if !self.eat('{') {
            return out;
        }
        let mut description = String::new();
        while let Some(t) = self.peek().cloned() {
            match t {
                Tok::P('}') => {
                    self.i += 1;
                    break;
                }
                Tok::Str(s) => {
                    self.i += 1;
                    description = s;
                }
                Tok::Name(name) => {
                    self.i += 1;
                    let mut args = Vec::new();
                    if self.eat('(') {
                        while !self.eat(')') {
                            match self.next() {
                                Some(Tok::Name(a)) => {
                                    self.eat(':');
                                    if let Some(ty) = self.ty() {
                                        args.push((a, ty));
                                    }
                                    if self.eat('=') {
                                        self.skip_value();
                                    }
                                    self.skip_directives();
                                }
                                None => break,
                                _ => {}
                            }
                        }
                    }
                    let ty = if self.eat(':') { self.ty() } else { None };
                    if self.eat('=') {
                        self.skip_value(); // input field default
                    }
                    self.skip_directives();
                    out.push(Field {
                        name,
                        description: std::mem::take(&mut description),
                        args,
                        ty,
                    });
                }
                Tok::P(_) => self.i += 1,
            }
        }
        out
    }
}

#[allow(clippy::too_many_lines)] // one arm per SDL definition kind
fn from_sdl(src: &str) -> Result<Schema, String> {
    let mut p = Parser {
        toks: tokenize(src),
        i: 0,
    };
    let mut s = Schema::default();
    while let Some(t) = p.next() {
        let Tok::Name(kw) = t else { continue };
        match kw.as_str() {
            "schema" => {
                p.skip_directives();
                if p.eat('{') {
                    while let Some(op) = p.name() {
                        p.eat(':');
                        let n = p.name();
                        match op.as_str() {
                            "query" => s.query = n,
                            "mutation" => s.mutation = n,
                            _ => {}
                        }
                    }
                    p.eat('}');
                }
            }
            "type" | "interface" | "input" => {
                let Some(name) = p.name() else { continue };
                // `implements A & B`, directives.
                while matches!(p.peek(), Some(Tok::Name(_) | Tok::P('&' | '@'))) {
                    if p.peek() == Some(&Tok::P('@')) {
                        p.skip_directives();
                    } else {
                        p.i += 1;
                    }
                }
                let kind = match kw.as_str() {
                    "type" => "OBJECT",
                    "interface" => "INTERFACE",
                    _ => "INPUT_OBJECT",
                };
                let fields = p.fields();
                let def = s.types.entry(name).or_default();
                def.kind = kind.into();
                def.fields.extend(fields); // `extend type` adds fields
            }
            "enum" => {
                let Some(name) = p.name() else { continue };
                p.skip_directives();
                let values = p.fields().into_iter().map(|f| f.name).collect();
                s.types.insert(
                    name,
                    TypeDef {
                        kind: "ENUM".into(),
                        values,
                        ..TypeDef::default()
                    },
                );
            }
            "scalar" => {
                if let Some(name) = p.name() {
                    s.types.insert(
                        name,
                        TypeDef {
                            kind: "SCALAR".into(),
                            ..TypeDef::default()
                        },
                    );
                }
            }
            "union" => {
                let Some(name) = p.name() else { continue };
                p.skip_directives();
                if p.eat('=') {
                    p.eat('|');
                    while p.name().is_some() && p.eat('|') {}
                }
                s.types.insert(
                    name,
                    TypeDef {
                        kind: "UNION".into(),
                        ..TypeDef::default()
                    },
                );
            }
            "directive" => {
                p.eat('@');
                p.name();
                if p.peek() == Some(&Tok::P('(')) {
                    p.skip_value();
                }
                while let Some(n) = p.name() {
                    if n != "repeatable" && n != "on" && !p.eat('|') {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    if s.types.is_empty() {
        return Err("no GraphQL type definitions found".into());
    }
    s.query = s
        .query
        .or_else(|| s.types.contains_key("Query").then(|| "Query".into()));
    s.mutation = s
        .mutation
        .or_else(|| s.types.contains_key("Mutation").then(|| "Mutation".into()));
    Ok(s)
}

// ---------------------------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------------------------

fn json_schema(s: &Schema, ty: &Ty, depth: usize) -> Value {
    match ty {
        Ty::NonNull(t) => json_schema(s, t, depth),
        Ty::List(t) => json!({"type": "array", "items": json_schema(s, t, depth)}),
        Ty::Named(n) => match n.as_str() {
            "Int" => json!({"type": "integer"}),
            "Float" => json!({"type": "number"}),
            "Boolean" => json!({"type": "boolean"}),
            "String" | "ID" => json!({"type": "string"}),
            _ => match s.types.get(n) {
                Some(t) if t.kind == "ENUM" => json!({"type": "string", "enum": t.values}),
                Some(t) if t.kind == "INPUT_OBJECT" && depth < 5 => {
                    object_schema(s, &t.fields_as_args(), depth + 1)
                }
                _ => json!({"description": format!("GraphQL {n}")}),
            },
        },
    }
}

impl TypeDef {
    fn fields_as_args(&self) -> Vec<(String, Ty)> {
        self.fields
            .iter()
            .filter_map(|f| Some((f.name.clone(), f.ty.clone()?)))
            .collect()
    }
}

fn object_schema(s: &Schema, args: &[(String, Ty)], depth: usize) -> Value {
    let mut props = Map::new();
    let mut required = Vec::new();
    for (n, t) in args {
        props.insert(n.clone(), json_schema(s, t, depth));
        if matches!(t, Ty::NonNull(_)) {
            required.push(n.clone());
        }
    }
    json!({"type": "object", "properties": props, "required": required})
}

/// Fields of `name` without required arguments: scalars always, objects up to `depth` 2.
fn selection(s: &Schema, name: &str, depth: usize) -> String {
    let Some(t) = s.types.get(name) else {
        return String::new();
    };
    match t.kind.as_str() {
        "UNION" => "{ __typename }".into(),
        "OBJECT" | "INTERFACE" => {
            let mut parts = Vec::new();
            for f in &t.fields {
                if parts.len() >= MAX_FIELDS
                    || f.args.iter().any(|(_, t)| matches!(t, Ty::NonNull(_)))
                {
                    continue;
                }
                let Some(ty) = &f.ty else { continue };
                let sub = s.types.get(ty.base());
                let leaf = sub.is_none_or(|d| matches!(d.kind.as_str(), "SCALAR" | "ENUM"));
                if leaf {
                    parts.push(f.name.clone());
                } else if depth < 2 {
                    let inner = selection(s, ty.base(), depth + 1);
                    if !inner.is_empty() {
                        parts.push(format!("{} {inner}", f.name));
                    }
                }
            }
            if parts.is_empty() {
                "{ __typename }".into()
            } else {
                format!("{{ {} }}", parts.join(" "))
            }
        }
        _ => String::new(),
    }
}

fn tools(s: &Schema, allow_writes: bool) -> (Vec<GenTool>, usize) {
    let mut out = Vec::new();
    let mut skipped = 0;
    for (op, root) in [("query", &s.query), ("mutation", &s.mutation)] {
        let Some(t) = root.as_ref().and_then(|r| s.types.get(r)) else {
            continue;
        };
        let write = op == "mutation";
        if write && !allow_writes {
            skipped += t.fields.len();
            continue;
        }
        for f in &t.fields {
            let Some(ty) = &f.ty else { continue };
            let mut schema = object_schema(s, &f.args, 0);
            let sel = selection(s, ty.base(), 0);
            if !sel.is_empty() {
                schema["properties"]["selection"] = json!({"type": "string", "maxLength": 4000,
                    "description": format!("GraphQL selection set to return instead of the default `{sel}`")});
            }
            let desc = if f.description.is_empty() {
                format!("GraphQL {op} `{}`", f.name)
            } else {
                f.description.clone()
            };
            let mut name: String = format!("{op}_{}", f.name)
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            name.truncate(64);
            out.push(GenTool {
                name,
                description: format!("{desc} (returns {})", ty.sdl()),
                input_schema: schema,
                write,
                plan: Plan::Graphql {
                    operation: op.into(),
                    field: f.name.clone(),
                    args: f.args.iter().map(|(n, t)| (n.clone(), t.sdl())).collect(),
                    selection: sel,
                },
            });
        }
    }
    (out, skipped)
}

async fn post(
    endpoint: &str,
    body: &Value,
    timeout: std::time::Duration,
    auth: Option<&super::Auth>,
    secrets: &Secrets,
) -> Result<String, String> {
    let client = super::http_client(timeout)?;
    let url = reqwest::Url::parse(endpoint).map_err(|e| e.to_string())?;
    let rb = super::openapi::request(&client, reqwest::Method::POST, url, auth, secrets)?;
    let resp = super::openapi::json_body(rb, body)
        .send()
        .await
        .map_err(|e| kor_core::net::describe(&e))?;
    let status = resp.status();
    let text = super::read_body(resp).await?;
    if status.is_success() {
        Ok(text)
    } else {
        Err(format!("HTTP {status}\n{text}"))
    }
}

/// Tools of the schema at `location`: an endpoint URL (introspected), an introspection JSON
/// file or an SDL file (`--base-url` gives the endpoint for files).
pub async fn definition(location: &str, req: &AddRequest) -> Result<Definition, String> {
    let url = location.starts_with("http://") || location.starts_with("https://");
    let schema = if url {
        // The credential for introspection comes from the environment only at add time.
        let secrets: Secrets = std::sync::Arc::new(|n: &str| std::env::var(n).ok());
        let text = post(
            location,
            &json!({"query": INTROSPECTION}),
            std::time::Duration::from_secs(30),
            req.auth.as_ref(),
            &secrets,
        )
        .await?;
        from_introspection(
            &serde_json::from_str(&text).map_err(|e| format!("introspection: {e}"))?,
        )?
    } else {
        let text = super::fetch_text(location).await?;
        match serde_json::from_str::<Value>(&text) {
            Ok(v) => from_introspection(&v)?,
            Err(_) => from_sdl(&text)?,
        }
    };
    let endpoint = req
        .endpoint
        .clone()
        .or_else(|| url.then(|| location.to_owned()))
        .ok_or("a schema file needs the endpoint: --base-url https://…/graphql")?;
    if !reqwest::Url::parse(&endpoint).is_ok_and(|u| matches!(u.scheme(), "http" | "https")) {
        return Err(format!("endpoint must be http(s): {endpoint}"));
    }
    let (tools, skipped) = tools(&schema, req.allow_writes);
    if tools.is_empty() && skipped == 0 {
        return Err("the schema has no query or mutation fields".into());
    }
    let id = reqwest::Url::parse(&endpoint)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_else(|| "graphql".into());
    Ok(Definition {
        id: super::slug(&id),
        kind: Kind::Graphql,
        source: location.to_owned(),
        endpoint: Some(endpoint),
        allow_writes: req.allow_writes,
        auth: req.auth.clone(),
        db: None,
        row_limit: super::default_rows(),
        timeout_secs: super::default_timeout(),
        tools,
        skipped,
    })
}

/// A selection set the model gave: one balanced `{ … }` block, no strings or comments, so it
/// cannot close the operation or add another one.
fn check_selection(s: &str) -> Result<(), String> {
    let s = s.trim();
    let bad = || Err("selection must be one `{ … }` block of field names".to_owned());
    if !s.starts_with('{') || !s.ends_with('}') || s.contains(['"', '#']) {
        return bad();
    }
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 && i + 1 != s.len() {
                    return bad();
                }
            }
            _ => {}
        }
    }
    if depth == 0 { Ok(()) } else { bad() }
}

/// A generated GraphQL server.
pub struct Server {
    def: Definition,
    secrets: Secrets,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("graphql::Server")
            .field("id", &self.def.id)
            .finish_non_exhaustive()
    }
}

impl Server {
    pub fn new(def: Definition, secrets: Secrets) -> Self {
        Self { def, secrets }
    }
}

#[async_trait]
impl Backend for Server {
    fn name(&self) -> String {
        format!("kor-{}", self.def.id)
    }

    fn tools(&self) -> Vec<Value> {
        self.def.tools.iter().map(GenTool::descriptor).collect()
    }

    async fn call(&self, name: &str, args: Value) -> Result<String, String> {
        let tool = self
            .def
            .tools
            .iter()
            .find(|t| t.name == name)
            .ok_or("unknown tool")?;
        let Plan::Graphql {
            operation,
            field,
            args: params,
            selection,
        } = &tool.plan
        else {
            return Err("not a GraphQL tool".into());
        };
        super::validate(&tool.input_schema, &args)?;
        let sel = match args.get("selection").and_then(Value::as_str) {
            Some(s) => {
                check_selection(s)?;
                s.to_owned()
            }
            None => selection.clone(),
        };
        let mut decls = Vec::new();
        let mut pass = Vec::new();
        let mut vars = Map::new();
        for (n, ty) in params {
            if let Some(v) = args.get(n).filter(|v| !v.is_null()) {
                decls.push(format!("${n}: {ty}"));
                pass.push(format!("{n}: ${n}"));
                vars.insert(n.clone(), v.clone());
            }
        }
        let paren = |v: &[String]| {
            if v.is_empty() {
                String::new()
            } else {
                format!("({})", v.join(", "))
            }
        };
        let query = format!(
            "{operation} KorCall{} {{ {field}{} {sel} }}",
            paren(&decls),
            paren(&pass)
        );
        let endpoint = self.def.endpoint.as_deref().ok_or("no endpoint")?;
        let text = post(
            endpoint,
            &json!({"query": query, "variables": vars}),
            self.def.timeout(),
            self.def.auth.as_ref(),
            &self.secrets,
        )
        .await?;
        let failed = serde_json::from_str::<Value>(&text).ok().is_some_and(|v| {
            v["data"].is_null() && v["errors"].as_array().is_some_and(|e| !e.is_empty())
        });
        let out = format!("{}\n{text}", crate::remote::UNTRUSTED);
        if failed { Err(out) } else { Ok(out) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use wiremock::matchers::{body_partial_json, body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const SDL: &str = r#"
        "A book" type Book @key(fields: "id") { id: ID! title: String author: Author reviews(first: Int!): [String] }
        type Author implements Node & Named { name: String books: [Book] }
        interface Node { id: ID! }
        union Item = Book | Author
        enum Genre { FICTION, SCIENCE }
        input BookInput { title: String!, genre: Genre = FICTION }
        scalar Date
        # a comment { not a field }
        type Query {
          """Find a book by id"""
          book(id: ID!): Book
          books(genre: Genre, limit: Int = 10 @deprecated): [Book!]!
          search(q: String!): [Item]
        }
        type Mutation { addBook(input: BookInput!): Book }
        directive @key(fields: String!) repeatable on OBJECT | INTERFACE
    "#;

    #[test]
    fn sdl_becomes_tools() {
        let s = from_sdl(SDL).unwrap();
        assert_eq!(s.query.as_deref(), Some("Query"));
        let (t, skipped) = tools(&s, false);
        assert_eq!(skipped, 1);
        let names: Vec<_> = t.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["query_book", "query_books", "query_search"]);
        assert_eq!(t[0].description, "Find a book by id (returns Book)");
        assert_eq!(t[0].input_schema["required"], json!(["id"]));
        assert_eq!(
            t[1].input_schema["properties"]["genre"]["enum"],
            json!(["FICTION", "SCIENCE"])
        );
        let Plan::Graphql { selection, .. } = &t[0].plan else {
            panic!()
        };
        assert_eq!(selection, "{ id title author { name books { id title } } }");
        let (t, _) = tools(&s, true);
        let add = t.iter().find(|t| t.name == "mutation_addBook").unwrap();
        assert!(add.write);
        assert_eq!(
            add.input_schema["properties"]["input"]["required"],
            json!(["title"])
        );
        assert!(check_selection("{ id title }").is_ok());
        for bad in ["{ id } } mutation { x }", "{ id # }", "id", "{ \"}\" }"] {
            assert!(check_selection(bad).is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn introspection_endpoint_to_tools_to_calls() {
        let mock = MockServer::start().await;
        let ty = |kind: &str, name: &str| json!({"kind": kind, "name": name, "ofType": null});
        let schema = json!({"data": {"__schema": {
        "queryType": {"name": "Query"}, "mutationType": {"name": "Mutation"},
        "types": [
            {"kind": "OBJECT", "name": "Query", "fields": [
                {"name": "user", "description": "A user", "args": [{"name": "id", "type": {"kind": "NON_NULL", "name": null, "ofType": ty("SCALAR", "ID")}}], "type": ty("OBJECT", "User")}]},
            {"kind": "OBJECT", "name": "Mutation", "fields": [
                {"name": "deleteUser", "args": [{"name": "id", "type": ty("SCALAR", "ID")}], "type": ty("SCALAR", "Boolean")}]},
            {"kind": "OBJECT", "name": "User", "fields": [
                {"name": "id", "args": [], "type": ty("SCALAR", "ID")},
                {"name": "email", "args": [], "type": ty("SCALAR", "String")}]},
            {"kind": "SCALAR", "name": "ID"}, {"kind": "SCALAR", "name": "String"}, {"kind": "SCALAR", "name": "Boolean"}
        ]}}});
        Mock::given(method("POST"))
            .and(body_string_contains("IntrospectionQuery"))
            .respond_with(ResponseTemplate::new(200).set_body_json(schema))
            .mount(&mock)
            .await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"query": "query KorCall($id: ID!) { user(id: $id) { id email } }", "variables": {"id": "42"}})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": {"user": {"id": "42", "email": "a@b"}}})))
            .mount(&mock)
            .await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"variables": {"id": "0"}})))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data": null, "errors": [{"message": "not found"}]})),
            )
            .mount(&mock)
            .await;
        let endpoint = format!("{}/graphql", mock.uri());
        let added = super::super::generate(
            &AddRequest {
                source: endpoint.clone(),
                ..AddRequest::default()
            },
            std::path::Path::new("."),
            std::path::Path::new("kor"),
        )
        .await
        .unwrap();
        let d = added.definition.unwrap();
        assert_eq!(d.kind, Kind::Graphql);
        assert_eq!(d.tools.len(), 1);
        assert_eq!(d.skipped, 1, "mutations need --allow-writes");
        let s = Server::new(d, Arc::new(|_: &str| None));
        let out = s.call("query_user", json!({"id": "42"})).await.unwrap();
        assert!(out.contains("a@b"), "{out}");
        assert!(
            s.call("query_user", json!({"id": "0"}))
                .await
                .unwrap_err()
                .contains("not found")
        );
        assert!(
            s.call(
                "query_user",
                json!({"id": "1", "selection": "{ id } } mutation { deleteUser }"})
            )
            .await
            .is_err()
        );
    }
}
