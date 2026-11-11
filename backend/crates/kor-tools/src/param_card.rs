//! A compact parameter card a tool asks before it runs (§20.2, ADR 0060): every open choice at
//! once with its options and default, plus notes, the cost estimate and whether the cost needs a
//! confirmation. The agent loop puts it to the surface's approver; the answers are merged into
//! the call's input.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// One open choice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamField {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Allowed values (empty: a free value within `min`/`max`).
    #[serde(default)]
    pub options: Vec<Value>,
    pub default: Option<Value>,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

impl ParamField {
    /// At most four labels for a one-line choice (default first), for surfaces that can only
    /// ask multiple-choice questions.
    pub fn choices(&self) -> Vec<Value> {
        let mut out: Vec<Value> = self.default.iter().cloned().collect();
        let range = [self.min, self.max].into_iter().flatten().map(|n| {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                #[allow(clippy::cast_possible_truncation)] // checked above
                let i = n as i64;
                Value::from(i)
            } else {
                Value::from(n)
            }
        });
        for v in self.options.iter().cloned().chain(range) {
            if out.len() == 4 {
                break;
            }
            if !out.contains(&v) {
                out.push(v);
            }
        }
        out
    }
}

/// Everything asked before one call.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamCard {
    /// `model: prompt`.
    pub title: String,
    pub fields: Vec<ParamField>,
    /// Values the model cannot do (with the nearest option) and models that can.
    pub notes: Vec<String>,
    /// Cost before running (`$0.04`, `≈$0.40`).
    pub cost: Option<String>,
    /// Typical duration from earlier runs (`≈ 45 s`).
    pub eta: Option<String>,
    /// The cost is above the configured threshold: running needs a yes.
    pub confirm: bool,
    /// Answers go into `input[target]`; `None`: the top level.
    pub target: Option<String>,
    /// Values settled without asking (saved defaults, values named in the prompt), merged with
    /// the answers.
    #[serde(default)]
    pub fixed: Map<String, Value>,
}

/// The user's answer to a card.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParamAnswer {
    pub values: Map<String, Value>,
    /// `false`: do not run (the cost was declined).
    pub go: bool,
}

impl ParamCard {
    /// Is there anything to show?
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.notes.is_empty() && !self.confirm
    }

    /// Every field at its default.
    pub fn defaults(&self) -> Map<String, Value> {
        self.fields
            .iter()
            .filter_map(|f| Some((f.name.clone(), f.default.clone()?)))
            .collect()
    }

    /// `input` with the answers merged (at `target` or the top level) and `ask: false`, so the
    /// tool does not ask again.
    pub fn apply(&self, mut input: Value, answers: &Map<String, Value>) -> Value {
        if !input.is_object() {
            return input;
        }
        let slot = match &self.target {
            Some(t) => {
                if !input[t.as_str()].is_object() {
                    input[t.as_str()] = Value::Object(Map::new());
                }
                &mut input[t.as_str()]
            }
            None => &mut input,
        };
        if let Some(o) = slot.as_object_mut() {
            for (k, v) in self.fixed.iter().chain(answers) {
                o.insert(k.clone(), v.clone());
            }
        }
        input["ask"] = Value::Bool(false);
        input
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn choices_put_the_default_first_and_cap_at_four() {
        let f = ParamField {
            name: "d".into(),
            description: String::new(),
            options: vec![
                json!("5s"),
                json!("8s"),
                json!("10s"),
                json!("12s"),
                json!("15s"),
            ],
            default: Some(json!("8s")),
            min: None,
            max: None,
        };
        assert_eq!(
            f.choices(),
            vec![json!("8s"), json!("5s"), json!("10s"), json!("12s")]
        );
        let r = ParamField {
            options: vec![],
            default: Some(json!(1)),
            min: Some(1.0),
            max: Some(4.0),
            ..f
        };
        assert_eq!(r.choices(), vec![json!(1), json!(4)]);
    }

    #[test]
    fn answers_merge_at_the_target() {
        let mut card = ParamCard {
            target: Some("input".into()),
            ..ParamCard::default()
        };
        card.fixed.insert("seed".into(), json!(7));
        let mut a = Map::new();
        a.insert("duration".into(), json!("5s"));
        let v = card.apply(json!({"prompt": "cat"}), &a);
        assert_eq!(
            v,
            json!({"prompt": "cat", "input": {"seed": 7, "duration": "5s"}, "ask": false})
        );
        let top = ParamCard::default().apply(json!({"prompt": "cat"}), &a);
        assert_eq!(top["duration"], "5s");
    }
}
