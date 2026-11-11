//! `OpenAPI` 3.x and Swagger 2.0 (JSON or YAML) → one tool per operation. Inputs are the
//! operation's path/query/header parameters plus `body` (the JSON request body), as JSON Schema
//! with local `$ref`s inlined. Calls go through the network guard to the spec's server (or
//! `--base-url`), with the credential from the vault.

use std::fmt::Write as _;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use super::{AddRequest, Definition, GenTool, Kind, Plan, Secrets, serve::Backend};

const METHODS: [&str; 7] = ["get", "put", "post", "delete", "patch", "head", "options"];
/// An input schema larger than this (after inlining `$ref`s) is replaced by a plain object.
const MAX_SCHEMA: usize = 64 * 1024;

fn parse_doc(text: &str) -> Result<Value, String> {
    serde_json::from_str(text)
        .or_else(|_| serde_yaml::from_str::<Value>(text))
        .map_err(|e| format!("not a JSON or YAML document: {e}"))
}

/// Inline local `#/...` references, guarding against cycles and depth.
fn deref(doc: &Value, v: &Value, stack: &mut Vec<String>) -> Value {
    if stack.len() > 12 {
        return json!({"type": "object"});
    }
    match v {
        Value::Object(o) => {
            if let Some(r) = o.get("$ref").and_then(Value::as_str) {
                if stack.iter().any(|s| s == r) {
                    return json!({"type": "object", "description": "recursive structure"});
                }
                let target = r
                    .strip_prefix('#')
                    .and_then(|p| doc.pointer(p))
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                stack.push(r.to_owned());
                let out = deref(doc, &target, stack);
                stack.pop();
                return out;
            }
            Value::Object(
                o.iter()
                    .filter(|(k, _)| {
                        !matches!(k.as_str(), "example" | "examples" | "xml" | "externalDocs")
                    })
                    .map(|(k, x)| (k.clone(), deref(doc, x, stack)))
                    .collect(),
            )
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| deref(doc, x, stack)).collect()),
        other => other.clone(),
    }
}

fn clean_name(s: &str) -> String {
    let mut n: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    n.truncate(64);
    n
}

fn short(s: &str, max: usize) -> String {
    let s = s.trim();
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_owned(),
    }
}

/// `{var}` in a server URL → the variable's default.
fn server_url(server: &Value) -> Option<String> {
    let mut url = server.get("url")?.as_str()?.to_owned();
    if let Some(vars) = server.get("variables").and_then(Value::as_object) {
        for (k, v) in vars {
            if let Some(d) = v.get("default").and_then(Value::as_str) {
                url = url.replace(&format!("{{{k}}}"), d);
            }
        }
    }
    Some(url)
}

fn base_url(doc: &Value, spec_url: Option<&str>) -> Option<String> {
    let origin = spec_url.and_then(|u| reqwest::Url::parse(u).ok());
    if doc.get("swagger").is_some() {
        let scheme = doc["schemes"][0]
            .as_str()
            .map(str::to_owned)
            .or_else(|| origin.as_ref().map(|u| u.scheme().to_owned()))
            .unwrap_or_else(|| "https".into());
        let host = doc["host"].as_str().map(str::to_owned).or_else(|| {
            origin.as_ref().and_then(|u| {
                Some(format!(
                    "{}{}",
                    u.host_str()?,
                    u.port().map(|p| format!(":{p}")).unwrap_or_default()
                ))
            })
        })?;
        return Some(format!(
            "{scheme}://{host}{}",
            doc["basePath"].as_str().unwrap_or("")
        ));
    }
    let first = doc["servers"].get(0).and_then(server_url);
    match (first, origin) {
        (Some(u), _) if u.starts_with("http://") || u.starts_with("https://") => Some(u),
        (Some(rel), Some(o)) => o.join(&rel).ok().map(|u| u.to_string()),
        (None, Some(o)) => Some(o.origin().ascii_serialization()),
        (Some(_) | None, None) => None,
    }
}

