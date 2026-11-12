//! §4.2: AWS Signature Version 4 for Bedrock and SageMaker
//! (<https://docs.aws.amazon.com/IAM/latest/UserGuide/reference_sigv-create-signed-request.html>),
//! and the credentials it signs with: `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` /
//! `AWS_SESSION_TOKEN`, else the `[default]` (or `AWS_PROFILE`) section of
//! `~/.aws/credentials`. The secret never leaves this process; only the signature is sent.

use std::fmt::Write as _;

use kor_secrets::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};

#[derive(Clone)]
pub struct Credentials {
    pub access_key: String,
    pub secret_key: SecretString,
    pub session_token: Option<SecretString>,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("access_key", &self.access_key)
            .finish_non_exhaustive()
    }
}

impl Credentials {
    /// From the environment, then from the shared credentials file.
    pub fn load() -> Option<Self> {
        Self::from_env(|k| std::env::var(k).ok()).or_else(|| {
            let path = std::env::var_os("AWS_SHARED_CREDENTIALS_FILE")
                .map(std::path::PathBuf::from)
                .or_else(|| dirs::home_dir().map(|h| h.join(".aws").join("credentials")))?;
            let text = std::fs::read_to_string(path).ok()?;
            let profile = std::env::var("AWS_PROFILE").unwrap_or_else(|_| "default".into());
            Self::from_ini(&text, &profile)
        })
    }

    fn from_env(var: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let access_key = var("AWS_ACCESS_KEY_ID").filter(|v| !v.is_empty())?;
        let secret = var("AWS_SECRET_ACCESS_KEY").filter(|v| !v.is_empty())?;
        Some(Self {
            access_key,
            secret_key: SecretString::from(secret),
            session_token: var("AWS_SESSION_TOKEN")
                .filter(|v| !v.is_empty())
                .map(SecretString::from),
        })
    }

    fn from_ini(text: &str, profile: &str) -> Option<Self> {
        let mut in_section = false;
        let (mut id, mut secret, mut token) = (None, None, None);
        for line in text.lines().map(str::trim) {
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                in_section = name.trim() == profile;
                continue;
            }
            if !in_section {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let v = v.trim().to_owned();
                match k.trim() {
                    "aws_access_key_id" => id = Some(v),
                    "aws_secret_access_key" => secret = Some(v),
                    "aws_session_token" => token = Some(v),
                    _ => {}
                }
            }
        }
        Some(Self {
            access_key: id.filter(|v| !v.is_empty())?,
            secret_key: SecretString::from(secret.filter(|v| !v.is_empty())?),
            session_token: token.filter(|v| !v.is_empty()).map(SecretString::from),
        })
    }
}

/// The region: `AWS_REGION`, then `AWS_DEFAULT_REGION`.
pub fn region_from_env() -> Option<String> {
    ["AWS_REGION", "AWS_DEFAULT_REGION"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.trim().is_empty()))
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

fn hmac(key: &[u8], msg: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    inner.update(msg);
    let mut outer = Sha256::new();
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

/// RFC 3986 encoding of one path segment, as SigV4 wants it (`-_.~` and alphanumerics kept).
pub fn encode_segment(s: &str) -> String {
    s.bytes().fold(String::new(), |mut out, b| {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
        out
    })
}

/// A request to sign: the path is already encoded once (as sent); the canonical form encodes it
/// again, which is what every AWS service except S3 expects.
#[derive(Debug)]
pub struct Request<'a> {
    pub method: &'a str,
    pub host: &'a str,
    pub path: &'a str,
    pub query: &'a str,
    pub body: &'a [u8],
    /// Extra headers to sign (lower-case names), besides `host` and the `x-amz-*` ones.
    pub headers: &'a [(&'a str, &'a str)],
}

