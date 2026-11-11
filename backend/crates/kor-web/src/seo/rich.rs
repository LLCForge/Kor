//! Structured data (JSON-LD) validation and the rich-result sub-report (spec §29.5, §29.16).
//!
//! The field lists follow Google Search Central's structured data documentation (required /
//! recommended properties per rich result type, as published in 2025). Types Google retired
//! from rich results (HowTo; FAQ outside government/health sites; the sitelinks search box) are
//! still validated, with a note saying no rich result is shown for them.

use serde::Serialize;
use serde_json::Value;

/// `(type, required, recommended, note)`.
pub type TypeRule = (
    &'static str,
    &'static [&'static str],
    &'static [&'static str],
    Option<&'static str>,
);

/// A field `a|b` means "one of"; `a.b` is a nested path.
pub const TYPES: &[TypeRule] = &[
    (
        "Product",
        &["name", "offers|review|aggregateRating"],
        &[
            "image",
            "description",
            "brand",
            "sku",
            "offers.price",
            "offers.priceCurrency",
            "offers.availability",
        ],
        None,
    ),
    (
        "Article",
        &[],
        &[
            "headline",
            "image",
            "datePublished",
            "dateModified",
            "author",
        ],
        None,
    ),
    (
        "NewsArticle",
        &[],
        &[
            "headline",
            "image",
            "datePublished",
            "dateModified",
            "author",
        ],
        None,
    ),
    (
        "BlogPosting",
        &[],
        &[
            "headline",
            "image",
            "datePublished",
            "dateModified",
            "author",
        ],
        None,
    ),
    (
        "FAQPage",
        &["mainEntity"],
        &[],
        Some(
            "FAQ rich results are shown only for well-known government and health sites (Google, 2023)",
        ),
    ),
    ("BreadcrumbList", &["itemListElement"], &[], None),
    (
        "Event",
        &["name", "startDate", "location"],
        &[
            "endDate",
            "eventStatus",
            "image",
            "offers",
            "organizer",
            "description",
            "performer",
        ],
        None,
    ),
    (
        "LocalBusiness",
        &["name", "address"],
        &[
            "telephone",
            "url",
            "geo",
            "openingHoursSpecification",
            "priceRange",
            "image",
        ],
        None,
    ),
    (
        "Review",
        &["itemReviewed", "author", "reviewRating.ratingValue"],
        &[],
        None,
    ),
    (
        "HowTo",
        &["name", "step"],
        &[],
        Some("HowTo rich results were retired by Google in 2023"),
    ),
    (
        "VideoObject",
        &["name", "thumbnailUrl", "uploadDate"],
        &["description", "contentUrl|embedUrl", "duration"],
        None,
    ),
    (
        "Recipe",
        &["name", "image"],
        &[
            "author",
            "datePublished",
            "recipeIngredient",
            "recipeInstructions",
            "aggregateRating",
        ],
        None,
    ),
    (
        "JobPosting",
        &[
            "title",
            "description",
            "datePosted",
            "hiringOrganization",
            "jobLocation|applicantLocationRequirements",
        ],
        &["validThrough", "employmentType", "baseSalary"],
        None,
    ),
    ("Organization", &[], &["name", "url", "logo"], None),
    (
        "WebSite",
        &[],
        &["name", "url"],
        Some("the sitelinks search box was retired by Google in 2024"),
    ),
];

/// One problem with one JSON-LD block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Issue {
    Invalid(String),
    NoContext,
    NoType,
    MissingRequired { ty: String, field: String },
    MissingRecommended { ty: String, field: String },
    Retired { ty: String, note: String },
}

/// Per-page rich result status: types present and valid, types present but missing required
/// fields (with what to add).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Eligibility {
    pub eligible: Vec<String>,
    pub blocked: Vec<(String, Vec<String>)>,
}