/// A parameter's schema: 3.x `schema`, 2.0 inline `type`/`format`/`items`/`enum`.
fn param_schema(p: &Value) -> Value {
    let mut s = p.get("schema").cloned().unwrap_or_else(|| {
        let mut m = Map::new();
        for k in [
            "type", "format", "items", "enum", "default", "minimum", "maximum",
        ] {
            if let Some(v) = p.get(k) {
                m.insert(k.into(), v.clone());
            }
        }
        Value::Object(m)
    });
    if let (Some(d), Some(o)) = (
        p.get("description").and_then(Value::as_str),
        s.as_object_mut(),
    ) {
        o.insert("description".into(), json!(short(d, 300)));
    }
    s
}

/// The tools of an OpenAPI/Swagger document.
///
/// # Errors
/// If the document is not `OpenAPI` 2.0/3.x or has no base URL.
#[allow(clippy::too_many_lines)] // one pass over paths × methods
pub fn definition(
    text: &str,
    spec_url: Option<&str>,
    req: &AddRequest,
) -> Result<Definition, String> {
    let doc = parse_doc(text)?;
    let v3 = doc["openapi"].as_str().is_some_and(|v| v.starts_with('3'));
    let v2 = doc["swagger"].as_str() == Some("2.0");
    if !v3 && !v2 {
        return Err("not an OpenAPI 3.x or Swagger 2.0 document".into());
    }
    let endpoint = req
        .endpoint
        .clone()
        .or_else(|| base_url(&doc, spec_url))
        .ok_or("the document names no server; give one with --base-url")?;
    let ok = reqwest::Url::parse(&endpoint).is_ok_and(|u| matches!(u.scheme(), "http" | "https"));
    if !ok {
        return Err(format!("base URL must be http(s): {endpoint}"));
    }
    let title = doc["info"]["title"].as_str().unwrap_or("api");
    let mut tools = Vec::new();
    let mut skipped = 0;
    for (path, item) in doc["paths"].as_object().into_iter().flatten() {
        let item = deref(&doc, item, &mut Vec::new());
        let shared = item["parameters"].as_array().cloned().unwrap_or_default();
        for method in METHODS {
            let Some(op) = item.get(method) else { continue };
            let write = !matches!(method, "get" | "head" | "options");
            if write && !req.allow_writes {
                skipped += 1;
                continue;
            }
            let mut props = Map::new();
            let mut required = Vec::new();
            let mut params = Vec::new();
            let mut body = false;
            let own = op["parameters"].as_array().cloned().unwrap_or_default();
            for p in shared.iter().chain(own.iter()) {
                let (Some(name), Some(place)) = (p["name"].as_str(), p["in"].as_str()) else {
                    continue;
                };
                match place {
                    "path" | "query" | "header" => {
                        params.retain(|(n, _): &(String, String)| n != name);
                        params.push((name.to_owned(), place.to_owned()));
                        props.insert(name.to_owned(), param_schema(p));
                        if place == "path" || p["required"] == true {
                            required.push(name.to_owned());
                        }
                    }
                    "body" => {
                        body = true;
                        props.insert("body".into(), p["schema"].clone());
                        if p["required"] == true {
                            required.push("body".into());
                        }
                    }
                    // shortcut: cookie and formData parameters are not sent; add them when an API needs them.
                    _ => {}
                }
            }
            if let Some(rb) = op.get("requestBody") {
                let content = rb["content"].as_object();
                let schema = content.and_then(|c| {
                    c.iter()
                        .find(|(k, _)| k.contains("json"))
                        .or_else(|| c.iter().next())
                        .map(|(_, v)| v["schema"].clone())
                });
                if let Some(s) = schema {
                    body = true;
                    props.insert("body".into(), s);
                    if rb["required"] == true {
                        required.push("body".into());
                    }
                }
            }
            required.sort();
            required.dedup();
            let mut schema = json!({"type": "object", "properties": props, "required": required});
            if schema.to_string().len() > MAX_SCHEMA {
                schema = json!({"type": "object", "description": "Input schema too large to inline; see the API documentation."});
            }
            let name = op["operationId"]
                .as_str()
                .map_or_else(|| clean_name(&format!("{method}{path}")), clean_name);
            let summary = op["summary"]
                .as_str()
                .or_else(|| op["description"].as_str())
                .unwrap_or("");
            let upper = method.to_ascii_uppercase();
            tools.push(GenTool {
                name,
                description: format!("{} ({upper} {path})", short(summary, 500)),
                input_schema: schema,
                write,
                plan: Plan::Http {
                    method: upper,
                    path: path.clone(),
                    params,
                    body,
                },
            });
        }
    }
    if tools.is_empty() && skipped == 0 {
        return Err("the document has no operations".into());
    }
    Ok(Definition {
        id: super::slug(title),
        kind: Kind::Openapi,
        source: spec_url.unwrap_or(&req.source).to_owned(),
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

/// Everything but RFC 3986 unreserved characters is escaped, `.` too (no `..` segments).
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

pub(crate) fn text_of(v: &Value) -> String {
    v.as_str().map_or_else(|| v.to_string(), str::to_owned)
}

/// A request to `url` with the configured credential applied.
pub(crate) fn request(
    client: &reqwest::Client,
    method: reqwest::Method,
    mut url: reqwest::Url,
    auth: Option<&super::Auth>,
    secrets: &Secrets,
) -> Result<reqwest::RequestBuilder, String> {
    let Some(a) = auth else {
        return Ok(client.request(method, url));
    };
    let secret = secrets(&a.secret).ok_or_else(|| {
        format!(
            "secret `{}` not found: add it with `kor vault add {}`",
            a.secret, a.secret
        )
    })?;
    if a.place == "query" {
        url.query_pairs_mut().append_pair(&a.name, &secret);
    }
    let rb = client.request(method, url);
    Ok(match a.place.as_str() {
        "bearer" => rb.bearer_auth(secret),
        "header" => rb.header(a.name.as_str(), secret),
        _ => rb,
    })
}

/// A JSON request body (reqwest is built without its `json` feature).
pub(crate) fn json_body(rb: reqwest::RequestBuilder, v: &Value) -> reqwest::RequestBuilder {
    rb.header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(v.to_string())
}

pub(crate) async fn send(rb: reqwest::RequestBuilder) -> Result<String, String> {
    let resp = rb.send().await.map_err(|e| kor_core::net::describe(&e))?;
    let status = resp.status();
    let body = super::read_body(resp).await?;
    let text = format!("{}\nHTTP {status}\n{body}", crate::remote::UNTRUSTED);
    if status.is_success() {
        Ok(text)
    } else {
        Err(text)
    }
}

/// A generated `OpenAPI` server.
pub struct Server {
    def: Definition,
    secrets: Secrets,
    client: Result<reqwest::Client, String>,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("openapi::Server")
            .field("id", &self.def.id)
            .finish_non_exhaustive()
    }
}

impl Server {
    pub fn new(def: Definition, secrets: Secrets) -> Self {
        let client = super::http_client(def.timeout());
        Self {
            def,
            secrets,
            client,
        }
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
        let Plan::Http {
            method,
            path,
            params,
            body,
        } = &tool.plan
        else {
            return Err("not an HTTP tool".into());
        };
        super::validate(&tool.input_schema, &args)?;
        let base = self.def.endpoint.as_deref().ok_or("no base URL")?;
        let mut route = path.clone();
        for (n, place) in params {
            if place == "path" {
                let v = args.get(n).ok_or_else(|| format!("`{n}` is required"))?;
                route = route.replace(&format!("{{{n}}}"), &encode(&text_of(v)));
            }
        }
        let mut url = reqwest::Url::parse(&format!("{}{route}", base.trim_end_matches('/')))
            .map_err(|e| e.to_string())?;
        for (n, place) in params {
            let Some(v) = args.get(n).filter(|v| !v.is_null()) else {
                continue;
            };
            if place == "query" {
                let mut q = url.query_pairs_mut();
                match v.as_array() {
                    Some(items) => {
                        for i in items {
                            q.append_pair(n, &text_of(i));
                        }
                    }
                    None => {
                        q.append_pair(n, &text_of(v));
                    }
                }
            }
        }
        let client = self.client.as_ref().map_err(Clone::clone)?;
        let m = reqwest::Method::from_bytes(method.as_bytes()).map_err(|e| e.to_string())?;
        let mut rb = request(client, m, url, self.def.auth.as_ref(), &self.secrets)?;
        for (n, place) in params {
            if place == "header"
                && let Some(v) = args.get(n).filter(|v| !v.is_null())
            {
                let value = reqwest::header::HeaderValue::from_str(&text_of(v))
                    .map_err(|_| format!("`{n}`: not a valid header value"))?;
                rb = rb.header(n.as_str(), value);
            }
        }
        if *body && let Some(b) = args.get("body").filter(|v| !v.is_null()) {
            rb = json_body(rb, b);
        }
        send(rb).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use wiremock::matchers::{body_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const PETSTORE: &str = r##"
openapi: 3.0.3
info: {title: Swagger Petstore}
servers: [{url: "/v3"}]
paths:
  /pet/{petId}:
    parameters:
      - {name: petId, in: path, required: true, schema: {type: integer}}
    get:
      operationId: getPetById
      summary: Find pet by ID
      responses: {"200": {description: ok}}
    delete:
      operationId: deletePet
      responses: {"200": {description: ok}}
  /pet/findByStatus:
    get:
      operationId: findPetsByStatus
      parameters:
        - {name: status, in: query, schema: {type: string, enum: [available, sold]}}
      responses: {"200": {description: ok}}
  /pet:
    post:
      operationId: addPet
      requestBody:
        required: true
        content:
          application/json:
            schema: {$ref: "#/components/schemas/Pet"}
      responses: {"200": {description: ok}}
components:
  schemas:
    Pet:
      type: object
      required: [name]
      properties:
        name: {type: string}
        children: {type: array, items: {$ref: "#/components/schemas/Pet"}}
"##;

    fn req(allow_writes: bool) -> AddRequest {
        AddRequest {
            source: "petstore".into(),
            allow_writes,
            auth: Some(super::super::Auth::parse("header:X-Api-Key:PET_TOKEN").unwrap()),
            ..AddRequest::default()
        }
    }

    #[test]
    fn swagger_2_and_refs() {
        let doc = r##"{"swagger":"2.0","info":{"title":"t"},"host":"api.x","basePath":"/v1","schemes":["https"],
          "paths":{"/users/{id}":{"get":{"parameters":[{"name":"id","in":"path","type":"string"}]},
          "put":{"parameters":[{"name":"id","in":"path","type":"string"},{"name":"u","in":"body","required":true,"schema":{"$ref":"#/definitions/U"}}]}}},
          "definitions":{"U":{"type":"object","properties":{"n":{"type":"string"}}}}}"##;
        let d = definition(doc, None, &req(true)).unwrap();
        assert_eq!(d.endpoint.as_deref(), Some("https://api.x/v1"));
        assert_eq!(d.tools.len(), 2);
        let put = &d.tools[1];
        assert!(put.write && put.name == "put_users__id_");
        assert_eq!(
            put.input_schema["properties"]["body"]["properties"]["n"]["type"],
            "string"
        );
        assert_eq!(put.input_schema["required"], json!(["body", "id"]));
        assert!(definition("{\"x\":1}", None, &req(false)).is_err());
        // A file without servers needs --base-url.
        assert!(definition(PETSTORE, None, &req(false)).is_err());
    }

    #[tokio::test]
    async fn petstore_spec_to_tools_to_calls() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v3/openapi.yaml"))
            .respond_with(ResponseTemplate::new(200).set_body_string(PETSTORE))
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .and(path("/v3/pet/7"))
            .and(header("x-api-key", "s3cret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 7, "name": "Rex"})))
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .and(path("/v3/pet/findByStatus"))
            .and(query_param("status", "sold"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("x".repeat(super::super::MAX_RESPONSE + 10)),
            )
            .mount(&mock)
            .await;
        Mock::given(method("POST"))
            .and(path("/v3/pet"))
            .and(body_json(json!({"name": "Tom"})))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": 8})))
            .mount(&mock)
            .await;
        let spec_url = format!("{}/v3/openapi.yaml", mock.uri());

        // Through the generic entry point: fetch, detect, generate (read-only by default).
        let added = super::super::generate(
            &AddRequest {
                source: spec_url.clone(),
                ..req(false)
            },
            std::path::Path::new("."),
            std::path::Path::new("kor"),
        )
        .await
        .unwrap();
        assert_eq!(added.id, "swagger-petstore");
        assert_eq!(
            added.entry["args"],
            json!(["mcp", "run", "swagger-petstore"])
        );
        let d = added.definition.unwrap();
        let names: Vec<_> = d.tools.iter().map(|t| t.name.as_str()).collect();
        // Paths in sorted order (serde_json maps).
        assert_eq!(names, ["findPetsByStatus", "getPetById"]);
        assert_eq!(d.skipped, 2, "deletePet and addPet need --allow-writes");
        assert_eq!(
            d.endpoint.as_deref(),
            Some(format!("{}/v3", mock.uri()).as_str())
        );

        let secrets: Secrets = Arc::new(|n: &str| (n == "PET_TOKEN").then(|| "s3cret".to_owned()));
        let s = Server::new(d, secrets.clone());
        let out = s.call("getPetById", json!({"petId": 7})).await.unwrap();
        assert!(out.contains("HTTP 200") && out.contains("Rex"), "{out}");
        assert!(
            s.call("getPetById", json!({}))
                .await
                .unwrap_err()
                .contains("petId")
        );
        assert!(
            s.call("findPetsByStatus", json!({"status": "lost"}))
                .await
                .is_err()
        );
        // Over-large answers are capped by the serve loop.
        let big = super::super::serve::handle(
            &s,
            &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "findPetsByStatus", "arguments": {"status": "sold"}}}).to_string(),
        )
        .await
        .unwrap();
        assert!(
            big["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .ends_with("limit]")
        );
        // Without the secret the call is refused before any request.
        let none: Secrets = Arc::new(|_: &str| None);
        let s2 = Server::new(s.def.clone(), none);
        assert!(
            s2.call("getPetById", json!({"petId": 7}))
                .await
                .unwrap_err()
                .contains("kor vault add PET_TOKEN")
        );

        // With writes: addPet is a destructive tool sending the JSON body.
        let d = definition(PETSTORE, Some(&spec_url), &req(true)).unwrap();
        let add = d.tools.iter().find(|t| t.name == "addPet").unwrap();
        assert!(add.write && add.descriptor()["annotations"]["destructiveHint"] == true);
        assert_eq!(
            add.input_schema["properties"]["body"]["properties"]["children"]["items"]["description"],
            "recursive structure"
        );
        let s = Server::new(d, secrets);
        let out = s
            .call("addPet", json!({"body": {"name": "Tom"}}))
            .await
            .unwrap();
        assert!(out.contains("HTTP 201"), "{out}");
    }

    #[test]
    fn path_values_cannot_escape() {
        assert_eq!(encode("../admin?x=1"), "%2E%2E%2Fadmin%3Fx%3D1");
        assert_eq!(encode("çay"), "%C3%A7ay");
    }
}
