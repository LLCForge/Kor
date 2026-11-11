//! Team memory sync: `PUT <base>/team_memory?repo=` to a server the user runs.
//! Off unless `KOR_TEAM_MEMORY_URL` (or legacy `TEAM_MEMORY_SYNC_URL`) is set; Kor has
//! no default endpoint and never uploads anywhere on its own.
//! Files that match the secret scan are not uploaded.

use std::collections::BTreeMap;

use serde_json::{Value, json};

pub const SYNC_PATH: &str = "/team_memory";

/// The configured sync server, if any.
pub fn base_url() -> Option<String> {
    crate::env_compat("KOR_TEAM_MEMORY_URL", "TEAM_MEMORY_SYNC_URL").filter(|s| !s.trim().is_empty())
}

pub fn endpoint(base: &str, repo: &str) -> String {
    use std::fmt::Write as _;
    let mut repo_q = String::new();
    for b in repo.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                repo_q.push(b as char);
            }
            _ => {
                let _ = write!(repo_q, "%{b:02X}");
            }
        }
    }
    format!("{}{SYNC_PATH}?repo={repo_q}", base.trim_end_matches('/'))
}

/// Build the PUT body. Refuses the whole upload if any file contains a secret.
pub fn prepare_body(entries: &[(impl AsRef<str>, impl AsRef<str>)]) -> Result<Value, String> {
    let mut map = BTreeMap::new();
    for (name, text) in entries {
        let labels = kor_tools::secret_labels(text.as_ref());
        if !labels.is_empty() {
            return Err(format!(
                "refusing to upload {}: {}",
                name.as_ref(),
                labels.join(", ")
            ));
        }
        map.insert(name.as_ref().to_owned(), text.as_ref().to_owned());
    }
    Ok(json!({ "entries": map }))
}

pub async fn push(base: &str, repo: &str, token: &str, body: &Value) -> Result<String, String> {
    let client = kor_core::net::client_builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let response = client
        .put(endpoint(base, repo))
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| kor_core::net::describe(&e))?;
    let status = response.status();
    let text = response.text().await.map_err(|e| e.to_string())?;
    if status.as_u16() == 412 {
        return Err("ETag mismatch".into());
    }
    if !status.is_success() {
        return Err(format!("team memory upload failed ({status})"));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn secret_blocks_the_body() {
        let key = format!("sk-ant-api03-{}AA", "a".repeat(93));
        let err = prepare_body(&[("note.md", key.as_str())]).unwrap_err();
        assert!(err.contains("anthropic-api-key"), "{err}");
        let ok = prepare_body(&[("note.md", "team prefers small patches")]).unwrap();
        assert_eq!(ok["entries"]["note.md"], "team prefers small patches");
    }

    #[tokio::test]
    async fn put_uses_the_source_path() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/team_memory"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"checksum":"abc"}"#))
            .mount(&server)
            .await;
        let body = prepare_body(&[("a.md", "hello")]).unwrap();
        let text = push(&server.uri(), "acme/repo", "token", &body)
            .await
            .unwrap();
        assert!(text.contains("abc"), "{text}");
        let url = endpoint(&server.uri(), "acme/repo");
        assert!(
            url.contains("/team_memory?repo=acme%2Frepo"),
            "{url}"
        );
    }
}
