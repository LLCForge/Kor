//! Sharing tool definitions with the team server (§26.2 "takım sunucusuyla paylaşılır", §11):
//! `kor tool share` sends the user's definitions with `PUT <server>/team_tools`; `kor tool pull`
//! fetches the team's into `<kor home>/tools/team/`, where they load like project tools (never
//! `read`, always the permission check). Nothing is sent without an explicit command, the server is
//! the user's own (`[team] server`), and definitions with a secret in them are refused.

use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

use super::defs::ToolDef;

pub const PATH: &str = "/team_tools";
const MAX_TOOLS: usize = 200;

fn endpoint(server: &str) -> String {
    format!("{}{PATH}", server.trim_end_matches('/'))
}

/// Trusts the team CA and presents this member's certificate (mTLS) once `kor server join` ran.
fn client() -> Result<reqwest::Client, String> {
    crate::team_client::builder(Duration::from_secs(30))?
        .build()
        .map_err(|e| e.to_string())
}

/// Upload `defs`; returns how many were sent.
///
/// # Errors
/// A definition with a secret in it, or the request failed.
pub async fn share(server: &str, token: &str, defs: &[ToolDef]) -> Result<usize, String> {
    for d in defs {
        let text = toml::to_string(d).map_err(|e| e.to_string())?;
        let labels = kor_tools::secret_labels(&text);
        if !labels.is_empty() {
            return Err(format!(
                "refusing to share `{}`: {}",
                d.name,
                labels.join(", ")
            ));
        }
    }
    let r = client()?
        .put(endpoint(server))
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(json!({ "tools": defs }).to_string())
        .send()
        .await
        .map_err(|e| kor_core::net::describe(&e))?;
    if !r.status().is_success() {
        return Err(format!("team server answered {}", r.status()));
    }
    Ok(defs.len())
}

/// Download the team's definitions into `dir` (replacing what an earlier pull wrote); returns
/// their names and the ones rejected with the reason.
///
/// # Errors
/// The request failed or the answer is not `{ "tools": [...] }`.
pub async fn pull(
    server: &str,
    token: &str,
    dir: &Path,
) -> Result<(Vec<String>, Vec<String>), String> {
    let r = client()?
        .get(endpoint(server))
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| kor_core::net::describe(&e))?;
    if !r.status().is_success() {
        return Err(format!("team server answered {}", r.status()));
    }
    let v: Value = serde_json::from_str(&r.text().await.map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let list = v
        .get("tools")
        .and_then(Value::as_array)
        .ok_or("the answer has no `tools` list")?;
    let (mut ok, mut rejected) = (Vec::new(), Vec::new());
    let mut defs = Vec::new();
    for item in list.iter().take(MAX_TOOLS) {
        match serde_json::from_value::<ToolDef>(item.clone())
            .map_err(|e| e.to_string())
            .and_then(|d| d.validate().map(|()| d))
        {
            Ok(d) => defs.push(d),
            Err(e) => rejected.push(format!(
                "{}: {e}",
                item.get("name").and_then(Value::as_str).unwrap_or("?")
            )),
        }
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    // Replace the previous pull: stale team tools must not linger.
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "toml"))
        {
            let _ = std::fs::remove_file(e.path());
        }
    }
    for d in defs {
        let text = toml::to_string_pretty(&d).map_err(|e| e.to_string())?;
        std::fs::write(dir.join(format!("{}.toml", d.name)), text).map_err(|e| e.to_string())?;
        ok.push(d.name);
    }
    Ok((ok, rejected))
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    #[tokio::test]
    async fn share_and_pull_round_trip() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path(PATH))
            .and(header("authorization", "Bearer t0k"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"tools": [
                {"name": "git_short", "command": ["git", "log", "--oneline"]},
                {"name": "Bad Name", "command": ["git"]}
            ]})))
            .mount(&server)
            .await;
        let d: ToolDef = toml::from_str("name = \"gs\"\ncommand = \"git status\"").unwrap();
        assert_eq!(share(&server.uri(), "t0k", &[d]).await.unwrap(), 1);
        let secret: ToolDef = toml::from_str("name = \"leak\"\ncommand = \"curl -H Authorization:ghp_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa x\"").unwrap();
        assert!(
            share(&server.uri(), "t0k", &[secret])
                .await
                .unwrap_err()
                .contains("refusing")
        );
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("old.toml"), "x").unwrap();
        let (ok, rejected) = pull(&server.uri(), "t0k", dir.path()).await.unwrap();
        assert_eq!(ok, ["git_short"]);
        assert_eq!(rejected.len(), 1);
        assert!(
            dir.path().join("git_short.toml").is_file() && !dir.path().join("old.toml").exists()
        );
    }
}
