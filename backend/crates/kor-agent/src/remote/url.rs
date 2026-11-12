//! `kor db connect <url>`: a connection URL becomes a `[db.<id>]` entry in the user's
//! `connections.toml`. The password never goes to the file; the caller stores it in the keychain.

use std::fmt::Write as _;
use std::path::Path;

use super::Connections;

/// What a URL describes: the id to save it under, the TOML section, and the password (if any).
#[derive(Debug)]
pub struct FromUrl {
    pub id: String,
    section: String,
    pub password: Option<String>,
}

fn decode(s: &str) -> Result<String, String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s.get(i + 1..i + 3).ok_or("bad %-escape in the URL")?;
            out.push(u8::from_str_radix(hex, 16).map_err(|_| "bad %-escape in the URL")?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| "the URL is not UTF-8".into())
}

/// `a-z0-9-`, at most 40 characters, from the database name (or the given id).
fn slug(s: &str) -> String {
    let mut id: String = s
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    id = id.trim_matches('-').chars().take(40).collect();
    id.trim_end_matches('-').to_owned()
}

fn q(s: &str) -> String {
    toml::Value::String(s.to_owned()).to_string()
}

/// Parse `postgres://`, `postgresql://`, `mysql://`, `mariadb://`, `mssql://`, `sqlserver://`
/// or `sqlite:<file>` (relative to the project root). `?sslmode=` is kept for Postgres; other
/// query parameters are refused rather than silently dropped.
pub fn parse(url: &str, id: Option<&str>) -> Result<FromUrl, String> {
    if let Some(file) = url
        .strip_prefix("sqlite://")
        .or_else(|| url.strip_prefix("sqlite:"))
    {
        let file = decode(file)?;
        if file.is_empty() {
            return Err("sqlite: needs a file".into());
        }
        let stem = Path::new(&file)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned());
        let id = slug(id.or(stem.as_deref()).unwrap_or("sqlite"));
        let section = format!("[db.{id}]\nkind = \"sqlite\"\nfile = {}\n", q(&file));
        return Ok(FromUrl {
            id,
            section,
            password: None,
        });
    }
    let u = reqwest::Url::parse(url).map_err(|e| format!("not a database URL: {e}"))?;
    let kind = match u.scheme() {
        "postgres" | "postgresql" => "postgres",
        "mysql" | "mariadb" => "mysql",
        "mssql" | "sqlserver" => "mssql",
        s => {
            return Err(format!(
                "unsupported scheme `{s}` (postgres, mysql, mssql, sqlite)"
            ));
        }
    };
    let host = u
        .host_str()
        .filter(|h| !h.is_empty())
        .ok_or("the URL has no host")?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let database = decode(u.path().trim_start_matches('/'))?;
    let mut section = format!("kind = \"{kind}\"\nhost = {}\n", q(host));
    if let Some(p) = u.port() {
        let _ = writeln!(section, "port = {p}");
    }
    if !database.is_empty() {
        let _ = writeln!(section, "database = {}", q(&database));
    }
    if !u.username().is_empty() {
        let _ = writeln!(section, "user = {}", q(&decode(u.username())?));
    }
    for (k, v) in u.query_pairs() {
        match (kind, k.as_ref()) {
            ("postgres", "sslmode") => {
                let _ = writeln!(section, "sslmode = {}", q(&v));
            }
            _ => {
                return Err(format!(
                    "unsupported URL parameter `{k}`; edit connections.toml for it"
                ));
            }
        }
    }
    let base = if database.is_empty() {
        host
    } else {
        database.as_str()
    };
    let id = slug(id.unwrap_or(base));
    let password = u
        .password()
        .map(decode)
        .transpose()?
        .filter(|p| !p.is_empty());
    Ok(FromUrl {
        section: format!("[db.{id}]\n{section}"),
        id,
        password,
    })
}

/// Append the entry to `file` (created if missing) after validating the whole result. An
/// existing entry with the same id is refused: editing it is the user's call.
pub fn save(file: &Path, entry: &FromUrl) -> Result<(), String> {
    let old = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", file.display())),
    };
    if Connections::parse(&old)?.db.contains_key(&entry.id) {
        return Err(format!(
            "`{}` is already in {}; use `kor db connect {}` or pick another id with --id",
            entry.id,
            file.display(),
            entry.id
        ));
    }
    let sep = if old.is_empty() || old.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    let new = format!("{old}{sep}\n{}", entry.section);
    Connections::parse(&new)?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(file, new).map_err(|e| format!("{}: {e}", file.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_become_entries_without_the_password() {
        let e = parse(
            "postgresql://app:p%40ss@db.example.com:5433/shop?sslmode=require",
            None,
        )
        .unwrap();
        assert_eq!(e.id, "shop");
        assert_eq!(e.password.as_deref(), Some("p@ss"));
        assert!(!e.section.contains("p@ss") && !e.section.contains("p%40ss"));
        let c = Connections::parse(&e.section).unwrap();
        let d = &c.db["shop"];
        assert_eq!(
            (d.host.as_deref(), d.port, d.user.as_deref()),
            (Some("db.example.com"), Some(5433), Some("app"))
        );
        assert_eq!(d.sslmode.as_deref(), Some("require"));

        let e = parse("mysql://root@127.0.0.1/My_DB", Some("Main DB")).unwrap();
        assert_eq!((e.id.as_str(), e.password), ("main-db", None));
        assert_eq!(
            Connections::parse(&e.section).unwrap().db["main-db"].kind,
            super::super::DbKind::Mysql
        );

        let e = parse("sqlite:data/app.db", None).unwrap();
        assert_eq!(e.id, "app");
        assert_eq!(
            Connections::parse(&e.section).unwrap().db["app"]
                .file
                .as_deref(),
            Some("data/app.db")
        );

        assert!(parse("redis://x/1", None).is_err());
        assert!(parse("mysql://h/db?ssl=1", None).is_err());
        // Host names that `ssh` or a shell could read as options fail validation on save.
        let bad = parse("postgres://-oProxyCommand=x/db", None);
        assert!(bad.is_err() || Connections::parse(&bad.unwrap().section).is_err());
    }

    #[test]
    fn save_appends_and_refuses_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("connections.toml");
        std::fs::write(&f, "[ssh.box]\nhost = \"box.lan\"").unwrap();
        let e = parse("postgres://u@h/main", None).unwrap();
        save(&f, &e).unwrap();
        let c = Connections::parse(&std::fs::read_to_string(&f).unwrap()).unwrap();
        assert!(c.ssh.contains_key("box") && c.db.contains_key("main"));
        assert!(save(&f, &e).unwrap_err().contains("already"));
    }
}
