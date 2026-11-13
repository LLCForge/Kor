//! Turkish registrar API: Domain Name API (`domainresellerapi.com`, the reseller platform behind
//! many Turkish registrars and resellers). Endpoints and headers follow the vendor's open-source
//! client (`domainreseller/php-dna`, `DNARest.php`): `X-API-KEY` and `__reseller` headers, JSON
//! bodies. Kor reads domain details and DNS zones and, after the user confirms, sets name servers
//! or one record set (staged, then applied). Credentials are vault names in `business.toml`.

use serde::Serialize;
use serde_json::{Value, json};

use crate::site::Result;
use crate::site::http::{Http, Resp};

pub const DNA: &str = "https://api.domainresellerapi.com/api/v1";
pub const DNA_TEST: &str = "https://ote.domainresellerapi.com/api/v1";

/// Record types the zone API manages.
pub const RECORD_TYPES: [&str; 7] = ["A", "AAAA", "CNAME", "MX", "TXT", "NS", "SRV"];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DomainInfo {
    pub domain: String,
    pub status: String,
    pub expiration: String,
    pub name_servers: Vec<String>,
    pub locked: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Record {
    pub name: String,
    pub kind: String,
    pub ttl: u64,
    pub content: String,
}

/// A Domain Name API account.
#[derive(Debug, Clone)]
pub struct Dna<'a> {
    pub http: &'a Http,
    pub base: &'a str,
    pub reseller: &'a str,
    pub key: &'a str,
}

/// `example.com.tr`-like names only (no paths, queries or spaces reach the URL).
pub fn valid_domain(d: &str) -> bool {
    d.len() <= 253
        && d.contains('.')
        && d.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.chars().all(|c| c.is_alphanumeric() || c == '-')
        })
}

fn check(d: &str) -> Result<()> {
    if valid_domain(d) {
        Ok(())
    } else {
        Err(format!("invalid domain name `{d}`"))
    }
}

