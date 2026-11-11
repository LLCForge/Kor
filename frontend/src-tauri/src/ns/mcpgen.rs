//! `mcpgen.*`: kaynaktan MCP sunucusu üretme ve sınama (`kor mcp add <kaynak>`, `kor mcp test`;
//! §8.3, ADR 0038). Üretim ve sınama çekirdekte (`kor_agent::mcp_gen`); burası yalnızca köprü.
//!
//! - `mcpgen.add({ source, id?, kind?, baseUrl?, allowWrites?, auth?, label? })` →
//!   `{ id, kind, tools, note }`. Veritabanı parolası adresten alınıp anahtar zincirine yazılır.
//! - `mcpgen.test(id)` → `{ id, connectMs, listMs, tools, probe, probeMs, probeOk, probeOutput, error }`
//!
//! Üretilen sunucular `kor mcp run <id>` ile başlar: uygulamanın yanındaki `kor` (paketle gelir),
//! yoksa PATH'teki `kor`.

use std::path::{Path, PathBuf};

use kor_agent::mcp_gen::{self, AddRequest, Auth, Kind};
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, root, s};
use crate::state::AppState;

pub const NS: &str = "mcpgen";

fn kor_exe() -> PathBuf {
    crate::kor_cli::path()
}

fn loc(root: &Path) -> Result<kor_agent::mcp_config::Locations, String> {
    Ok(kor_agent::mcp_config::Locations {
        kor_file: kor_config::user_dir()
            .ok_or("Kullanıcı ayar klasörü bulunamadı")?
            .join("mcp.json"),
        home: dirs::home_dir(),
        root: root.to_owned(),
        trusted: kor_runtime::is_trusted(root),
    })
}

fn text(v: &Value, key: &str, max: usize) -> Result<Option<String>, String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(Value::String(s)) if s.len() <= max => Ok(Some(s.trim().to_owned())),
        _ => Err(format!("`{key}` geçersiz")),
    }
}

/// Arayüzden gelen isteği doğrular (güven sınırı).
pub fn request(v: &Value) -> Result<AddRequest, String> {
    let source = text(v, "source", 4096)?.ok_or("Kaynak boş olamaz")?;
    let kind = match text(v, "kind", 20)? {
        Some(k) => Some(Kind::parse(&k).ok_or("Tür openapi, graphql, database ya da local olmalı")?),
        None => None,
    };
    let auth = text(v, "auth", 300)?.map(|a| Auth::parse(&a)).transpose()?;
    let label = text(v, "label", 80)?;
    Ok(AddRequest {
        source,
        id: text(v, "id", 40)?,
        kind,
        endpoint: text(v, "baseUrl", 2048)?,
        allow_writes: v.get("allowWrites").and_then(Value::as_bool) == Some(true),
        auth,
        label,
        row_limit: None,
        timeout_secs: None,
    })
}

pub async fn call(_ns: &str, name: &str, args: &[Value], _app: &AppHandle, state: &AppState) -> Option<Res> {
    if !matches!(name, "add" | "test") {
        return None;
    }
    let root = match root(state) {
        Ok(r) => r,
        Err(e) => return Some(Err(e)),
    };
    Some(match name {
        "add" => add(&root, args.first().unwrap_or(&Value::Null)).await,
        "test" => test(&root, args).await,
        _ => Err(format!("`mcpgen.{name}` henüz Kor arka ucunda yok")),
    })
}

async fn add(root: &Path, v: &Value) -> Res {
    let req = request(v)?;
    let loc = loc(root)?;
    let added = mcp_gen::generate(&req, root, &kor_exe()).await?;
    mcp_gen::install(&loc, &added, &kor_secrets::KeychainStore)?;
    Ok(json!({ "id": added.id, "kind": added.kind.key(), "tools": added.tools, "note": added.note }))
}

async fn test(root: &Path, args: &[Value]) -> Res {
    let id = s(args, 0, "mcpgen.test")?;
    let entry = kor_agent::mcp_config::entries(&loc(root)?)?
        .into_iter()
        .find(|e| e.id == id)
        .ok_or("Sunucu bulunamadı")?;
    if kor_agent::mcp_config::is_http(&entry.raw) {
        return Err("HTTP MCP sunucuları henüz bağlanmıyor; yalnızca stdio sunucuları denenebilir".into());
    }
    let env = |k: &str| std::env::var(k).ok();
    let x = |t: &str| kor_agent::mcp_tools::expand(t, &env);
    let spec = kor_agent::mcp_tools::ServerSpec {
        name: entry.id.clone(),
        command: x(entry.raw.get("command").and_then(Value::as_str).unwrap_or("")),
        args: kor_agent::mcp_config::strings(entry.raw.get("args")).iter().map(|a| x(a)).collect(),
        env: kor_agent::mcp_config::string_map(entry.raw.get("env"))
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), x(v))))
            .collect(),
        origin: entry.external.unwrap_or("mcp.json").to_owned(),
    };
    serde_json::to_value(mcp_gen::test(&spec, root).await).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_validated() {
        let r = request(&json!({"source": " sqlite:app.db ", "kind": "database", "allowWrites": true, "auth": "bearer:TOKEN"})).unwrap();
        assert_eq!((r.source.as_str(), r.kind, r.allow_writes), ("sqlite:app.db", Some(Kind::Database), true));
        for bad in [
            json!({}),
            json!({"source": ""}),
            json!({"source": "x", "kind": "redis"}),
            json!({"source": "x", "auth": "cookie:a:b"}),
            json!({"source": 5}),
            json!({"source": "x", "label": "l".repeat(81)}),
        ] {
            assert!(request(&bad).is_err(), "{bad}");
        }
    }
}