/// The headers to add: `x-amz-date`, `x-amz-content-sha256`, the session token if any, and
/// `authorization`. `amz_date` is `YYYYMMDD'T'HHMMSS'Z'`.
pub fn sign(
    req: &Request<'_>,
    creds: &Credentials,
    region: &str,
    service: &str,
    amz_date: &str,
) -> Vec<(String, String)> {
    let date = &amz_date[..8.min(amz_date.len())];
    let payload = hex(&Sha256::digest(req.body));
    let mut headers: Vec<(String, String)> = vec![
        ("host".into(), req.host.to_owned()),
        ("x-amz-date".into(), amz_date.to_owned()),
    ];
    if service != "service" {
        headers.push(("x-amz-content-sha256".into(), payload.clone()));
    }
    if let Some(t) = &creds.session_token {
        headers.push(("x-amz-security-token".into(), t.expose_secret().to_owned()));
    }
    for (k, v) in req.headers {
        headers.push(((*k).to_lowercase(), v.trim().to_owned()));
    }
    headers.sort();
    let canonical_headers: String = headers.iter().fold(String::new(), |mut s, (k, v)| {
        let _ = writeln!(s, "{k}:{v}");
        s
    });
    let signed: Vec<&str> = headers.iter().map(|(k, _)| k.as_str()).collect();
    let signed = signed.join(";");
    let path = if req.path.is_empty() { "/" } else { req.path };
    let canonical_path: String = path
        .split('/')
        .map(encode_segment)
        .collect::<Vec<_>>()
        .join("/");
    let canonical = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        req.method, canonical_path, req.query, canonical_headers, signed, payload
    );
    let scope = format!("{date}/{region}/{service}/aws4_request");
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        hex(&Sha256::digest(canonical.as_bytes()))
    );
    let k_date = hmac(
        format!("AWS4{}", creds.secret_key.expose_secret()).as_bytes(),
        date.as_bytes(),
    );
    let k_region = hmac(&k_date, region.as_bytes());
    let k_service = hmac(&k_region, service.as_bytes());
    let k_signing = hmac(&k_service, b"aws4_request");
    let signature = hex(&hmac(&k_signing, to_sign.as_bytes()));
    let mut out: Vec<(String, String)> = headers
        .into_iter()
        .filter(|(k, _)| k.starts_with("x-amz-"))
        .collect();
    out.push((
        "authorization".into(),
        format!(
            "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed}, Signature={signature}",
            creds.access_key
        ),
    ));
    out
}

/// `x-amz-date` for now.
pub fn now() -> String {
    chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example() -> Credentials {
        Credentials {
            access_key: "AKIDEXAMPLE".into(),
            secret_key: SecretString::from("wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".to_owned()),
            session_token: None,
        }
    }

    /// AWS's own test suite, case `get-vanilla` (service name `service`, no payload header).
    #[test]
    fn matches_the_aws_test_suite() {
        let req = Request {
            method: "GET",
            host: "example.amazonaws.com",
            path: "/",
            query: "",
            body: b"",
            headers: &[],
        };
        let h = sign(&req, &example(), "us-east-1", "service", "20150830T123600Z");
        let auth = &h.iter().find(|(k, _)| k == "authorization").unwrap().1;
        assert_eq!(
            auth,
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, \
             SignedHeaders=host;x-amz-date, \
             Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
        );
    }

    #[test]
    fn hmac_matches_rfc_4231() {
        // RFC 4231 test case 2.
        assert_eq!(
            hex(&hmac(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn credentials_from_env_and_file() {
        let env = |k: &str| match k {
            "AWS_ACCESS_KEY_ID" => Some("AKID".to_owned()),
            "AWS_SECRET_ACCESS_KEY" => Some("s".to_owned()),
            _ => None,
        };
        let c = Credentials::from_env(env).unwrap();
        assert_eq!(c.access_key, "AKID");
        assert!(c.session_token.is_none());
        let ini = "[other]\naws_access_key_id = X\n[default]\naws_access_key_id = AKID2\naws_secret_access_key = s2\naws_session_token = t\n";
        let c = Credentials::from_ini(ini, "default").unwrap();
        assert_eq!(c.access_key, "AKID2");
        assert!(c.session_token.is_some());
        assert!(Credentials::from_ini(ini, "missing").is_none());
        // Session tokens are signed too.
        let req = Request {
            method: "POST",
            host: "h",
            path: "/x",
            query: "",
            body: b"{}",
            headers: &[],
        };
        let h = sign(&req, &c, "eu-west-1", "bedrock", "20250101T000000Z");
        assert!(h.iter().any(|(k, _)| k == "x-amz-security-token"));
        assert!(h.iter().any(|(k, v)| k == "authorization"
            && v.contains("x-amz-content-sha256;x-amz-date;x-amz-security-token")));
    }
}
