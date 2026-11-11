//! Deterministic matcher (§30.1 stage 1): normalization, fuzzy token similarity, phrase scores
//! and parameter candidates. No model, no network, no allocation-heavy indexes: the catalog has a
//! few thousand short phrases, a linear scan takes well under a millisecond.

/// Words that carry no command meaning (tr/en/fr, already folded).
const STOPWORDS: &[&str] = &[
    // tr
    "lutfen",
    "bana",
    "beni",
    "benim",
    "bir",
    "bu",
    "su",
    "o",
    "mi",
    "mu",
    "misin",
    "musun",
    "miyim",
    "acaba",
    "ve",
    "ile",
    "icin",
    "de",
    "da",
    "ki",
    "ya",
    "simdi",
    "hemen",
    "bi",
    "biraz",
    "rica",
    "ederim",
    "sen",
    "siz",
    "bizim",
    "olan",
    "gibi",
    "artik",
    "hadi",
    "bakalim",
    "sey",
    "seyi",
    "iste",
    "istiyorum",
    "ister",
    "yap",
    "yapar",
    "yapabilir",
    "edebilir",
    "eder",
    // en
    "please",
    "can",
    "could",
    "would",
    "you",
    "me",
    "my",
    "mine",
    "the",
    "a",
    "an",
    "to",
    "for",
    "of",
    "i",
    "it",
    "is",
    "are",
    "am",
    "be",
    "this",
    "that",
    "these",
    "those",
    "and",
    "or",
    "in",
    "on",
    "at",
    "with",
    "now",
    "just",
    "some",
    "want",
    "wanna",
    "need",
    "pls",
    "plz",
    "kindly",
    "hey",
    "ok",
    "okay",
    "let",
    "lets",
    "us",
    "our",
    "do",
    "does",
    "did",
    "go",
    "get",
    "give",
    "your",
    "we",
    "into",
    "from",
    "by",
    "up",
    "out",
    "about",
    "there",
    "here",
    "them",
    "its",
    // fr
    "s",
    "il",
    "vous",
    "plait",
    "moi",
    "mon",
    "ma",
    "mes",
    "le",
    "la",
    "les",
    "du",
    "des",
    "un",
    "une",
    "je",
    "tu",
    "est",
    "ce",
    "cette",
    "ces",
    "et",
    "ou",
    "en",
    "au",
    "aux",
    "pour",
    "sur",
    "dans",
    "avec",
    "peux",
    "pouvez",
    "veux",
    "voudrais",
    "merci",
    "svp",
    "qui",
    "que",
    "on",
    "nous",
    "notre",
    "me",
    "te",
    "se",
    "y",
    "l",
    "d",
    "j",
    "c",
    "n",
    "m",
    "t",
    "qu",
];

/// Lowercase and fold Turkish and French letters to ASCII (`İ`/`ı` → `i`, `ş` → `s`, `é` → `e`).
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().flat_map(char::to_lowercase) {
        match c {
            '\u{307}' => {} // the dot `İ` leaves after lowercasing
            'ı' | 'î' | 'ï' | 'í' | 'ì' => out.push('i'),
            'ğ' => out.push('g'),
            'ş' => out.push('s'),
            'ç' => out.push('c'),
            'ö' | 'ô' | 'ó' | 'ò' => out.push('o'),
            'ü' | 'û' | 'ù' | 'ú' => out.push('u'),
            'é' | 'è' | 'ê' | 'ë' => out.push('e'),
            'à' | 'â' | 'á' | 'ä' => out.push('a'),
            'œ' => out.push_str("oe"),
            'æ' => out.push_str("ae"),
            '’' | '‘' | '`' | '´' => out.push('\''),
            _ => out.push(c),
        }
    }
    out
}

/// A word as typed, cut at an apostrophe: Turkish case suffixes (`sandbox'ı` → `sandbox`) are
/// dropped, French elisions (`l'historique` → `historique`) keep the word.
fn unapostrophe(word: &str) -> &str {
    let w = word.trim_matches(|c: char| matches!(c, '\'' | '’' | '‘' | '`' | '´'));
    match w.find(['\'', '’', '‘', '`', '´']) {
        Some(i) => {
            let (head, tail) = w.split_at(i);
            if head.chars().count() <= 2 {
                tail.trim_start_matches(['\'', '’', '‘', '`', '´'])
            } else {
                head
            }
        }
        None => w,
    }
}

/// Raw words of `text`, without surrounding punctuation and apostrophe suffixes.
pub fn words(text: &str) -> Vec<&str> {
    text.split_whitespace()
        .map(|w| {
            unapostrophe(w.trim_matches(|c: char| {
                matches!(
                    c,
                    ',' | ';' | '?' | '!' | '"' | '(' | ')' | '[' | ']' | '«' | '»' | '“' | '”'
                )
            }))
        })
        .map(|w| w.trim_end_matches(['.', ':']))
        .filter(|w| !w.is_empty())
        .collect()
}

/// A typed word that names something (a path, a symbol, a version, a host) rather than an action:
/// `src/main.rs`, `parseConfig`, `STRIPE_KEY`, `v1.2`, `deploy@10.0.0.5`, `42`.
pub fn is_value_like(word: &str) -> bool {
    let chars: Vec<char> = word.chars().collect();
    let camel = chars
        .windows(2)
        .any(|w| w[0].is_lowercase() && w[1].is_uppercase());
    camel
        || word.contains(['/', '\\', '.', '@', ':', '_', '=', '~'])
        || chars.iter().any(char::is_ascii_digit)
}

