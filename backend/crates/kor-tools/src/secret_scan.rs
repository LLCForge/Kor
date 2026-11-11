//! High-confidence secret scan for team memory.
//!
//! Rule ids and shapes follow `services/teamMemorySync/secretScanner.ts`, which
//! cites the public gitleaks config. Only distinctive prefixes are included.
//! A match blocks the write; nothing is uploaded anywhere.

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

struct Rule {
    id: &'static str,
    source: &'static str,
    case_insensitive: bool,
}

static RULES: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    SOURCES
        .iter()
        .filter_map(|r| {
            regex::RegexBuilder::new(r.source)
                .case_insensitive(r.case_insensitive)
                .build()
                .ok()
                .map(|re| (r.id, re))
        })
        .collect()
});

/// Every rule in one automaton: one pass tells whether [`redact`] has anything to do.
static ANY: LazyLock<Option<regex::RegexSet>> = LazyLock::new(|| {
    regex::RegexSet::new(SOURCES.iter().map(|r| {
        if r.case_insensitive {
            format!("(?i){}", r.source)
        } else {
            r.source.to_owned()
        }
    }))
    .ok()
});

const SOURCES: &[Rule] = &[
        Rule {
            id: "aws-access-token",
            source: r"\b((?:A3T[A-Z0-9]|AKIA|ASIA|ABIA|ACCA)[A-Z2-7]{16})\b",
            case_insensitive: false,
        },
        Rule {
            id: "anthropic-api-key",
            source: r"\b(sk-ant-api03-[a-zA-Z0-9_\-]{93}AA)(?:[\x60\x27\x22\s;]|\\[nr]|$)",
            case_insensitive: false,
        },
        Rule {
            id: "anthropic-admin-api-key",
            source: r"\b(sk-ant-admin01-[a-zA-Z0-9_\-]{93}AA)(?:[\x60\x27\x22\s;]|\\[nr]|$)",
            case_insensitive: false,
        },
        Rule {
            id: "openai-api-key",
            source: r"\b(sk-(?:proj|svcacct|admin)-(?:[A-Za-z0-9_-]{74}|[A-Za-z0-9_-]{58})T3BlbkFJ(?:[A-Za-z0-9_-]{74}|[A-Za-z0-9_-]{58})\b|sk-[a-zA-Z0-9]{20}T3BlbkFJ[a-zA-Z0-9]{20})(?:[\x60\x27\x22\s;]|\\[nr]|$)",
            case_insensitive: false,
        },
        Rule {
            id: "github-pat",
            source: r"ghp_[0-9a-zA-Z]{36}",
            case_insensitive: false,
        },
        Rule {
            id: "github-fine-grained-pat",
            source: r"github_pat_\w{82}",
            case_insensitive: false,
        },
        Rule {
            id: "github-oauth",
            source: r"gho_[0-9a-zA-Z]{36}",
            case_insensitive: false,
        },
        Rule {
            id: "gitlab-pat",
            source: r"glpat-[\w-]{20}",
            case_insensitive: false,
        },
        Rule {
            id: "slack-bot-token",
            source: r"xoxb-[0-9]{10,13}-[0-9]{10,13}[a-zA-Z0-9-]*",
            case_insensitive: false,
        },
        Rule {
            id: "npm-access-token",
            source: r"\b(npm_[a-zA-Z0-9]{36})(?:[\x60\x27\x22\s;]|\\[nr]|$)",
            case_insensitive: false,
        },
        Rule {
            id: "stripe-access-token",
            source: r"\b((?:sk|rk)_(?:test|live|prod)_[a-zA-Z0-9]{10,99})(?:[\x60\x27\x22\s;]|\\[nr]|$)",
            case_insensitive: false,
        },
        Rule {
            id: "private-key",
            source: r"-----BEGIN[ A-Z0-9_-]{0,100}PRIVATE KEY(?: BLOCK)?-----[\s\S-]{64,}?-----END[ A-Z0-9_-]{0,100}PRIVATE KEY(?: BLOCK)?-----",
            case_insensitive: true,
        },
    ];

/// Rule ids that matched. Empty when the content is safe to store.
pub fn scan(content: &str) -> Vec<&'static str> {
    RULES
        .iter()
        .filter(|(_, re)| re.is_match(content))
        .map(|(id, _)| *id)
        .collect()
}

/// Every match as `(rule id, byte offset of the key)` — `kor audit --security` turns the offset
/// into a line (ADR 0045). The ids are gitleaks rule ids, so its findings dedup with ours.
pub fn find(content: &str) -> Vec<(&'static str, usize)> {
    if ANY.as_ref().is_some_and(|s| !s.is_match(content)) {
        return Vec::new();
    }
    RULES
        .iter()
        .flat_map(|(id, re)| {
            re.captures_iter(content).filter_map(move |c| {
                c.get(1).or_else(|| c.get(0)).map(|m| (*id, m.start()))
            })
        })
        .collect()
}

/// `content` with every match replaced by `[secret:<rule id>]` (the knowledge base, §31.3).
pub fn redact(content: &str) -> String {
    let mut out = content.to_owned();
    if ANY.as_ref().is_some_and(|s| !s.is_match(content)) {
        return out;
    }
    for (id, re) in RULES.iter() {
        if re.is_match(&out) {
            let tag = format!("[secret:{id}]");
            // Some rules also match the delimiter after the key (group 1 is the key): keep it.
            out = re
                .replace_all(&out, |c: &regex::Captures<'_>| {
                    let all = c.get(0).map_or("", |m| m.as_str());
                    c.get(1)
                        .map_or_else(|| tag.clone(), |k| all.replacen(k.as_str(), &tag, 1))
                })
                .into_owned();
        }
    }
    out
}

/// Error text when `path` is inside `team` and `content` contains a secret.
/// `None` when the path is not team memory or the content is clean.
pub(crate) fn block_team_write(path: &Path, team: &Path, content: &str) -> Option<String> {
    let path = crate::without_verbatim(path);
    let team = crate::without_verbatim(team);
    if !path.starts_with(&team) {
        return None;
    }
    let hits = scan(content);
    if hits.is_empty() {
        return None;
    }
    Some(format!(
        "Content contains potential secrets ({}) and cannot be written to team memory. \
         Team memory is shared with all repository collaborators. \
         Remove the sensitive content and try again.",
        hits.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_and_github_keys_match_plain_text_does_not() {
        let key = format!("sk-ant-api03-{}AA", "a".repeat(93));
        assert!(scan(&key).contains(&"anthropic-api-key"));
        assert!(scan("ghp_abcdefghijklmnopqrstuvwxyz0123456789").contains(&"github-pat"));
        assert!(scan("the user prefers rustfmt").is_empty());
        assert!(scan("sk-ant-api03-too-short").is_empty());
    }

    #[test]
    fn redact_masks_only_the_key() {
        let key = format!("sk-ant-api03-{}AA", "a".repeat(93));
        assert_eq!(
            redact(&format!("key=\"{key}\" ok")),
            "key=\"[secret:anthropic-api-key]\" ok"
        );
        assert_eq!(redact("plain text"), "plain text");
    }
}
