//! Faz 8 acceptance (spec §14): a three-member team (admin, member, viewer) on one in-process
//! server over mTLS shares the same rules and graph; an outside system queues a task through the
//! API and gets a signed callback; SSO through a mock OIDC provider (and a mock GitHub API); the
//! vault, a shared customer report, the audit chain and the peer relay.
#![allow(clippy::too_many_lines, clippy::many_single_char_names)] // scenario tests read top to bottom

use std::sync::Arc;
use std::time::Duration;

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{self, EcdsaKeyPair, KeyPair as _, RsaKeyPair};
use base64::Engine as _;
use futures::{SinkExt as _, StreamExt as _};
use kor_server::store::Enrollment;
use kor_server::{Layout, OidcProvider, Role, ServerConfig, http};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const B64URL: base64::engine::GeneralPurpose = base64::engine::general_purpose::URL_SAFE_NO_PAD;

struct Team {
    dir: tempfile::TempDir,
    base: String,
    port: u16,
    admin: Enrollment,
    ali: Enrollment,
    veli: Enrollment,
    idp: MockServer,
    ec: EcdsaKeyPair,
    rsa: RsaKeyPair,
    running: http::Running,
}

fn client(e: Option<&Enrollment>, token: Option<&str>) -> reqwest::Client {
    let ca_src = e.map_or_else(String::new, |e| e.ca.clone());
    let mut b = reqwest::Client::builder().timeout(Duration::from_secs(20));
    if !ca_src.is_empty() {
        b = b.add_root_certificate(reqwest::Certificate::from_pem(ca_src.as_bytes()).unwrap());
    }
    if let Some(e) = e {
        b = b.identity(
            reqwest::Identity::from_pem(format!("{}{}", e.cert, e.key).as_bytes()).unwrap(),
        );
    }
    if let Some(t) = token {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert("authorization", format!("Bearer {t}").parse().unwrap());
        b = b.default_headers(h);
    }
    b.build().unwrap()
}

/// A client trusting the team CA but without a client certificate (a customer opening a link).
fn anonymous(ca: &str) -> reqwest::Client {
    reqwest::Client::builder()
        .add_root_certificate(reqwest::Certificate::from_pem(ca.as_bytes()).unwrap())
        .build()
        .unwrap()
}

fn jwt(alg: &str, kid: &str, claims: &Value, sign: impl Fn(&[u8]) -> Vec<u8>) -> String {
    let h = B64URL.encode(json!({ "alg": alg, "kid": kid, "typ": "JWT" }).to_string());
    let c = B64URL.encode(claims.to_string());
    let msg = format!("{h}.{c}");
    let s = B64URL.encode(sign(msg.as_bytes()));
    format!("{msg}.{s}")
}

