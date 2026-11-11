//! Notifications and asynchronous approval (§8.4, §11.3, ADR 0039): Telegram (inline Approve /
//! Reject / Details buttons, answers by polling `getUpdates`, no public URL), Slack (incoming
//! webhook; approve links back to the daemon when `daemon.public_url` is set) and e-mail (SMTP).
//! Secrets come from the vault and never appear in logs or errors.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::sync::oneshot;

use super::smtp;

pub const TELEGRAM_TOKEN: &str = "KOR_TELEGRAM_TOKEN";
pub const SLACK_WEBHOOK: &str = "KOR_SLACK_WEBHOOK";
pub const SMTP_PASSWORD: &str = "KOR_SMTP_PASSWORD";
/// Vault entries only the daemon reads; agent commands never receive them.
pub const NOTIFY_SECRETS: &[&str] = &[TELEGRAM_TOKEN, SLACK_WEBHOOK, SMTP_PASSWORD];

/// A message; with `ask` it is an approval request.
#[derive(Debug, Clone, Default)]
pub struct Note {
    pub title: String,
    pub text: String,
    pub ask: Option<Ask>,
}

#[derive(Debug, Clone, Default)]
pub struct Ask {
    pub id: String,
    /// Approve page on the daemon (`<public_url>/approve/<id>?t=<token>`), when reachable.
    pub link: Option<String>,
    /// Button labels: approve, reject, details.
    pub labels: [String; 3],
}

/// A button press.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Approve(String),
    Reject(String),
    Details(String),
}

#[async_trait]
pub trait Notifier: Send + Sync {
    fn name(&self) -> &'static str;
    async fn send(&self, note: &Note) -> Result<(), String>;
    /// Button presses since the last call (long poll); channels without buttons return none.
    async fn poll(&self) -> Vec<Answer> {
        Vec::new()
    }
    /// Whether [`Notifier::poll`] is worth calling (it long-polls).
    fn polls(&self) -> bool {
        false
    }
}

fn http() -> Result<reqwest::Client, String> {
    kor_core::net::client_builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.without_url().to_string())
}

/// Never the URL: it carries the bot token or the webhook secret.
#[allow(clippy::needless_pass_by_value)] // used as `map_err(net_err)`
fn net_err(e: reqwest::Error) -> String {
    if let Some(b) = kor_core::net::blocked_in(&e) {
        return b.to_string();
    }
    e.without_url().to_string()
}

// ---------------------------------------------------------------------------------------------
// Telegram
// ---------------------------------------------------------------------------------------------

#[derive(Debug)]
pub struct Telegram {
    pub api: String,
    token: String,
    pub chat: String,
    /// Long-poll seconds for `getUpdates` (tests use 0).
    pub wait_secs: u64,
    offset: Mutex<i64>,
}

impl Telegram {
    pub fn new(token: String, chat: String) -> Self {
        Self {
            api: "https://api.telegram.org".into(),
            token,
            chat,
            wait_secs: 25,
            offset: Mutex::new(0),
        }
    }

    async fn call(&self, method: &str, body: &Value, timeout: Duration) -> Result<Value, String> {
        let url = format!(
            "{}/bot{}/{method}",
            self.api.trim_end_matches('/'),
            self.token
        );
        let resp = http()?
            .post(url)
            .timeout(timeout)
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(net_err)?;
        let status = resp.status();
        let bytes = resp.bytes().await.map_err(net_err)?;
        let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if !status.is_success() || v["ok"] != true {
            return Err(format!(
                "Telegram {method}: {}",
                v["description"].as_str().unwrap_or(status.as_str())
            ));
        }
        Ok(v["result"].clone())
    }
}

#[async_trait]
impl Notifier for Telegram {
    fn name(&self) -> &'static str {
        "telegram"
    }

    fn polls(&self) -> bool {
        true
    }

    async fn send(&self, note: &Note) -> Result<(), String> {
        let text: String = format!("{}\n\n{}", note.title, note.text)
            .chars()
            .take(4000)
            .collect();
        let mut body =
            json!({ "chat_id": self.chat, "text": text, "disable_web_page_preview": true });
        if let Some(a) = &note.ask {
            body["reply_markup"] = json!({ "inline_keyboard": [[
                { "text": a.labels[0], "callback_data": format!("a:{}", a.id) },
                { "text": a.labels[1], "callback_data": format!("r:{}", a.id) },
                { "text": a.labels[2], "callback_data": format!("d:{}", a.id) },
            ]]});
        }
        self.call("sendMessage", &body, Duration::from_secs(30))
            .await
            .map(|_| ())
    }

    async fn poll(&self) -> Vec<Answer> {
        let offset = *self
            .offset
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let body = json!({ "offset": offset, "timeout": self.wait_secs, "allowed_updates": ["callback_query"] });
        let Ok(result) = self
            .call(
                "getUpdates",
                &body,
                Duration::from_secs(self.wait_secs + 15),
            )
            .await
        else {
            tokio::time::sleep(Duration::from_secs(5)).await;
            return Vec::new();
        };
        let mut out = Vec::new();
        for u in result.as_array().into_iter().flatten() {
            if let Some(id) = u["update_id"].as_i64() {
                let mut o = self
                    .offset
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                *o = (*o).max(id + 1);
            }
            let q = &u["callback_query"];
            // Only the configured chat may answer.
            let chat = q.pointer("/message/chat/id").map(|c| match c {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            });
            if chat.as_deref() != Some(self.chat.as_str()) {
                continue;
            }
            if let Some(qid) = q["id"].as_str() {
                let _ = self
                    .call(
                        "answerCallbackQuery",
                        &json!({ "callback_query_id": qid }),
                        Duration::from_secs(10),
                    )
                    .await;
            }
            let data = q["data"].as_str().unwrap_or_default();
            let answer = match data.split_once(':') {
                Some(("a", id)) => Answer::Approve(id.to_owned()),
                Some(("r", id)) => Answer::Reject(id.to_owned()),
                Some(("d", id)) => Answer::Details(id.to_owned()),
                _ => continue,
            };
            out.push(answer);
        }
        out
    }
}

