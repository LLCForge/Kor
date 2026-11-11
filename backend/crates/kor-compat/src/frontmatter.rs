//! Minimal YAML front matter: flat `key: value`, inline `[a, b]` and block `- item` lists.
//! Enough for SKILL.md, agents, commands and Cursor `.mdc` rules without a YAML dependency.

use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Front {
    map: BTreeMap<String, Vec<String>>,
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    for q in ['"', '\''] {
        if let Some(inner) = s.strip_prefix(q).and_then(|r| r.strip_suffix(q)) {
            return inner.to_owned();
        }
    }
    s.to_owned()
}

impl Front {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.map
            .get(key)
            .and_then(|v| v.first())
            .map(String::as_str)
            .filter(|s| !s.is_empty())
    }

    pub fn bool(&self, key: &str) -> bool {
        self.get(key)
            .is_some_and(|v| v.eq_ignore_ascii_case("true"))
    }

    /// A list key's items as written: a scalar is one item, never split on commas.
    pub fn items(&self, key: &str) -> &[String] {
        self.map.get(key).map_or(&[], Vec::as_slice)
    }

    /// A list key; a scalar is split on commas (`tools: Read, Grep`).
    pub fn list(&self, key: &str) -> Vec<String> {
        let Some(v) = self.map.get(key) else {
            return Vec::new();
        };
        let items: Vec<String> = if v.len() == 1 {
            v[0].split(',').map(unquote).collect()
        } else {
            v.clone()
        };
        items.into_iter().filter(|s| !s.is_empty()).collect()
    }
}

/// Split `---\n…\n---\n body` into front matter and body. No front matter → empty map.
pub fn split(text: &str) -> (Front, &str) {
    let t = text.strip_prefix('\u{feff}').unwrap_or(text);
    let Some(rest) = t
        .strip_prefix("---")
        .filter(|r| r.starts_with('\n') || r.starts_with("\r\n"))
    else {
        return (Front::default(), t);
    };
    let Some(end) = rest.find("\n---") else {
        return (Front::default(), t);
    };
    let header = &rest[..end];
    let after = &rest[end + 4..];
    let body = after.split_once('\n').map_or("", |(_, b)| b);

    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut current: Option<String> = None;
    for line in header.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let (Some(key), Some(item)) = (&current, trimmed.strip_prefix("- ")) {
            map.entry(key.clone()).or_default().push(unquote(item));
            continue;
        }
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        if line.starts_with([' ', '\t']) {
            continue; // nested structures are not supported; ignore rather than misread
        }
        let key = k.trim().to_owned();
        let v = v.trim();
        let values = if let Some(inner) = v.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            inner
                .split(',')
                .map(unquote)
                .filter(|s| !s.is_empty())
                .collect()
        } else if v.is_empty() {
            Vec::new()
        } else {
            vec![unquote(v)]
        };
        current = Some(key.clone());
        map.insert(key, values);
    }
    (Front { map }, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_shapes() {
        let text = "---\nname: reviewer\ndescription: \"Reviews code: carefully\"\ntools: Read, Grep\nglobs: [\"src/**/*.rs\", '*.toml']\nalwaysApply: true\npaths:\n  - docs/**\n  - \"*.md\"\nnested:\n  key: x\n---\nBody line\n";
        let (f, body) = split(text);
        assert_eq!(f.get("name"), Some("reviewer"));
        assert_eq!(f.get("description"), Some("Reviews code: carefully"));
        assert_eq!(f.list("tools"), ["Read", "Grep"]);
        assert_eq!(f.list("globs"), ["src/**/*.rs", "*.toml"]);
        assert!(f.bool("alwaysApply"));
        assert_eq!(f.list("paths"), ["docs/**", "*.md"]);
        assert_eq!(f.get("nested"), None);
        assert_eq!(body, "Body line\n");
    }

    #[test]
    fn no_front_matter_and_crlf() {
        let (f, body) = split("# Title\n---\nnot front matter");
        assert_eq!(f, Front::default());
        assert!(body.starts_with("# Title"));
        let (f, body) = split("---\r\ndescription: x\r\n---\r\nbody");
        assert_eq!(f.get("description"), Some("x"));
        assert_eq!(body, "body");
    }
}