impl Team {
    async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let idp = MockServer::start().await;
        let rng = SystemRandom::new();
        let ec_pkcs8 =
            EcdsaKeyPair::generate_pkcs8(&signature::ECDSA_P256_SHA256_FIXED_SIGNING, &rng)
                .unwrap();
        let ec = EcdsaKeyPair::from_pkcs8(
            &signature::ECDSA_P256_SHA256_FIXED_SIGNING,
            ec_pkcs8.as_ref(),
        )
        .unwrap();
        let rsa = RsaKeyPair::generate(aws_lc_rs::rsa::KeySize::Rsa2048).unwrap();
        let pt = ec.public_key().as_ref().to_vec();
        let n = rsa
            .public_key()
            .modulus()
            .big_endian_without_leading_zero()
            .to_vec();
        let e = rsa
            .public_key()
            .exponent()
            .big_endian_without_leading_zero()
            .to_vec();
        let issuer = idp.uri();
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": issuer,
                "jwks_uri": format!("{issuer}/jwks"),
                "token_endpoint": format!("{issuer}/token"),
                "device_authorization_endpoint": format!("{issuer}/device"),
            })))
            .mount(&idp)
            .await;
        Mock::given(method("GET"))
            .and(path("/jwks"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "keys": [
                { "kty": "EC", "crv": "P-256", "kid": "ec1", "use": "sig", "alg": "ES256",
                  "x": B64URL.encode(&pt[1..33]), "y": B64URL.encode(&pt[33..]) },
                { "kty": "RSA", "kid": "rsa1", "use": "sig", "alg": "RS256",
                  "n": B64URL.encode(&n), "e": B64URL.encode(&e) },
            ]})))
            .mount(&idp)
            .await;
        Mock::given(method("GET"))
            .and(path("/user/emails"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                { "email": "other@example.com", "primary": false, "verified": true },
                { "email": "Veli@Example.com", "primary": true, "verified": true },
            ])))
            .mount(&idp)
            .await;

        let (_, admin) =
            kor_server::init(dir.path(), "acme", "localhost", "admin@example.com", None)
                .await
                .unwrap();
        let layout = Layout::new(dir.path());
        let mut cfg = ServerConfig::load(&layout.config()).unwrap();
        cfg.listen = "127.0.0.1:0".into();
        cfg.allow_private_callbacks = true;
        cfg.providers = vec![
            OidcProvider {
                name: "kc".into(),
                kind: "oidc".into(),
                issuer: issuer.clone(),
                client_id: "kor".into(),
                ..OidcProvider::default()
            },
            OidcProvider {
                name: "gh".into(),
                kind: "github".into(),
                api: issuer.clone(),
                ..OidcProvider::default()
            },
        ];
        cfg.save(&layout.config()).unwrap();
        let app = http::App::open(dir.path()).await.unwrap();
        let ali = app
            .store
            .add_member(
                &cfg,
                &layout,
                "ali@example.com",
                Role::Member,
                "admin@example.com",
            )
            .await
            .unwrap();
        let veli = app
            .store
            .add_member(
                &cfg,
                &layout,
                "veli@example.com",
                Role::Viewer,
                "admin@example.com",
            )
            .await
            .unwrap();
        let running = http::start(app, "127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let port = running.addr.port();
        Self {
            dir,
            base: format!("https://localhost:{port}"),
            port,
            admin,
            ali,
            veli,
            idp,
            ec,
            rsa,
            running,
        }
    }

    fn as_(e: &Enrollment) -> reqwest::Client {
        client(Some(e), Some(&e.token))
    }

    fn id_token(&self, alg: &str, claims: &Value) -> String {
        let rng = SystemRandom::new();
        match alg {
            "ES256" => jwt("ES256", "ec1", claims, |m| {
                self.ec.sign(&rng, m).unwrap().as_ref().to_vec()
            }),
            _ => jwt("RS256", "rsa1", claims, |m| {
                let mut sig = vec![0; self.rsa.public_modulus_len()];
                self.rsa
                    .sign(&signature::RSA_PKCS1_SHA256, &rng, m, &mut sig)
                    .unwrap();
                sig
            }),
        }
    }
}

async fn status_of(r: reqwest::RequestBuilder) -> u16 {
    r.send().await.unwrap().status().as_u16()
}

