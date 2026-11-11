//! Wayland / Flatpak portal permissions on one consent screen (§27.10, ADR 0052): screen capture
//! (`ScreenCast`), input (`RemoteDesktop` keyboard + pointer), file access (`FileChooser`) and
//! notifications. The desktop's settings card explains all four once, then this asks the portals
//! in order over D-Bus; each portal shows its own system dialog. Screen and input grants are
//! requested with `persist_mode = 2` and their restore tokens returned, so later computer-use and
//! screenshot sessions do not ask again.

use std::io;

/// Texts of the requests (localized by the caller).
#[derive(Debug, Clone)]
pub struct Texts {
    /// File chooser title.
    pub folder_title: String,
    /// Test notification.
    pub notify_title: String,
    pub notify_body: String,
}

/// Result of one portal request.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Outcome {
    /// `screencast`, `remote-desktop`, `file-chooser`, `notification`.
    pub portal: &'static str,
    pub granted: bool,
    /// Restore token (screen, input) or the chosen folder (files).
    pub detail: Option<String>,
}

/// Portal response code: 0 granted, 1 cancelled by the user, 2 other.
pub fn granted(code: u32) -> bool {
    code == 0
}

/// Object path of the `Request` a call with `token` creates, for connection `unique` (`:1.42`).
pub fn request_path(unique: &str, token: &str) -> String {
    let sender = unique.trim_start_matches(':').replace('.', "_");
    format!("/org/freedesktop/portal/desktop/request/{sender}/{token}")
}

/// Ask the four portals in order. Linux only.
///
/// # Errors
/// Outside Linux, or without a D-Bus session bus.
pub async fn request_all(t: &Texts) -> io::Result<Vec<Outcome>> {
    if crate::os() != crate::Os::Linux {
        return Err(io::Error::new(io::ErrorKind::Unsupported, "Linux only"));
    }
    imp::request_all(t).await
}

#[cfg(target_os = "linux")]
mod imp {
    use std::collections::HashMap;
    use std::future::Future;
    use std::io;
    use std::sync::atomic::{AtomicU32, Ordering};

    use futures_util::StreamExt as _;
    use zbus::Connection;
    use zbus::zvariant::{ObjectPath, OwnedValue, Value};

    use super::{Outcome, Texts, granted, request_path, text};

    const DEST: &str = "org.freedesktop.portal.Desktop";
    const PATH: &str = "/org/freedesktop/portal/desktop";

    fn err(e: impl std::fmt::Display) -> io::Error {
        io::Error::other(e.to_string())
    }

    static N: AtomicU32 = AtomicU32::new(0);

