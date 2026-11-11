//! `kor vault` (spec §6.6): named secrets (`DATABASE_URL`, …) in the OS keychain.
//!
//! Values live under the keychain account `vault:<NAME>`. Keychains cannot list their entries,
//! so the names (never the values) are kept in one more entry, `vault:#index`, one per line.
//! Nothing here writes to a file; callers get names back, values only through [`entries`]
//! (for injecting them as environment variables of agent commands).

use crate::{ExposeSecret, MAX_SECRET_LEN, SecretError, SecretStore, SecretString};

const INDEX: &str = "vault:#index";
/// Upper bound on stored names, so a hostile `.env` cannot fill the keychain.
pub const MAX_ENTRIES: usize = 512;
const MAX_NAME: usize = 128;

/// Environment variable style: `[A-Za-z_][A-Za-z0-9_]*`, at most 128 bytes.
pub fn valid_name(name: &str) -> bool {
    let mut b = name.bytes();
    name.len() <= MAX_NAME
        && b.next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        && b.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

fn account(name: &str) -> Result<String, SecretError> {
    if valid_name(name) {
        Ok(format!("vault:{name}"))
    } else {
        Err(SecretError::InvalidName(name.to_owned()))
    }
}

/// Stored names, sorted.
pub fn names(store: &dyn SecretStore) -> Result<Vec<String>, SecretError> {
    let Some(index) = store.get(INDEX)? else {
        return Ok(Vec::new());
    };
    let mut out: Vec<String> = index
        .expose_secret()
        .lines()
        .filter(|n| valid_name(n))
        .map(str::to_owned)
        .collect();
    out.sort();
    out.dedup();
    Ok(out)
}

fn write_index(store: &dyn SecretStore, names: &[String]) -> Result<(), SecretError> {
    if names.is_empty() {
        store.delete(INDEX).map(|_| ())
    } else {
        store.set(INDEX, &SecretString::from(names.join("\n")))
    }
}

/// Store or replace `name`. Returns whether it replaced an existing entry.
pub fn add(store: &dyn SecretStore, name: &str, value: &SecretString) -> Result<bool, SecretError> {
    let acc = account(name)?;
    let v = value.expose_secret();
    if v.is_empty() || v.len() > MAX_SECRET_LEN || v.chars().any(char::is_control) {
        return Err(SecretError::InvalidSecret);
    }
    let mut all = names(store)?;
    let existed = all.iter().any(|n| n == name);
    if !existed && all.len() >= MAX_ENTRIES {
        return Err(SecretError::Keychain(format!(
            "vault is full ({MAX_ENTRIES} entries)"
        )));
    }
    store.set(&acc, value)?;
    if !existed {
        all.push(name.to_owned());
        all.sort();
        write_index(store, &all)?;
    }
    Ok(existed)
}

/// Delete `name`. Returns whether it existed.
pub fn remove(store: &dyn SecretStore, name: &str) -> Result<bool, SecretError> {
    let acc = account(name)?;
    let had = store.delete(&acc)?;
    let mut all = names(store)?;
    let before = all.len();
    all.retain(|n| n != name);
    if all.len() != before {
        write_index(store, &all)?;
    }
    Ok(had || all.len() != before)
}

/// One secret by name (`kor daemon`: notifier tokens, webhook HMAC secrets).
pub fn get(store: &dyn SecretStore, name: &str) -> Result<Option<SecretString>, SecretError> {
    store.get(&account(name)?)
}

/// Every stored secret, for the agent's command environment. Entries whose value vanished
/// from the keychain are skipped.
pub fn entries(store: &dyn SecretStore) -> Result<Vec<(String, SecretString)>, SecretError> {
    let mut out = Vec::new();
    for n in names(store)? {
        if let Some(v) = store.get(&account(&n)?)? {
            out.push((n, v));
        }
    }
    Ok(out)
}

/// A parsed `.env` file: importable pairs and the names (or line numbers) that were skipped.
#[derive(Debug, Default)]
pub struct DotEnv {
    pub entries: Vec<(String, SecretString)>,
    pub skipped: Vec<String>,
}

/// Parse `KEY=value` lines (`export ` prefix, `#` comments, single or double quotes).
/// shortcut: multi-line quoted values and `${VAR}` expansion are not supported (skipped / kept
/// literally); upgrade if users import such files.
pub fn parse_dotenv(text: &str) -> DotEnv {
    let mut out = DotEnv::default();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((k, v)) = line.split_once('=') else {
            out.skipped.push(format!("#{}", i + 1));
            continue;
        };
        let k = k.trim();
        let v = v.trim();
        let value = match v.chars().next() {
            Some(q @ ('"' | '\'')) => {
                let Some(end) = v[1..].find(q) else {
                    out.skipped.push(k.to_owned());
                    continue;
                };
                &v[1..=end]
            }
            // Unquoted: an inline comment starts at ` #`.
            _ => v.split(" #").next().unwrap_or_default().trim_end(),
        };
        if !valid_name(k)
            || value.is_empty()
            || value.len() > MAX_SECRET_LEN
            || value.chars().any(char::is_control)
        {
            out.skipped.push(if valid_name(k) {
                k.to_owned()
            } else {
                format!("#{}", i + 1)
            });
            continue;
        }
        // A later line wins, like shells and dotenv loaders.
        out.entries.retain(|(n, _)| n != k);
        out.entries
            .push((k.to_owned(), SecretString::from(value.to_owned())));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryStore;

    fn s(v: &str) -> SecretString {
        SecretString::from(v.to_owned())
    }

    #[test]
    fn add_list_remove_keep_values_out_of_the_index() {
        let store = MemoryStore::default();
        assert!(names(&store).unwrap().is_empty());
        assert!(!add(&store, "DATABASE_URL", &s("postgres://u:p@h/db")).unwrap());
        assert!(!add(&store, "API_TOKEN", &s("t0ken")).unwrap());
        assert!(add(&store, "API_TOKEN", &s("t1ken")).unwrap());
        assert_eq!(names(&store).unwrap(), ["API_TOKEN", "DATABASE_URL"]);
        let index = store.get(INDEX).unwrap().unwrap();
        assert!(!index.expose_secret().contains("t1ken"));
        let all = entries(&store).unwrap();
        assert_eq!(all[0].1.expose_secret(), "t1ken");

        assert!(remove(&store, "API_TOKEN").unwrap());
        assert!(!remove(&store, "API_TOKEN").unwrap());
        assert_eq!(names(&store).unwrap(), ["DATABASE_URL"]);
        assert!(remove(&store, "DATABASE_URL").unwrap());
        assert!(store.get(INDEX).unwrap().is_none());
    }

    #[test]
    fn rejects_bad_names_and_values() {
        let store = MemoryStore::default();
        for bad in ["", "1X", "A-B", "a b", "x=y", "../x"] {
            assert!(matches!(
                add(&store, bad, &s("v")),
                Err(SecretError::InvalidName(_))
            ));
        }
        assert!(matches!(
            add(&store, "OK", &s("a\nb")),
            Err(SecretError::InvalidSecret)
        ));
        assert!(matches!(
            add(&store, "OK", &s("")),
            Err(SecretError::InvalidSecret)
        ));
        assert!(valid_name("_x9") && !valid_name(&"A".repeat(129)));
    }

    #[test]
    fn dotenv_parsing() {
        let text = "# comment\n\
                    export DB_URL=postgres://u:p@h/db\n\
                    QUOTED=\"a b # not a comment\"\n\
                    SINGLE='x'\n\
                    INLINE=value # trailing\n\
                    EMPTY=\n\
                    bad line\n\
                    1BAD=x\n\
                    OPEN=\"unterminated\n\
                    DB_URL=second\n";
        let d = parse_dotenv(text);
        let got: Vec<(&str, &str)> = d
            .entries
            .iter()
            .map(|(k, v)| (k.as_str(), v.expose_secret()))
            .collect();
        assert_eq!(
            got,
            [
                ("QUOTED", "a b # not a comment"),
                ("SINGLE", "x"),
                ("INLINE", "value"),
                ("DB_URL", "second"),
            ]
        );
        assert_eq!(d.skipped, ["EMPTY", "#7", "#8", "OPEN"]);
    }
}