async fn json_of(r: reqwest::RequestBuilder) -> Value {
    let resp = r.send().await.unwrap();
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    assert!(status.is_success(), "{status}: {v}");
    v
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_members_share_rules_and_graph() {
    let t = Team::start().await;
    let (admin, ali, veli) = (Team::as_(&t.admin), Team::as_(&t.ali), Team::as_(&t.veli));
    let b = &t.base;

    // Rules: the admin writes, all three read the same text.
    let rules = "# Team rules\n- small patches\n";
    json_of(
        admin
            .put(format!("{b}/api/v1/docs/rules/AGENTS.md"))
            .json(&json!({ "body": rules, "version": 0 })),
    )
    .await;
    for c in [&admin, &ali, &veli] {
        let d = json_of(c.get(format!("{b}/api/v1/docs/rules/AGENTS.md"))).await;
        assert_eq!(d["body"], rules);
        assert_eq!(d["version"], 1);
    }
    // Members may not change rules (they run on everyone's machine); viewers may not write at all.
    assert_eq!(
        status_of(
            ali.put(format!("{b}/api/v1/docs/rules/AGENTS.md"))
                .json(&json!({ "body": "x" }))
        )
        .await,
        403
    );

    // Graph: a member publishes, everyone sees the same graph; a stale write is refused.
    let graph = json!({ "nodes": 1247, "edges": 4891 }).to_string();
    json_of(
        ali.put(format!("{b}/api/v1/docs/graph/acme/app"))
            .json(&json!({ "body": graph, "version": 0 })),
    )
    .await;
    for c in [&admin, &ali, &veli] {
        assert_eq!(
            json_of(c.get(format!("{b}/api/v1/docs/graph/acme/app"))).await["body"],
            graph
        );
    }
    assert_eq!(
        status_of(
            admin
                .put(format!("{b}/api/v1/docs/graph/acme/app"))
                .json(&json!({ "body": "{}", "version": 0 }))
        )
        .await,
        409
    );
    assert_eq!(
        status_of(
            veli.put(format!("{b}/api/v1/docs/graph/acme/app"))
                .json(&json!({ "body": "{}" }))
        )
        .await,
        403
    );
    assert_eq!(
        status_of(admin.get(format!("{b}/api/v1/docs/rules/..%2Fx"))).await,
        404
    );

    // Templates are versioned: the old version stays readable.
    json_of(
        admin
            .put(format!("{b}/api/v1/docs/templates/new-endpoint.md"))
            .json(&json!({ "body": "v1" })),
    )
    .await;
    json_of(
        admin
            .put(format!("{b}/api/v1/docs/templates/new-endpoint.md"))
            .json(&json!({ "body": "v2" })),
    )
    .await;
    assert_eq!(
        json_of(ali.get(format!(
            "{b}/api/v1/docs/templates/new-endpoint.md?version=1"
        )))
        .await["body"],
        "v1"
    );
    let list = json_of(ali.get(format!("{b}/api/v1/docs/templates"))).await;
    assert_eq!(list["docs"][0]["version"], 2);

    // The existing client paths (`kor` team memory and `kor tool share`).
    let r = json_of(
        ali.put(format!("{b}/team_memory?repo=acme%2Fapp"))
            .json(&json!({ "entries": { "a.md": "hi" } })),
    )
    .await;
    assert!(r["checksum"].is_string());
    assert_eq!(
        status_of(
            ali.put(format!("{b}/team_memory?repo=acme%2Fapp"))
                .header("if-match", "\"stale\"")
                .json(&json!({ "entries": {} }))
        )
        .await,
        412
    );
    assert_eq!(
        json_of(veli.get(format!("{b}/team_memory?repo=acme%2Fapp"))).await["entries"]["a.md"],
        "hi"
    );
    assert_eq!(
        status_of(
            ali.put(format!("{b}/team_tools"))
                .json(&json!({ "tools": [] }))
        )
        .await,
        403
    );
    json_of(
        admin
            .put(format!("{b}/team_tools"))
            .json(&json!({ "tools": [{ "name": "lint" }] })),
    )
    .await;
    assert_eq!(
        json_of(veli.get(format!("{b}/team_tools"))).await["tools"][0]["name"],
        "lint"
    );
    t.running.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mtls_and_tokens_are_both_checked() {
    let t = Team::start().await;
    let b = &t.base;
    // Token without a client certificate: refused (require_client_cert).
    let no_cert = reqwest::Client::builder()
        .add_root_certificate(reqwest::Certificate::from_pem(t.ali.ca.as_bytes()).unwrap())
        .build()
        .unwrap();
    assert_eq!(
        status_of(
            no_cert
                .get(format!("{b}/api/v1/me"))
                .bearer_auth(&t.ali.token)
        )
        .await,
        401
    );
    // Certificate alone identifies the member.
    let me = json_of(client(Some(&t.ali), None).get(format!("{b}/api/v1/me"))).await;
    assert_eq!(me["email"], "ali@example.com");
    assert_eq!(me["role"], "member");
    // Ali's certificate with Veli's token: refused.
    assert_eq!(
        status_of(client(Some(&t.ali), Some(&t.veli.token)).get(format!("{b}/api/v1/me"))).await,
        403
    );
    // A forged token.
    assert_eq!(
        status_of(client(Some(&t.ali), Some("kmt_forged")).get(format!("{b}/api/v1/me"))).await,
        401
    );
    // Health and security headers.
    let r = client(Some(&t.ali), None)
        .get(format!("{b}/healthz"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.headers()["x-frame-options"], "DENY");
    assert!(
        r.headers()["strict-transport-security"]
            .to_str()
            .unwrap()
            .contains("max-age")
    );
    t.running.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sso_tokens_are_validated() {
    let t = Team::start().await;
    let b = &t.base;
    let c = client(Some(&t.ali), None);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .try_into()
        .unwrap();
    let now: i64 = now;
    let claims = |email: &str| json!({ "iss": t.idp.uri(), "aud": "kor", "exp": now + 300, "iat": now, "email": email, "email_verified": true });
    let exchange = |tok: String| {
        c.post(format!("{b}/api/v1/auth/token"))
            .json(&json!({ "provider": "kc", "id_token": tok }))
    };

    for alg in ["ES256", "RS256"] {
        let v = json_of(exchange(t.id_token(alg, &claims("ali@example.com")))).await;
        assert_eq!(v["role"], "member");
        let session = v["token"].as_str().unwrap().to_owned();
        let me = json_of(client(Some(&t.ali), Some(&session)).get(format!("{b}/api/v1/me"))).await;
        assert_eq!(me["email"], "ali@example.com");
    }
    // Wrong audience, wrong issuer, expired, tampered, `none`, not a member, another member's id.
    let mut bad = claims("ali@example.com");
    bad["aud"] = json!("someone-else");
    assert_eq!(status_of(exchange(t.id_token("ES256", &bad))).await, 401);
    let mut bad = claims("ali@example.com");
    bad["iss"] = json!("https://evil.example.com");
    assert_eq!(status_of(exchange(t.id_token("ES256", &bad))).await, 401);
    let mut bad = claims("ali@example.com");
    bad["exp"] = json!(now - 3600);
    assert_eq!(status_of(exchange(t.id_token("RS256", &bad))).await, 401);
    let good = t.id_token("ES256", &claims("ali@example.com"));
    let (head, rest) = good.split_once('.').unwrap();
    let sig = rest.rsplit_once('.').unwrap().1;
    let forged = format!(
        "{head}.{}.{sig}",
        B64URL.encode(claims("admin@example.com").to_string())
    );
    assert_eq!(status_of(exchange(forged)).await, 401);
    let none = format!(
        "{}.{}.",
        B64URL.encode(r#"{"alg":"none"}"#),
        B64URL.encode(claims("ali@example.com").to_string())
    );
    assert_eq!(status_of(exchange(none)).await, 401);
    assert_eq!(
        status_of(exchange(
            t.id_token("ES256", &claims("stranger@example.com"))
        ))
        .await,
        403
    );
    assert_eq!(
        status_of(exchange(t.id_token("ES256", &claims("veli@example.com")))).await,
        403
    );

    // GitHub: the verified primary e-mail from the API.
    let v = json_of(
        client(Some(&t.veli), None)
            .post(format!("{b}/api/v1/auth/token"))
            .json(&json!({ "provider": "gh", "access_token": "gho_test" })),
    )
    .await;
    assert_eq!(v["email"], "veli@example.com");
    assert_eq!(v["role"], "viewer");

    let p = json_of(c.get(format!("{b}/api/v1/auth/providers"))).await;
    assert_eq!(
        p["providers"][0]["deviceEndpoint"],
        format!("{}/device", t.idp.uri())
    );
    t.running.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn task_via_api_gets_a_signed_callback() {
    let t = Team::start().await;
    let b = &t.base;
    let hook = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/hook"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&hook)
        .await;

    // An outside system (a CI job holding Ali's credentials) queues a task.
    let ali = Team::as_(&t.ali);
    let created = ali
        .post(format!("{b}/api/v1/queue"))
        .json(&json!({ "prompt": "fix the failing test", "repo": "acme/app", "callback": format!("{}/hook", hook.uri()) }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status().as_u16(), 201);
    let created: Value = created.json().await.unwrap();
    let (id, secret) = (
        created["id"].as_str().unwrap(),
        created["secret"].as_str().unwrap(),
    );

    // A viewer cannot take work; the admin's machine is the worker.
    assert_eq!(
        status_of(Team::as_(&t.veli).post(format!("{b}/api/v1/queue/claim"))).await,
        403
    );
    let admin = Team::as_(&t.admin);
    let job = json_of(admin.post(format!("{b}/api/v1/queue/claim"))).await;
    assert_eq!(job["id"], id);
    assert_eq!(
        status_of(admin.post(format!("{b}/api/v1/queue/claim"))).await,
        204
    );
    json_of(
        admin
            .post(format!("{b}/api/v1/queue/{id}/finish"))
            .json(&json!({ "ok": true, "result": "diff --git a/x b/x" })),
    )
    .await;
    assert_eq!(
        json_of(ali.get(format!("{b}/api/v1/queue/{id}"))).await["state"],
        "done"
    );

    // The callback arrives, signed with the task's secret.
    let mut got = Vec::new();
    for _ in 0..50 {
        got = hook.received_requests().await.unwrap_or_default();
        if !got.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let req = got.first().expect("callback delivered");
    let body: Value = serde_json::from_slice(&req.body).unwrap();
    assert_eq!(body["id"], id);
    assert_eq!(body["state"], "done");
    let sig = req
        .headers
        .get("x-kor-signature")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(sig, format!("sha256={}", http::sign(secret, &req.body)));

    // A callback to a non-https URL is refused when private callbacks are off (checked in unit tests);
    // here an invalid one is refused outright.
    assert_eq!(
        status_of(
            ali.post(format!("{b}/api/v1/queue"))
                .json(&json!({ "prompt": "x", "callback": "ftp://x" }))
        )
        .await,
        400
    );
    t.running.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn vault_report_share_audit_and_offboarding() {
    let t = Team::start().await;
    let b = &t.base;
    let (admin, ali, veli) = (Team::as_(&t.admin), Team::as_(&t.ali), Team::as_(&t.veli));

    json_of(
        admin
            .put(format!("{b}/api/v1/vault/DATABASE_URL"))
            .json(&json!({ "value": "postgres://u:p@db/x" })),
    )
    .await;
    assert_eq!(
        status_of(
            ali.put(format!("{b}/api/v1/vault/X"))
                .json(&json!({ "value": "y" }))
        )
        .await,
        403
    );
    assert_eq!(
        json_of(ali.get(format!("{b}/api/v1/vault/DATABASE_URL"))).await["value"],
        "postgres://u:p@db/x"
    );
    assert_eq!(
        status_of(veli.get(format!("{b}/api/v1/vault/DATABASE_URL"))).await,
        403
    );

    // A customer report, shared with someone outside the team through an expiring link.
    let report = "# Site report\n<script>alert(1)</script>\n";
    json_of(
        ali.put(format!("{b}/api/v1/docs/reports/customer-x/2026-11"))
            .json(&json!({ "body": report })),
    )
    .await;
    let link = json_of(
        ali.post(format!("{b}/api/v1/shares"))
            .json(&json!({ "key": "customer-x/2026-11", "hours": 24 })),
    )
    .await;
    let r = anonymous(&t.ali.ca)
        .get(format!("{b}{}", link["path"].as_str().unwrap()))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    assert!(
        r.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/plain")
    );
    assert!(
        r.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("default-src 'none'")
    );
    assert_eq!(r.text().await.unwrap(), report);
    assert_eq!(
        status_of(anonymous(&t.ali.ca).get(format!("{b}/share/ksh_nope"))).await,
        404
    );
    assert_eq!(
        status_of(
            ali.post(format!("{b}/api/v1/shares"))
                .json(&json!({ "ns": "rules", "key": "AGENTS.md" }))
        )
        .await,
        400
    );

    // The audit log: admin only, chain intact, the vault opening is in it.
    assert_eq!(status_of(ali.get(format!("{b}/api/v1/audit"))).await, 403);
    let a = json_of(admin.get(format!("{b}/api/v1/audit?limit=1000"))).await;
    assert_eq!(a["chain"]["ok"], true);
    let actions: Vec<&str> = a["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["action"].as_str())
        .collect();
    for want in [
        "member.add",
        "vault.put",
        "vault.open",
        "doc.put",
        "share.create",
        "share.open",
    ] {
        assert!(actions.contains(&want), "{want} missing from {actions:?}");
    }

    // Offboarding: removing Ali kills his certificate and token at once.
    let app = http::App::open(t.dir.path()).await.unwrap();
    app.store
        .remove_member("ali@example.com", "admin@example.com")
        .await
        .unwrap();
    assert_eq!(status_of(ali.get(format!("{b}/api/v1/me"))).await, 401);
    t.running.stop();
}

fn tls(e: &Enrollment) -> Arc<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    for c in kor_server::pki::certs_from_pem(e.ca.as_bytes()).unwrap() {
        roots.add(c).unwrap();
    }
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    Arc::new(
        rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_client_auth_cert(
                kor_server::pki::certs_from_pem(e.cert.as_bytes()).unwrap(),
                kor_server::pki::key_from_pem(e.key.as_bytes()).unwrap(),
            )
            .unwrap(),
    )
}

async fn ws(
    t: &Team,
    e: &Enrollment,
    mode: &str,
    name: &str,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>,
    String,
> {
    let tcp = tokio::net::TcpStream::connect(("127.0.0.1", t.port))
        .await
        .unwrap();
    let s = tokio_rustls::TlsConnector::from(tls(e))
        .connect(
            rustls::pki_types::ServerName::try_from("localhost").unwrap(),
            tcp,
        )
        .await
        .unwrap();
    let mut req = format!("wss://localhost:{}/api/v1/relay/{mode}/{name}", t.port)
        .into_client_request()
        .unwrap();
    req.headers_mut().insert(
        "authorization",
        format!("Bearer {}", e.token).parse().unwrap(),
    );
    tokio_tungstenite::client_async(req, s)
        .await
        .map(|(w, _)| w)
        .map_err(|e| e.to_string())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn relay_pipes_two_members() {
    let t = Team::start().await;
    let mut listener = ws(&t, &t.admin, "listen", "peer:abcd").await.unwrap();
    // Viewers cannot use the relay; another member cannot take over the name.
    assert!(ws(&t, &t.veli, "dial", "peer:abcd").await.is_err());
    assert!(ws(&t, &t.ali, "listen", "peer:abcd").await.is_err());
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut dialer = ws(&t, &t.ali, "dial", "peer:abcd").await.unwrap();
    let hello = listener.next().await.unwrap().unwrap();
    assert_eq!(
        hello.into_text().unwrap().as_str(),
        r#"{"type":"connected"}"#
    );
    dialer
        .send(Message::Binary(b"noise-frame-1".to_vec().into()))
        .await
        .unwrap();
    assert_eq!(
        listener.next().await.unwrap().unwrap().into_data().as_ref(),
        b"noise-frame-1"
    );
    listener.send(Message::Text("reply".into())).await.unwrap();
    assert_eq!(
        dialer
            .next()
            .await
            .unwrap()
            .unwrap()
            .into_text()
            .unwrap()
            .as_str(),
        "reply"
    );
    // Nobody left listening: a second dial is refused.
    assert!(ws(&t, &t.ali, "dial", "peer:abcd").await.is_err());
    t.running.stop();
}
