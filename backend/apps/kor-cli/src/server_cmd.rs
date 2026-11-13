//! `kor server …` (spec §11.1): the self-hosted team server and a member's connection to it.
//! Server side: `init`, `add-member`, `start`, `status`. Member side: `join` (import the bundle
//! `add-member` wrote) and `login` (single sign-on with the device flow). The logic is
//! `kor_server` and `kor_agent::team_client`; this file parses, prints and asks.

use std::path::{Path, PathBuf};
use std::time::Duration;

use kor_i18n::{Localizer, args};
use serde_json::{Value, json};

use crate::Ctx;

#[derive(Debug, clap::Subcommand)]
pub enum ServerCmd {
    /// Create a team server here: settings, certificate authority, server certificate and the first admin.
    Init {
        /// Host name members use (in the server certificate).
        #[arg(long)]
        domain: String,
        /// E-mail of the first admin.
        #[arg(long)]
        admin: String,
        /// Team name (the certificate authority's organisation).
        #[arg(long, default_value = "team")]
        team: String,
        /// `postgres://…?sslmode=require` instead of the embedded `SQLite` database.
        #[arg(long)]
        db: Option<String>,
        /// Server directory (default `<kor home>/server`).
        #[arg(long, env = "KOR_SERVER_DIR")]
        dir: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Add a member or change their role; writes their bundle (certificate, key, token).
    AddMember {
        email: String,
        /// admin, member (also `dev`) or viewer.
        #[arg(long, default_value = "member")]
        role: String,
        /// Remove the member instead: their certificates and tokens stop working at once.
        #[arg(long)]
        remove: bool,
        #[arg(long, env = "KOR_SERVER_DIR")]
        dir: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// The team server here and this machine's team connection.
    Status {
        #[arg(long, env = "KOR_SERVER_DIR")]
        dir: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Run the team server (HTTPS with client certificates).
    Start {
        /// `host:port` (default from server.toml, `0.0.0.0:8443`).
        #[arg(long)]
        listen: Option<String>,
        /// Start in the background and return.
        #[arg(long)]
        background: bool,
        #[arg(long, env = "KOR_SERVER_DIR")]
        dir: Option<PathBuf>,
    },
    /// Join a team with the bundle an admin gave you (stored in the OS keychain).
    Join {
        bundle: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Sign in to the team server through its identity provider (device code).
    Login {
        /// Identity provider name (default: the first one the server offers).
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    let dir = |a: clap::Arg| a.help(l.t("arg-server-dir"));
    let js = |a: clap::Arg| a.help(l.t("arg-server-json"));
    c.about(l.t("cmd-server"))
        .mut_subcommand("init", |s| {
            s.about(l.t("cmd-server-init"))
                .mut_arg("domain", |a| a.help(l.t("arg-server-domain")))
                .mut_arg("admin", |a| a.help(l.t("arg-server-admin")))
                .mut_arg("team", |a| a.help(l.t("arg-server-team")))
                .mut_arg("db", |a| a.help(l.t("arg-server-db")))
                .mut_arg("dir", dir)
                .mut_arg("json", js)
        })
        .mut_subcommand("add-member", |s| {
            s.about(l.t("cmd-server-add-member"))
                .mut_arg("email", |a| a.help(l.t("arg-server-email")))
                .mut_arg("role", |a| a.help(l.t("arg-server-role")))
                .mut_arg("remove", |a| a.help(l.t("arg-server-remove")))
                .mut_arg("dir", dir)
                .mut_arg("json", js)
        })
        .mut_subcommand("status", |s| {
            s.about(l.t("cmd-server-status"))
                .mut_arg("dir", dir)
                .mut_arg("json", js)
        })
        .mut_subcommand("start", |s| {
            s.about(l.t("cmd-server-start"))
                .mut_arg("listen", |a| a.help(l.t("arg-server-listen")))
                .mut_arg("background", |a| a.help(l.t("arg-server-background")))
                .mut_arg("dir", dir)
        })
        .mut_subcommand("join", |s| {
            s.about(l.t("cmd-server-join"))
                .mut_arg("bundle", |a| a.help(l.t("arg-server-bundle")))
                .mut_arg("json", js)
        })
        .mut_subcommand("login", |s| {
            s.about(l.t("cmd-server-login"))
                .mut_arg("provider", |a| a.help(l.t("arg-server-provider")))
                .mut_arg("json", js)
        })
}

/// A running server would hold the interactive screen (sign-in waits like any long command).
pub fn blocks_tui(cmd: &ServerCmd) -> bool {
    matches!(
        cmd,
        ServerCmd::Start {
            background: false,
            ..
        }
    )
}

fn fail(l: &Localizer, e: impl std::fmt::Display) -> bool {
    eprintln!(
        "{}",
        l.t_args("error-generic", &args!("error" => e.to_string()))
    );
    false
}

fn server_dir(dir: Option<PathBuf>) -> Result<PathBuf, String> {
    dir.or_else(|| kor_runtime::kor_home().map(|h| h.join("server")))
        .ok_or_else(|| "no data directory; pass --dir".to_owned())
}

pub fn run(ctx: &Ctx, cmd: ServerCmd) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let rt = crate::providers::runtime()?;
    Ok(match cmd {
        ServerCmd::Init {
            domain,
            admin,
            team,
            db,
            dir,
            json,
        } => match server_dir(dir) {
            Ok(d) => rt.block_on(init(l, &d, &team, &domain, &admin, db.as_deref(), json)),
            Err(e) => fail(l, e),
        },
        ServerCmd::AddMember {
            email,
            role,
            remove,
            dir,
            json,
        } => match server_dir(dir) {
            Ok(d) => rt.block_on(add_member(l, &d, &email, &role, remove, json)),
            Err(e) => fail(l, e),
        },
        ServerCmd::Status { dir, json } => {
            let d = server_dir(dir).ok();
            rt.block_on(status(ctx, d.as_deref(), json))
        }
        ServerCmd::Start {
            listen,
            background: true,
            dir,
        } => {
            let mut a = vec!["server".to_owned(), "start".to_owned()];
            if let Some(x) = listen {
                a.extend(["--listen".to_owned(), x]);
            }
            if let Some(d) = dir {
                a.extend(["--dir".to_owned(), d.display().to_string()]);
            }
            detached(l, &a)
        }
        ServerCmd::Start {
            listen,
            background: false,
            dir,
        } => match server_dir(dir) {
            Ok(d) => rt.block_on(start(l, &d, listen)),
            Err(e) => fail(l, e),
        },
        ServerCmd::Join { bundle, json } => join(ctx, &bundle, json),
        ServerCmd::Login { provider, json } => rt.block_on(login(ctx, provider.as_deref(), json)),
    })
}

async fn init(
    l: &Localizer,
    dir: &Path,
    team: &str,
    domain: &str,
    admin: &str,
    db: Option<&str>,
    json: bool,
) -> bool {
    match kor_server::init(dir, team, domain, admin, db).await {
        Ok((cfg, e)) => {
            if json {
                println!(
                    "{}",
                    json!({ "dir": dir, "domain": cfg.domain, "listen": cfg.listen, "admin": e.email, "bundle": e.bundle })
                );
            } else {
                println!("{}", l.t_args("server-initialized", &args!("dir" => dir.display().to_string(), "domain" => cfg.domain, "listen" => cfg.listen)));
                println!(
                    "{}",
                    l.t_args(
                        "server-bundle-written",
                        &args!("email" => e.email, "path" => e.bundle.display().to_string())
                    )
                );
            }
            true
        }
        Err(e) => fail(l, e),
    }
}

async fn add_member(
    l: &Localizer,
    dir: &Path,
    email: &str,
    role: &str,
    remove: bool,
    json: bool,
) -> bool {
    let layout = kor_server::Layout::new(dir);
    let cfg = match kor_server::ServerConfig::load(&layout.config()) {
        Ok(c) => c,
        Err(e) => return fail(l, e),
    };
    let store = match kor_server::Store::open(&cfg, dir).await {
        Ok(s) => s,
        Err(e) => return fail(l, e),
    };
    // Changes made here on the server machine are audited as the server's operator.
    let by = "server-admin";
    if remove {
        return match store.remove_member(email, by).await {
            Ok(()) => {
                if json {
                    println!("{}", json!({ "removed": email }));
                } else {
                    println!(
                        "{}",
                        l.t_args("server-member-removed", &args!("email" => email.to_owned()))
                    );
                }
                true
            }
            Err(e) => fail(l, e),
        };
    }
    let Some(role) = kor_server::Role::parse(role) else {
        return fail(
            l,
            l.t_args("server-bad-role", &args!("role" => role.to_owned())),
        );
    };
    match store.add_member(&cfg, &layout, email, role, by).await {
        Ok(e) => {
            if json {
                println!(
                    "{}",
                    json!({ "email": e.email, "role": e.role, "bundle": e.bundle, "expires": e.expires_ms })
                );
            } else {
                println!(
                    "{}",
                    l.t_args(
                        "server-member-added",
                        &args!("email" => e.email.clone(), "role" => role.as_str())
                    )
                );
                println!(
                    "{}",
                    l.t_args(
                        "server-bundle-written",
                        &args!("email" => e.email, "path" => e.bundle.display().to_string())
                    )
                );
            }
            true
        }
        Err(e) => fail(l, e),
    }
}

async fn start(l: &Localizer, dir: &Path, listen: Option<String>) -> bool {
    let app = match kor_server::http::App::open(dir).await {
        Ok(a) => a,
        Err(e) => return fail(l, e),
    };
    let addr = listen.unwrap_or_else(|| app.cfg.listen.clone());
    let Ok(addr) = addr.parse::<std::net::SocketAddr>() else {
        return fail(l, format!("{addr}: not host:port"));
    };
    match kor_server::http::start(app.clone(), addr).await {
        Ok(r) => {
            eprintln!(
                "{}",
                l.t_args(
                    "server-listening",
                    &args!("addr" => r.addr.to_string(), "team" => app.cfg.team.clone())
                )
            );
            // Ctrl+C ends the process; open connections close with it.
            tokio::select! {
                _ = r.task => {}
                _ = tokio::signal::ctrl_c() => {}
            }
            true
        }
        Err(e) => fail(l, e),
    }
}

async fn local_status(dir: &Path) -> Result<Value, String> {
    let layout = kor_server::Layout::new(dir);
    let cfg = kor_server::ServerConfig::load(&layout.config())?;
    let store = kor_server::Store::open(&cfg, dir).await?;
    let counts = store.counts().await.map_err(|e| e.message)?;
    let chain = store.audit_verify().await;
    let running = tokio::time::timeout(
        Duration::from_secs(2),
        tokio::net::TcpStream::connect(cfg.listen.replace("0.0.0.0", "127.0.0.1")),
    )
    .await
    .is_ok_and(|r| r.is_ok());
    Ok(json!({
        "dir": dir, "team": cfg.team, "domain": cfg.domain, "listen": cfg.listen, "db": store.db.engine(),
        "running": running, "requireClientCert": cfg.require_client_cert,
        "providers": cfg.providers.iter().map(|p| p.name.clone()).collect::<Vec<_>>(),
        "counts": counts,
        "audit": match chain { Ok(n) => json!({ "ok": true, "entries": n }), Err(seq) => json!({ "ok": false, "brokenAt": seq }) },
    }))
}

async fn member_status(ctx: &Ctx) -> Option<Value> {
    let server = ctx
        .config
        .as_ref()
        .ok()?
        .config
        .team
        .server
        .trim()
        .to_owned();
    if server.is_empty() {
        return None;
    }
    let me = async {
        let c = kor_agent::team_client::builder(Duration::from_secs(5))?
            .build()
            .map_err(|e| e.to_string())?;
        let r = c
            .get(format!("{}/api/v1/me", server.trim_end_matches('/')))
            .send()
            .await
            .map_err(|e| kor_core::net::describe(&e))?;
        let status = r.status();
        let v: Value = serde_json::from_str(&r.text().await.map_err(|e| e.to_string())?)
            .unwrap_or(Value::Null);
        if status.is_success() {
            Ok(v)
        } else {
            Err(v["error"].as_str().unwrap_or(status.as_str()).to_owned())
        }
    }
    .await;
    Some(match me {
        Ok(v) => json!({ "server": server, "email": v["email"], "role": v["role"] }),
        Err(e) => json!({ "server": server, "error": e }),
    })
}

async fn status(ctx: &Ctx, dir: Option<&Path>, json: bool) -> bool {
    let l = &ctx.l;
    let here = match dir.filter(|d| kor_server::Layout::new(d).config().is_file()) {
        Some(d) => Some(local_status(d).await),
        None => None,
    };
    let member = member_status(ctx).await;
    if json {
        let server = match &here {
            Some(Ok(v)) => v.clone(),
            Some(Err(e)) => json!({ "error": e }),
            None => Value::Null,
        };
        println!("{}", json!({ "server": server, "member": member }));
        return !matches!(here, Some(Err(_)));
    }
    match &here {
        Some(Ok(v)) => {
            let c = &v["counts"];
            println!(
                "{}",
                l.t_args(
                    "server-status-line",
                    &args!(
                        "team" => v["team"].as_str().unwrap_or_default().to_owned(),
                        "domain" => v["domain"].as_str().unwrap_or_default().to_owned(),
                        "listen" => v["listen"].as_str().unwrap_or_default().to_owned(),
                        "db" => v["db"].as_str().unwrap_or_default().to_owned(),
                        "running" => if v["running"] == true { "yes" } else { "no" }
                    )
                )
            );
            println!(
                "{}",
                l.t_args(
                    "server-status-counts",
                    &args!(
                        "admins" => c["admins"].as_i64().unwrap_or(0),
                        "members" => c["members"].as_i64().unwrap_or(0),
                        "viewers" => c["viewers"].as_i64().unwrap_or(0),
                        "docs" => c["docs"].as_i64().unwrap_or(0),
                        "queued" => c["queued"].as_i64().unwrap_or(0),
                        "running" => c["running"].as_i64().unwrap_or(0),
                        "vault" => c["vault"].as_i64().unwrap_or(0)
                    )
                )
            );
            if v["audit"]["ok"] == true {
                println!(
                    "{}",
                    l.t_args(
                        "server-audit-ok",
                        &args!("entries" => v["audit"]["entries"].as_i64().unwrap_or(0))
                    )
                );
            } else {
                println!(
                    "{}",
                    l.t_args(
                        "server-audit-broken",
                        &args!("seq" => v["audit"]["brokenAt"].as_i64().unwrap_or(-1))
                    )
                );
            }
        }
        Some(Err(e)) => return fail(l, e),
        None => println!("{}", l.t("server-none-here")),
    }
    match member {
        Some(m) if m["error"].is_string() => println!(
            "{}",
            l.t_args(
                "server-member-error",
                &args!(
                    "server" => m["server"].as_str().unwrap_or_default().to_owned(),
                    "error" => m["error"].as_str().unwrap_or_default().to_owned()
                )
            )
        ),
        Some(m) => println!(
            "{}",
            l.t_args(
                "server-member-line",
                &args!(
                    "server" => m["server"].as_str().unwrap_or_default().to_owned(),
                    "email" => m["email"].as_str().unwrap_or_default().to_owned(),
                    "role" => m["role"].as_str().unwrap_or_default().to_owned()
                )
            )
        ),
        None => println!("{}", l.t("server-not-joined")),
    }
    true
}

fn set_team_server(server: &str) -> Result<(), String> {
    let file = kor_config::user_dir()
        .ok_or("no user config directory")?
        .join("config.toml");
    kor_config::set_scalar(&file, "team.server", server).map_err(|e| e.to_string())
}

fn join(ctx: &Ctx, bundle: &Path, json: bool) -> bool {
    let l = &ctx.l;
    let text = match std::fs::read_to_string(bundle) {
        Ok(t) if t.len() < 64 * 1024 => t,
        Ok(_) => return fail(l, "bundle too large"),
        Err(e) => return fail(l, format!("{}: {e}", bundle.display())),
    };
    let b = match kor_agent::team_client::import(&kor_secrets::KeychainStore, &text) {
        Ok(b) => b,
        Err(e) => return fail(l, e),
    };
    if let Err(e) = set_team_server(&b.server) {
        return fail(l, e);
    }
    if json {
        println!(
            "{}",
            json!({ "server": b.server, "email": b.email, "role": b.role })
        );
    } else {
        println!(
            "{}",
            l.t_args(
                "server-joined",
                &args!("server" => b.server, "email" => b.email, "role" => b.role)
            )
        );
        // The bundle holds a private key: it is in the keychain now.
        println!(
            "{}",
            l.t_args(
                "server-delete-bundle",
                &args!("path" => bundle.display().to_string())
            )
        );
    }
    true
}

async fn login(ctx: &Ctx, provider: Option<&str>, json: bool) -> bool {
    use kor_agent::team_client as tc;
    let l = &ctx.l;
    let server = ctx
        .config
        .as_ref()
        .map(|c| c.config.team.server.trim().to_owned())
        .unwrap_or_default();
    if server.is_empty() {
        return fail(l, l.t("server-not-joined"));
    }
    let client = match tc::builder(Duration::from_secs(20))
        .and_then(|b| b.build().map_err(|e| e.to_string()))
    {
        Ok(c) => c,
        Err(e) => return fail(l, e),
    };
    let list = match tc::providers(&client, &server).await {
        Ok(p) => p,
        Err(e) => return fail(l, e),
    };
    let Some(p) = list.iter().find(|p| provider.is_none_or(|n| n == p.name)) else {
        return fail(l, l.t("server-no-provider"));
    };
    // The provider is another host: a plain client (no team certificate goes to it).
    let idp = match kor_core::net::client_builder()
        .timeout(Duration::from_secs(20))
        .build()
    {
        Ok(c) => c,
        Err(e) => return fail(l, e),
    };
    let d = match tc::device_start(&idp, p).await {
        Ok(d) => d,
        Err(e) => return fail(l, e),
    };
    let url = d
        .verification_uri_complete
        .clone()
        .unwrap_or_else(|| d.verification_uri.clone());
    if json {
        println!(
            "{}",
            json!({ "verificationUri": url, "userCode": d.user_code })
        );
    } else {
        println!(
            "{}",
            l.t_args(
                "server-login-open",
                &args!("url" => url, "code" => d.user_code.clone())
            )
        );
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(d.expires_in.min(1800));
    let mut wait = Duration::from_secs(d.interval.clamp(1, 30));
    loop {
        tokio::time::sleep(wait).await;
        match tc::device_poll(&idp, p, &d).await {
            Ok(Some(tok)) => {
                return match tc::exchange(&client, &kor_secrets::KeychainStore, &server, p, &tok)
                    .await
                {
                    Ok((email, role)) => {
                        if json {
                            println!("{}", json!({ "email": email, "role": role }));
                        } else {
                            println!(
                                "{}",
                                l.t_args(
                                    "server-logged-in",
                                    &args!("email" => email, "role" => role)
                                )
                            );
                        }
                        true
                    }
                    Err(e) => fail(l, e),
                };
            }
            Ok(None) if std::time::Instant::now() < deadline => {
                wait = (wait + Duration::from_secs(1)).min(Duration::from_secs(30));
            }
            Ok(None) => return fail(l, l.t("server-login-expired")),
            Err(e) => return fail(l, e),
        }
    }
}

fn detached(l: &Localizer, args: &[String]) -> bool {
    use kor_platform::Detach as _;
    let (Ok(exe), Some(dir)) = (std::env::current_exe(), kor_runtime::kor_home()) else {
        return fail(l, "no data directory");
    };
    let _ = std::fs::create_dir_all(&dir);
    let Ok(log) = std::fs::File::create(dir.join("server.log")) else {
        return fail(l, "log file");
    };
    let err = log.try_clone().ok();
    let mut cmd = std::process::Command::new(exe);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(log);
    if let Some(e) = err {
        cmd.stderr(e);
    }
    match cmd.detach().spawn() {
        Ok(c) => {
            println!("{}", l.t_args("server-background", &args!("pid" => c.id())));
            true
        }
        Err(e) => fail(l, e),
    }
}