fn get<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = v;
    for part in path.split('.') {
        cur = match cur {
            Value::Array(a) => a.first()?.get(part)?,
            _ => cur.get(part)?,
        };
    }
    match cur {
        Value::Null => None,
        Value::String(s) if s.trim().is_empty() => None,
        Value::Array(a) if a.is_empty() => None,
        _ => Some(cur),
    }
}

fn types_of(v: &Value) -> Vec<String> {
    match v.get("@type") {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|t| t.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

/// Every typed node: top level, arrays and `@graph`.
fn nodes(v: &Value, out: &mut Vec<Value>) {
    match v {
        Value::Array(a) => a.iter().for_each(|x| nodes(x, out)),
        Value::Object(o) => {
            if let Some(g) = o.get("@graph") {
                nodes(g, out);
            }
            if o.contains_key("@type") {
                out.push(v.clone());
            }
        }
        _ => {}
    }
}

/// Check one `<script type="application/ld+json">` body.
pub fn check(raw: &str, elig: &mut Eligibility) -> Vec<Issue> {
    let v: Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(e) => return vec![Issue::Invalid(e.to_string())],
    };
    let mut issues = Vec::new();
    let has_context = |v: &Value| match v {
        Value::Array(a) => a.iter().all(|x| x.get("@context").is_some()),
        _ => v.get("@context").is_some(),
    };
    if !has_context(&v) {
        issues.push(Issue::NoContext);
    }
    let mut all = Vec::new();
    nodes(&v, &mut all);
    if all.is_empty() {
        issues.push(Issue::NoType);
    }
    for node in &all {
        for ty in types_of(node) {
            let Some((_, req, rec, note)) = TYPES
                .iter()
                .find(|(t, ..)| *t == ty || (ty.ends_with("Business") && *t == "LocalBusiness"))
            else {
                continue;
            };
            let missing = |fields: &[&str]| -> Vec<String> {
                fields
                    .iter()
                    .filter(|f| !f.split('|').any(|alt| get(node, alt).is_some()))
                    .map(|f| (*f).to_owned())
                    .collect()
            };
            let mreq = missing(req);
            for f in &mreq {
                issues.push(Issue::MissingRequired {
                    ty: ty.clone(),
                    field: f.clone(),
                });
            }
            for f in missing(rec) {
                issues.push(Issue::MissingRecommended {
                    ty: ty.clone(),
                    field: f,
                });
            }
            if let Some(n) = note {
                issues.push(Issue::Retired {
                    ty: ty.clone(),
                    note: (*n).to_owned(),
                });
            }
            if mreq.is_empty() {
                if note.is_none() && !elig.eligible.contains(&ty) {
                    elig.eligible.push(ty);
                }
            } else {
                elig.blocked.push((ty, mreq));
            }
        }
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_without_offers_is_blocked() {
        let mut e = Eligibility::default();
        let i = check(
            r#"{"@context":"https://schema.org","@type":"Product","name":"X"}"#,
            &mut e,
        );
        assert!(i.contains(&Issue::MissingRequired {
            ty: "Product".into(),
            field: "offers|review|aggregateRating".into()
        }));
        assert_eq!(e.blocked[0].0, "Product");
        let mut e = Eligibility::default();
        check(
            r#"{"@context":"https://schema.org","@graph":[{"@type":"Product","name":"X","offers":{"price":"1"}},{"@type":"BreadcrumbList","itemListElement":[{"position":1}]}]}"#,
            &mut e,
        );
        assert_eq!(e.eligible, ["Product", "BreadcrumbList"]);
    }

    #[test]
    fn invalid_and_untyped() {
        let mut e = Eligibility::default();
        assert!(matches!(check("{oops", &mut e)[0], Issue::Invalid(_)));
        let i = check(r#"{"name":"x"}"#, &mut e);
        assert!(i.contains(&Issue::NoContext) && i.contains(&Issue::NoType));
        let i = check(
            r#"{"@context":"https://schema.org","@type":"HowTo","name":"a","step":[1]}"#,
            &mut e,
        );
        assert!(i.iter().any(|x| matches!(x, Issue::Retired { .. })));
    }
}