impl Dna<'_> {
    fn headers(&self) -> [(&'static str, String); 3] {
        [
            ("x-api-key", self.key.to_owned()),
            ("__reseller", self.reseller.to_owned()),
            ("accept", "application/json".into()),
        ]
    }

    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Resp> {
        let r = self
            .http
            .send(
                method,
                &format!("{}/{path}", self.base),
                &self.headers(),
                body.map(|b| ("application/json", b.to_string().into_bytes())),
            )
            .await?;
        if r.ok() {
            return Ok(r);
        }
        // A redirect means the credentials were refused.
        let why = if (300..400).contains(&r.status) {
            "invalid API credentials".to_owned()
        } else {
            r.json()
                .ok()
                .and_then(|v| {
                    v.get("message")
                        .or_else(|| v.pointer("/error/message"))
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| format!("HTTP {}", r.status))
        };
        Err(format!("Domain Name API: {why}"))
    }

    fn q(domain: &str) -> String {
        url::form_urlencoded::Serializer::new(String::new())
            .append_pair("domainName", domain)
            .finish()
    }

    pub async fn info(&self, domain: &str) -> Result<DomainInfo> {
        check(domain)?;
        let r = self
            .call(
                reqwest::Method::GET,
                &format!(
                    "domains/info?{}",
                    Self::q(domain).replacen("domainName", "DomainName", 1)
                ),
                None,
            )
            .await?;
        let v = r.json()?;
        let d = v.get("data").unwrap_or(&v);
        let s = |k: &str| {
            d.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        Ok(DomainInfo {
            domain: Some(s("domainName"))
                .filter(|x| !x.is_empty())
                .unwrap_or_else(|| domain.to_owned()),
            status: s("status"),
            expiration: s("expirationDate"),
            name_servers: d
                .get("nameservers")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            locked: d.get("lockStatus").and_then(Value::as_bool) == Some(true),
        })
    }

    /// The zone, one row per record value.
    pub async fn zone(&self, domain: &str) -> Result<Vec<Record>> {
        check(domain)?;
        let r = self
            .call(
                reqwest::Method::GET,
                &format!("domains/zones?{}", Self::q(domain)),
                None,
            )
            .await?;
        let v = r.json()?;
        let mut out = Vec::new();
        for set in v.as_array().into_iter().flatten() {
            let s = |k: &str| {
                set.get(k)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            };
            let ttl = set.get("ttl").and_then(Value::as_u64).unwrap_or_default();
            for rec in set
                .get("records")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if rec.get("disabled").and_then(Value::as_bool) == Some(true) {
                    continue;
                }
                out.push(Record {
                    name: s("name"),
                    kind: s("type"),
                    ttl,
                    content: rec
                        .get("content")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                });
            }
        }
        Ok(out)
    }

    /// Make the `name`/`kind` record set hold exactly `contents`, then apply the zone.
    pub async fn set_records(
        &self,
        domain: &str,
        name: &str,
        kind: &str,
        contents: &[String],
        ttl: u64,
    ) -> Result<()> {
        check(domain)?;
        let kind = kind.to_ascii_uppercase();
        if !RECORD_TYPES.contains(&kind.as_str()) {
            return Err(format!(
                "record type `{kind}` is not one of {}",
                RECORD_TYPES.join(", ")
            ));
        }
        if contents.is_empty() || contents.iter().any(|c| c.trim().is_empty()) {
            return Err("no record value".into());
        }
        let body = json!({ "zoneStruct": {
            "name": name, "ttl": ttl.clamp(1, 86_400), "type": kind, "contents": contents,
        }});
        let q = Self::q(domain);
        self.call(
            reqwest::Method::POST,
            &format!("domains/zones?{q}"),
            Some(&body),
        )
        .await?;
        self.call(
            reqwest::Method::POST,
            &format!("domains/zones/apply?{q}"),
            Some(&json!({})),
        )
        .await?;
        Ok(())
    }

    pub async fn set_name_servers(&self, domain: &str, ns: &[String]) -> Result<()> {
        check(domain)?;
        if ns.len() < 2 || ns.iter().any(|n| !valid_domain(n)) {
            return Err("give at least two valid name server host names".into());
        }
        self.call(
            reqwest::Method::PUT,
            "domains/dns/name-server",
            Some(&json!({ "domainName": domain, "nameServers": ns })),
        )
        .await
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    #[allow(clippy::many_single_char_names)]
    async fn info_zone_and_changes_against_mock() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/domains/info"))
            .and(query_param("DomainName", "ornek.com.tr"))
            .and(header("x-api-key", "key"))
            .and(header("__reseller", "res"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "domainName": "ornek.com.tr", "status": "Active", "lockStatus": true,
                "expirationDate": "2027-01-01T00:00:00", "nameservers": ["ns1.x.com", "ns2.x.com"]})))
            .mount(&s)
            .await;
        Mock::given(method("GET"))
            .and(path("/domains/zones"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"name": "www", "type": "A", "ttl": 300, "records": [{"content": "1.2.3.4"}, {"content": "5.6.7.8", "disabled": true}]}
            ])))
            .mount(&s)
            .await;
        Mock::given(method("POST"))
            .and(path("/domains/zones"))
            .and(body_json(json!({"zoneStruct": {"name": "@", "ttl": 3600, "type": "TXT", "contents": ["v=spf1 -all"]}})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(1)
            .mount(&s)
            .await;
        Mock::given(method("POST"))
            .and(path("/domains/zones/apply"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(1)
            .mount(&s)
            .await;
        Mock::given(method("PUT"))
            .and(path("/domains/dns/name-server"))
            .respond_with(
                ResponseTemplate::new(400).set_body_json(json!({"message": "Domain is locked"})),
            )
            .mount(&s)
            .await;
        let http = Http::new().unwrap();
        let base = s.uri();
        let d = Dna {
            http: &http,
            base: &base,
            reseller: "res",
            key: "key",
        };
        let i = d.info("ornek.com.tr").await.unwrap();
        assert!(i.locked && i.name_servers.len() == 2);
        let z = d.zone("ornek.com.tr").await.unwrap();
        assert_eq!(z.len(), 1);
        assert_eq!(
            (z[0].kind.as_str(), z[0].content.as_str()),
            ("A", "1.2.3.4")
        );
        d.set_records("ornek.com.tr", "@", "txt", &["v=spf1 -all".into()], 3600)
            .await
            .unwrap();
        let e = d
            .set_name_servers("ornek.com.tr", &["ns1.y.com".into(), "ns2.y.com".into()])
            .await
            .unwrap_err();
        assert!(e.contains("Domain is locked"));
        assert!(d.info("x/../y").await.is_err());
        assert!(
            d.set_records("ornek.com.tr", "@", "PTR", &["x".into()], 1)
                .await
                .is_err()
        );
        assert!(
            d.set_name_servers("ornek.com.tr", &["ns1.y.com".into()])
                .await
                .is_err()
        );
    }
}