// ---------------------------------------------------------------------------------------------
// Slack
// ---------------------------------------------------------------------------------------------

#[derive(Debug)]
pub struct Slack {
    webhook: String,
}

impl Slack {
    pub fn new(webhook: String) -> Self {
        Self { webhook }
    }
}

/// Slack mrkdwn escaping.
fn slack_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[async_trait]
impl Notifier for Slack {
    fn name(&self) -> &'static str {
        "slack"
    }

    async fn send(&self, note: &Note) -> Result<(), String> {
        let mut text = format!(
            "*{}*\n{}",
            slack_escape(&note.title),
            slack_escape(&note.text)
        );
        if let Some(a) = &note.ask {
            match &a.link {
                Some(link) => {
                    let _ = write!(text, "\n<{link}|{}>", slack_escape(&a.labels[0]));
                }
                None => {
                    let _ = write!(text, "\n`kor triggers approve {}`", a.id);
                }
            }
        }
        let resp = http()?
            .post(&self.webhook)
            .header("content-type", "application/json")
            .body(json!({ "text": text }).to_string())
            .send()
            .await
            .map_err(net_err)?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("Slack: HTTP {}", resp.status().as_u16()))
        }
    }
}

// ---------------------------------------------------------------------------------------------
// E-mail
// ---------------------------------------------------------------------------------------------

#[derive(Debug)]
pub struct Email {
    pub server: smtp::Server,
    pub from: String,
    pub to: Vec<String>,
}

#[async_trait]
impl Notifier for Email {
    fn name(&self) -> &'static str {
        "email"
    }

    async fn send(&self, note: &Note) -> Result<(), String> {
        let mut text = note.text.clone();
        if let Some(a) = &note.ask {
            match &a.link {
                Some(link) => {
                    let _ = write!(text, "\n\n{}: {link}", a.labels[0]);
                }
                None => {
                    let _ = write!(text, "\n\nkor triggers approve {}", a.id);
                }
            }
        }
        smtp::send(&self.server, &self.from, &self.to, &note.title, &text).await
    }
}

// ---------------------------------------------------------------------------------------------
// Approvals
// ---------------------------------------------------------------------------------------------