    fn token() -> String {
        format!(
            "kor_{}_{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        )
    }

    type Results = HashMap<String, OwnedValue>;

    /// Subscribe to the `Response` of the request `call(handle_token)` creates, make the call,
    /// and wait for the user's answer.
    async fn request<F, Fut>(conn: &Connection, call: F) -> io::Result<(u32, Results)>
    where
        F: FnOnce(String) -> Fut,
        Fut: Future<Output = zbus::Result<zbus::Message>>,
    {
        let tok = token();
        let unique = conn
            .unique_name()
            .map(ToString::to_string)
            .unwrap_or_default();
        let proxy = zbus::Proxy::new(
            conn,
            DEST,
            request_path(&unique, &tok),
            "org.freedesktop.portal.Request",
        )
        .await
        .map_err(err)?;
        let mut responses = proxy.receive_signal("Response").await.map_err(err)?;
        call(tok).await.map_err(err)?;
        let msg = responses.next().await.ok_or_else(|| err("portal closed"))?;
        msg.body().deserialize::<(u32, Results)>().map_err(err)
    }

    fn opts(token: String) -> HashMap<&'static str, Value<'static>> {
        HashMap::from([("handle_token", Value::from(token))])
    }

    /// `CreateSession` → `Select…` → `Start` on a session portal (`ScreenCast`, `RemoteDesktop`).
    async fn session(
        conn: &Connection,
        iface: &'static str,
        select: &'static str,
        types: u32,
    ) -> io::Result<Outcome> {
        let name = if iface.ends_with("ScreenCast") {
            "screencast"
        } else {
            "remote-desktop"
        };
        let (code, res) = request(conn, |t| async move {
            let mut o = opts(t);
            o.insert("session_handle_token", Value::from(token()));
            conn.call_method(Some(DEST), PATH, Some(iface), "CreateSession", &(o,))
                .await
        })
        .await?;
        let handle = res.get("session_handle").and_then(text);
        let (true, Some(handle)) = (granted(code), handle) else {
            return Ok(Outcome {
                portal: name,
                granted: false,
                detail: None,
            });
        };
        let path = ObjectPath::try_from(handle.as_str()).map_err(err)?;
        let (code, _) = request(conn, |t| {
            let path = path.clone();
            async move {
                let mut o = opts(t);
                o.insert("types", Value::from(types));
                o.insert("persist_mode", Value::from(2u32));
                if select == "SelectSources" {
                    o.insert("multiple", Value::from(false));
                }
                conn.call_method(Some(DEST), PATH, Some(iface), select, &(path, o))
                    .await
            }
        })
        .await?;
        let mut out = Outcome {
            portal: name,
            granted: false,
            detail: None,
        };
        if granted(code) {
            let (code, res) = request(conn, |t| {
                let path = path.clone();
                async move {
                    conn.call_method(Some(DEST), PATH, Some(iface), "Start", &(path, "", opts(t)))
                        .await
                }
            })
            .await?;
            out.granted = granted(code);
            out.detail = res.get("restore_token").and_then(text);
        }
        let _ = conn
            .call_method(
                Some(DEST),
                &path,
                Some("org.freedesktop.portal.Session"),
                "Close",
                &(),
            )
            .await;
        Ok(out)
    }

    pub async fn request_all(t: &Texts) -> io::Result<Vec<Outcome>> {
        let conn = Connection::session().await.map_err(err)?;
        let mut out = vec![
            // Monitor (1) | window (2).
            session(
                &conn,
                "org.freedesktop.portal.ScreenCast",
                "SelectSources",
                3,
            )
            .await?,
            // Keyboard (1) | pointer (2).
            session(
                &conn,
                "org.freedesktop.portal.RemoteDesktop",
                "SelectDevices",
                3,
            )
            .await?,
        ];
        let title = t.folder_title.clone();
        let (code, res) = request(&conn, |tok| {
            let conn = &conn;
            async move {
                let mut o = opts(tok);
                o.insert("directory", Value::from(true));
                conn.call_method(
                    Some(DEST),
                    PATH,
                    Some("org.freedesktop.portal.FileChooser"),
                    "OpenFile",
                    &("", title.as_str(), o),
                )
                .await
            }
        })
        .await?;
        let folder = res
            .get("uris")
            .and_then(|v| <Vec<String>>::try_from(v.try_clone().ok()?).ok())
            .and_then(|u| u.into_iter().next());
        out.push(Outcome {
            portal: "file-chooser",
            granted: granted(code) && folder.is_some(),
            detail: folder,
        });
        let note = HashMap::from([
            ("title", Value::from(t.notify_title.as_str())),
            ("body", Value::from(t.notify_body.as_str())),
        ]);
        let sent = conn
            .call_method(
                Some(DEST),
                PATH,
                Some("org.freedesktop.portal.Notification"),
                "AddNotification",
                &("kor-portals", note),
            )
            .await;
        out.push(Outcome {
            portal: "notification",
            granted: sent.is_ok(),
            detail: sent.err().map(|e| e.to_string()),
        });
        Ok(out)
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use super::{Outcome, Texts};

    #[allow(clippy::unused_async)] // same signature as the Linux one
    pub async fn request_all(_: &Texts) -> std::io::Result<Vec<Outcome>> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Linux only",
        ))
    }
}

/// A string inside a D-Bus variant.
#[cfg(target_os = "linux")]
fn text(v: &zbus::zvariant::OwnedValue) -> Option<String> {
    v.try_clone().ok().and_then(|v| String::try_from(v).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_paths_follow_the_portal_spec() {
        assert_eq!(
            request_path(":1.42", "kor_7_0"),
            "/org/freedesktop/portal/desktop/request/1_42/kor_7_0"
        );
        assert!(granted(0) && !granted(1) && !granted(2));
        let o = Outcome {
            portal: "screencast",
            granted: true,
            detail: Some("tok".into()),
        };
        assert_eq!(serde_json::to_value(&o).unwrap()["portal"], "screencast");
    }
}