/// Normalized content tokens of free text (stopwords and value-like words removed).
pub fn tokens(text: &str) -> Vec<String> {
    words(text)
        .into_iter()
        .filter(|w| !is_value_like(w))
        .flat_map(|w| {
            fold(w)
                .split(|c: char| !c.is_alphanumeric())
                .filter(|t| t.chars().count() >= 2 && !STOPWORDS.contains(t))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// `true` for words the matcher ignores.
pub fn is_stopword(folded: &str) -> bool {
    STOPWORDS.contains(&folded)
}

/// Levenshtein distance with an early exit above `max`.
fn lev(a: &[char], b: &[char], max: usize) -> usize {
    if a.len().abs_diff(b.len()) > max {
        return max + 1;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        let mut row_min = cur[0];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
            row_min = row_min.min(cur[j + 1]);
        }
        if row_min > max {
            return max + 1;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Similarity of two normalized tokens in `[0, 1]`: equal, one a stem of the other (Turkish
/// suffixes, plurals), or a typo away.
pub fn sim(a: &str, b: &str) -> f32 {
    if a == b {
        return 1.0;
    }
    let (ac, bc): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let (s, l) = if ac.len() <= bc.len() {
        (&ac, &bc)
    } else {
        (&bc, &ac)
    };
    let prefix = l.starts_with(s);
    if prefix && s.len() >= 4 {
        return 0.9;
    }
    if prefix && s.len() == 3 && l.len() <= 5 {
        return 0.85;
    }
    // Short Turkish verb stems: `aç` ~ `açar`, `al` ~ `alır`.
    if prefix && s.len() == 2 && l.len() <= 4 {
        return 0.8;
    }
    if s.len() >= 5 {
        let max = if s.len() >= 8 { 2 } else { 1 };
        if lev(s, l, max) <= max {
            return 0.8;
        }
        // A stem with a typo followed by a suffix: `hatyi` ~ `hatasini` is too far, `duzeltr` ~ `duzelt` is not.
        if lev(s, &l[..s.len()], 1) <= 1 {
            return 0.7;
        }
    }
    0.0
}

/// Best similarity of `t` against `set`.
pub fn best(t: &str, set: &[String]) -> f32 {
    set.iter().map(|x| sim(t, x)).fold(0.0, f32::max)
}

/// Score of `input` against one phrase: the geometric mean of how much of the phrase the input
/// says (recall) and how much of the input the phrase explains (precision, `weights` per input
/// word), so a long coding request that happens to contain two command words stays low.
/// Precision weight of an input word the whole catalog never uses.
pub const OOV_WEIGHT: f32 = 1.5;

pub fn score(input: &[String], weights: &[f32], phrase: &[String]) -> f32 {
    if input.is_empty() || phrase.is_empty() {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)] // a few words
    let recall = phrase.iter().map(|p| best(p, input)).sum::<f32>() / phrase.len() as f32;
    let w = |k: usize| weights.get(k).copied().unwrap_or(1.0);
    let total: f32 = (0..input.len()).map(w).sum();
    let precision = input
        .iter()
        .enumerate()
        .map(|(k, i)| w(k) * best(i, phrase))
        .sum::<f32>()
        / total;
    (recall * precision).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folding_and_tokens() {
        assert_eq!(
            fold("İstanbul ığdır çğşöü éàœ"),
            "istanbul igdir cgsou eaoe"
        );
        assert_eq!(tokens("Sandbox'ı test et lütfen"), ["sandbox", "test"]);
        assert_eq!(
            tokens("explique l'historique de src/main.rs"),
            ["explique", "historique"]
        );
        assert_eq!(words("main.rs'i açıkla."), ["main.rs", "açıkla"]);
        assert!(
            is_value_like("parseConfig") && is_value_like("v1.2") && is_value_like("STRIPE_KEY")
        );
        assert!(!is_value_like("KVKK") && !is_value_like("maliyet"));
    }

    #[test]
    fn similarity_handles_suffixes_and_typos() {
        assert!((sim("maliyetimi", "maliyet") - 0.9).abs() < f32::EPSILON);
        assert!(sim("hatayi", "hata") > 0.8);
        assert!(sim("son", "sonu") > 0.8);
        assert!(sim("son", "sonraki") < 0.1);
        assert!(sim("sandbx", "sandbox") >= 0.8);
        assert!(sim("graf", "graph") < 0.1);
        assert!(sim("commitle", "commit") > 0.8);
    }

    #[test]
    fn a_coding_request_scores_below_a_command_phrase() {
        let phrase = tokens("son hatayı düzelt");
        let yes = score(&tokens("son hatayı düzeltir misin"), &[], &phrase);
        let no_in = tokens("parse fonksiyonundaki null hatasını düzelt");
        let no = score(&no_in, &[1.5, 1.5, 1.5, 1.0, 1.0], &phrase);
        assert!(yes > 0.9, "{yes}");
        assert!(no < 0.5, "{no}");
    }
}