#[derive(Debug)]
struct Pending {
    tx: oneshot::Sender<(bool, &'static str)>,
    token: String,
    job: String,
    summary: String,
    details: String,
}

/// Approvals waiting for an answer; the first answer from any channel wins.
#[derive(Debug, Default)]
pub struct Hub {
    pending: Mutex<HashMap<String, Pending>>,
}

/// Random hex (ids, approval and control tokens). Fails closed without an OS random source.
///
/// # Errors
/// When the OS has no secure random source.
pub fn random_hex(bytes: usize) -> Result<String, String> {
    let mut b = vec![0u8; bytes];
    getrandom::fill(&mut b).map_err(|e| format!("no secure random source: {e}"))?;
    Ok(b.iter().fold(String::new(), |mut s, x| {
        let _ = write!(s, "{x:02x}");
        s
    }))
}

/// An approval request from the outside: id and summary (no secret token).
#[derive(Debug, Clone, serde::Serialize)]
pub struct Waiting {
    pub id: String,
    pub job: String,
    pub summary: String,
}

impl Hub {
    /// Register an approval; returns its id, the link token and the receiver of the answer.
    ///
    /// # Errors
    /// Without a secure random source.
    pub fn open(
        &self,
        job: &str,
        summary: &str,
        details: &str,
    ) -> Result<(String, String, oneshot::Receiver<(bool, &'static str)>), String> {
        let id = random_hex(8)?;
        let token = random_hex(24)?;
        let (tx, rx) = oneshot::channel();
        self.lock().insert(
            id.clone(),
            Pending {
                tx,
                token: token.clone(),
                job: job.to_owned(),
                summary: summary.to_owned(),
                details: details.to_owned(),
            },
        );
        Ok((id, token, rx))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Pending>> {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Answer `id` from channel `by` (`telegram`, `api`, `link`); `false` when unknown or
    /// already answered.
    pub fn answer(&self, id: &str, approve: bool, by: &'static str) -> bool {
        self.lock()
            .remove(id)
            .is_some_and(|p| p.tx.send((approve, by)).is_ok())
    }

    /// Answer from an approve link: only with the link's token.
    pub fn answer_with_token(&self, id: &str, token: &str, approve: bool) -> bool {
        use subtle::ConstantTimeEq as _;
        let ok = self
            .lock()
            .get(id)
            .is_some_and(|p| bool::from(p.token.as_bytes().ct_eq(token.as_bytes())));
        ok && self.answer(id, approve, "link")
    }

    pub fn forget(&self, id: &str) {
        self.lock().remove(id);
    }

    pub fn details(&self, id: &str) -> Option<String> {
        self.lock().get(id).map(|p| p.details.clone())
    }

    pub fn summary_with_token(&self, id: &str, token: &str) -> Option<String> {
        use subtle::ConstantTimeEq as _;
        self.lock()
            .get(id)
            .filter(|p| bool::from(p.token.as_bytes().ct_eq(token.as_bytes())))
            .map(|p| p.summary.clone())
    }

    pub fn waiting(&self) -> Vec<Waiting> {
        self.lock()
            .iter()
            .map(|(id, p)| Waiting {
                id: id.clone(),
                job: p.job.clone(),
                summary: p.summary.clone(),
            })
            .collect()
    }
}

/// Send to every channel; failures are returned (logged by the caller), never fatal.
pub async fn broadcast(notifiers: &[Box<dyn Notifier>], note: &Note) -> Vec<String> {
    let mut errors = Vec::new();
    for n in notifiers {
        if let Err(e) = n.send(note).await {
            errors.push(format!("{}: {e}", n.name()));
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn hub_first_answer_wins_and_tokens_are_checked() {
        let hub = Hub::default();
        let (id, token, mut rx) = hub.open("job", "Bash: rm -rf x", "details").unwrap();
        assert_eq!(hub.waiting().len(), 1);
        assert!(!hub.answer_with_token(&id, "wrong", true));
        assert!(hub.summary_with_token(&id, &token).is_some());
        assert!(hub.answer_with_token(&id, &token, false));
        assert_eq!(rx.try_recv(), Ok((false, "link")));
        assert!(!hub.answer(&id, true, "test"), "only once");
    }

    #[tokio::test]
    async fn telegram_buttons_and_chat_check() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/botT0K/sendMessage"))
            .and(body_partial_json(json!({"chat_id": "42", "reply_markup": {"inline_keyboard": [[{"callback_data": "a:x1"}, {"callback_data": "r:x1"}, {"callback_data": "d:x1"}]]}})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true, "result": {}})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/botT0K/getUpdates"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true, "result": [
                {"update_id": 7, "callback_query": {"id": "q1", "data": "a:x1", "message": {"chat": {"id": 42}}}},
                {"update_id": 8, "callback_query": {"id": "q2", "data": "a:x2", "message": {"chat": {"id": 99}}}},
                {"update_id": 9, "callback_query": {"id": "q3", "data": "d:x1", "message": {"chat": {"id": "42"}}}}
            ]})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/botT0K/answerCallbackQuery"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"ok": true, "result": true})),
            )
            .expect(2)
            .mount(&server)
            .await;
        let mut t = Telegram::new("T0K".into(), "42".into());
        t.api = server.uri();
        t.wait_secs = 0;
        let note = Note {
            title: "Kor".into(),
            text: "approve?".into(),
            ask: Some(Ask {
                id: "x1".into(),
                link: None,
                labels: ["Onayla".into(), "Reddet".into(), "Detay".into()],
            }),
        };
        t.send(&note).await.unwrap();
        let answers = t.poll().await;
        assert_eq!(
            answers,
            [Answer::Approve("x1".into()), Answer::Details("x1".into())]
        );
        assert_eq!(*t.offset.lock().unwrap(), 10);
    }

    #[tokio::test]
    async fn slack_text_with_link_and_errors_hide_the_url() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/services/T/B/secret"))
            .and(body_partial_json(
                json!({"text": "*Kor &lt;x&gt;*\nok\n<http://h/approve/1?t=z|Onayla>"}),
            ))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let s = Slack::new(format!("{}/services/T/B/secret", server.uri()));
        let note = Note {
            title: "Kor <x>".into(),
            text: "ok".into(),
            ask: Some(Ask {
                id: "1".into(),
                link: Some("http://h/approve/1?t=z".into()),
                labels: ["Onayla".into(), "Reddet".into(), "Detay".into()],
            }),
        };
        s.send(&note).await.unwrap();
        let bad = Slack::new("http://127.0.0.1:9/services/T/B/secret".into());
        let e = bad.send(&note).await.unwrap_err();
        assert!(!e.contains("secret"), "{e}");
    }
}
