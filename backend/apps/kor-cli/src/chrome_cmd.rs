//! `kor chrome pair|install|uninstall|status|allow|deny|domains|profile` (§18, §27.12, ADR 0058):
//! the Kor browser extension. Logic in `kor_tools::chrome` (pairing, domain lists),
//! `kor_platform::native_messaging` (host manifests per OS and browser) and
//! `kor_runtime::browser_assist::forms` (form profiles).

use std::collections::BTreeMap;
use std::path::PathBuf;

use kor_i18n::{Localizer, args};
use kor_platform::native_messaging::{self as nm, Browser};
use kor_tools::chrome::{pairing, policy};

use crate::Ctx;

/// Chrome Web Store and Firefox Add-ons listings (published by the Kor maintainers).
pub const CHROME_STORE: &str = "https://chromewebstore.google.com/search/Kor";
pub const FIREFOX_STORE: &str = "https://addons.mozilla.org/firefox/addon/kor/";

#[derive(Debug, clap::Subcommand)]
pub enum ChromeCmd {
    /// One-time pairing code for the extension's side panel.
    Pair {
        /// Forget every paired browser instead.
        #[arg(long)]
        revoke: bool,
    },
    /// Register the native messaging host and show how to load the extension.
    Install {
        /// chrome, edge, brave, chromium, firefox or all (default: the installed ones).
        #[arg(long)]
        browser: Option<String>,
        /// Open the store page instead of the unpacked build.
        #[arg(long)]
        store: bool,
        /// Do not open the browser's extensions page.
        #[arg(long)]
        no_open: bool,
        /// Also allow this extension id (a Web Store build).
        #[arg(long = "extension-id")]
        extension_id: Vec<String>,
    },
    /// Remove the native messaging host registrations.
    Uninstall {
        #[arg(long)]
        browser: Option<String>,
    },
    /// Extension, pairing and domain status.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Let Kor open and act on a domain (and its subdomains).
    Allow { domain: String },
    /// Never let Kor open or act on a domain.
    Deny { domain: String },
    /// The allow and deny lists.
    Domains {
        /// Remove a domain from both lists.
        #[arg(long)]
        remove: Option<String>,
    },
    /// Form filler profiles (kept in the OS keychain).
    Profile {
        #[command(subcommand)]
        command: ProfileCmd,
    },
}

#[derive(Debug, clap::Subcommand)]
pub enum ProfileCmd {
    List,
    Show {
        name: String,
    },
    /// Add or change values: `kor chrome profile set kisisel email=a@b.co phone="+90 …"`.
    Set {
        name: String,
        values: Vec<String>,
        /// Remove these keys.
        #[arg(long)]
        unset: Vec<String>,
    },
    Remove {
        name: String,
    },
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    let browser = |s: clap::Command| s.mut_arg("browser", |a| a.help(l.t("arg-chrome-browser")));
    let pname = |s: clap::Command| s.mut_arg("name", |a| a.help(l.t("arg-chrome-profile-name")));
    c.about(l.t("cmd-chrome"))
        .mut_subcommand("pair", |s| {
            s.about(l.t("cmd-chrome-pair"))
                .mut_arg("revoke", |a| a.help(l.t("arg-chrome-revoke")))
        })
        .mut_subcommand("install", |s| {
            browser(s.about(l.t("cmd-chrome-install")))
                .mut_arg("store", |a| a.help(l.t("arg-chrome-store")))
                .mut_arg("no_open", |a| a.help(l.t("arg-chrome-no-open")))
                .mut_arg("extension_id", |a| a.help(l.t("arg-chrome-extension-id")))
        })
        .mut_subcommand("uninstall", |s| {
            browser(s.about(l.t("cmd-chrome-uninstall")))
        })
        .mut_subcommand("status", |s| {
            s.about(l.t("cmd-chrome-status"))
                .mut_arg("json", |a| a.help(l.t("arg-json")))
        })
        .mut_subcommand("allow", |s| {
            s.about(l.t("cmd-chrome-allow"))
                .mut_arg("domain", |a| a.help(l.t("arg-chrome-domain")))
        })
        .mut_subcommand("deny", |s| {
            s.about(l.t("cmd-chrome-deny"))
                .mut_arg("domain", |a| a.help(l.t("arg-chrome-domain")))
        })
        .mut_subcommand("domains", |s| {
            s.about(l.t("cmd-chrome-domains"))
                .mut_arg("remove", |a| a.help(l.t("arg-chrome-remove")))
        })
        .mut_subcommand("profile", |s| {
            s.about(l.t("cmd-chrome-profile"))
                .mut_subcommand("list", |s| s.about(l.t("cmd-chrome-profile-list")))
                .mut_subcommand("show", |s| pname(s.about(l.t("cmd-chrome-profile-show"))))
                .mut_subcommand("set", |s| {
                    pname(s.about(l.t("cmd-chrome-profile-set")))
                        .mut_arg("values", |a| a.help(l.t("arg-chrome-profile-values")))
                        .mut_arg("unset", |a| a.help(l.t("arg-chrome-profile-unset")))
                })
                .mut_subcommand("remove", |s| {
                    pname(s.about(l.t("cmd-chrome-profile-remove")))
                })
        })
}

fn browsers(l: &Localizer, arg: Option<&str>) -> anyhow::Result<Vec<Browser>> {
    match arg {
        None => {
            let found: Vec<Browser> = nm::ALL
                .iter()
                .copied()
                .filter(|b| nm::browser_exe(*b).is_some())
                .collect();
            Ok(if found.is_empty() {
                vec![Browser::Chrome]
            } else {
                found
            })
        }
        Some("all") => Ok(nm::ALL.to_vec()),
        Some(b) => Browser::parse(b).map(|b| vec![b]).ok_or_else(|| {
            anyhow::anyhow!(l.t_args("chrome-unknown-browser", &args!("browser" => b.to_owned())))
        }),
    }
}

/// The built extension for `browser`: `KOR_EXTENSION_DIR`, the install next to the CLI (Windows,
/// macOS bundle, Linux share), or the repository build (`frontend/extension/dist`).
pub fn extension_dir(browser: Browser) -> Option<PathBuf> {
    let flavour = if browser == Browser::Firefox {
        "firefox"
    } else {
        "chrome"
    };
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.to_path_buf();
    let mut c: Vec<PathBuf> = Vec::new();
    if let Some(v) = std::env::var_os("KOR_EXTENSION_DIR") {
        c.push(PathBuf::from(v).join(flavour));
    }
    c.push(dir.join("extension").join(flavour));
    c.push(dir.join("../Resources/extension").join(flavour));
    c.push(dir.join("../share/kor/extension").join(flavour));
    c.push(dir.join("../lib/kor/extension").join(flavour));
    c.push(dir.join("../../../frontend/extension/dist").join(flavour));
    c.into_iter()
        .find(|d| d.join("manifest.json").is_file())
        .map(|d| {
            d.canonicalize()
                .map(|p| PathBuf::from(kor_tools::display_path(&p)))
                .unwrap_or(d)
        })
}

fn domains_file(l: &Localizer) -> anyhow::Result<PathBuf> {
    policy::file().ok_or_else(|| anyhow::anyhow!(l.t("system-no-home")))
}

fn print_domains(l: &Localizer, d: &policy::Domains) {
    if d.allow.is_empty() && d.deny.is_empty() {
        println!("{}", l.t("chrome-domains-empty"));
        return;
    }
    for a in &d.allow {
        println!(
            "{}",
            l.t_args("chrome-domain-allowed", &args!("domain" => a.clone()))
        );
    }
    for x in &d.deny {
        println!(
            "{}",
            l.t_args("chrome-domain-denied", &args!("domain" => x.clone()))
        );
    }
}

fn state_dir(l: &Localizer) -> anyhow::Result<PathBuf> {
    kor_tools::chrome::state_dir().ok_or_else(|| anyhow::anyhow!(l.t("system-no-home")))
}

#[allow(clippy::too_many_lines)]
pub fn run(ctx: &Ctx, cmd: ChromeCmd) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let store = kor_secrets::KeychainStore;
    match cmd {
        ChromeCmd::Pair { revoke: true } => {
            pairing::revoke_all(&state_dir(l)?, &store).map_err(|e| anyhow::anyhow!(e))?;
            println!("{}", l.t("chrome-revoked"));
            Ok(true)
        }
        ChromeCmd::Pair { revoke: false } => {
            let (code, _) = pairing::new_code(&state_dir(l)?).map_err(|e| anyhow::anyhow!(e))?;
            println!("{}", l.t_args("chrome-pair-code", &args!("code" => code)));
            println!("{}", l.t("chrome-pair-how"));
            Ok(true)
        }
        ChromeCmd::Install {
            browser,
            store: to_store,
            no_open,
            extension_id,
        } => {
            let env = nm::Env::system().ok_or_else(|| anyhow::anyhow!(l.t("system-no-home")))?;
            let kor = std::env::current_exe()?;
            let mut ok = true;
            for b in browsers(l, browser.as_deref())? {
                match nm::install(&env, b, &kor, &extension_id) {
                    Ok(i) => println!(
                        "{}",
                        l.t_args(
                            "chrome-host-installed",
                            &args!("browser" => b.id(), "path" => i.manifest.display().to_string())
                        )
                    ),
                    Err(e) => {
                        ok = false;
                        eprintln!(
                            "{}",
                            l.t_args(
                                "chrome-host-failed",
                                &args!("browser" => b.id(), "error" => e.to_string())
                            )
                        );
                        continue;
                    }
                }
                if to_store {
                    let url = if b == Browser::Firefox {
                        FIREFOX_STORE
                    } else {
                        CHROME_STORE
                    };
                    println!("{}", l.t_args("chrome-store-open", &args!("url" => url)));
                    if !no_open {
                        let _ = nm::open(url);
                    }
                    continue;
                }
                match extension_dir(b) {
                    Some(dir) => {
                        let key = if b == Browser::Firefox {
                            "chrome-load-firefox"
                        } else {
                            "chrome-load-unpacked"
                        };
                        println!(
                            "{}",
                            l.t_args(
                                key,
                                &args!("browser" => b.id(), "path" => dir.display().to_string())
                            )
                        );
                        if !no_open {
                            let page = if b == Browser::Firefox {
                                "about:debugging#/runtime/this-firefox"
                            } else {
                                "chrome://extensions"
                            };
                            let _ = nm::open_in(b, page);
                        }
                    }
                    None => {
                        ok = false;
                        eprintln!("{}", l.t("chrome-no-build"));
                    }
                }
            }
            println!("{}", l.t("chrome-pair-next"));
            Ok(ok)
        }
        ChromeCmd::Uninstall { browser } => {
            let env = nm::Env::system().ok_or_else(|| anyhow::anyhow!(l.t("system-no-home")))?;
            let list = match browser.as_deref() {
                None => nm::ALL.to_vec(),
                b => browsers(l, b)?,
            };
            for b in list {
                if nm::uninstall(&env, b)? {
                    println!(
                        "{}",
                        l.t_args("chrome-host-removed", &args!("browser" => b.id()))
                    );
                }
            }
            Ok(true)
        }
        ChromeCmd::Status { json } => {
            let env = nm::Env::system().ok_or_else(|| anyhow::anyhow!(l.t("system-no-home")))?;
            let hosts: BTreeMap<&str, bool> = nm::ALL
                .iter()
                .map(|b| (b.id(), nm::installed(&env, *b)))
                .collect();
            let paired = pairing::paired_count(&store);
            let waiting = pairing::code_pending(&state_dir(l)?);
            let domains = policy::current();
            let backend = ctx
                .config
                .as_ref()
                .map(|c| format!("{:?}", c.config.browser.backend).to_lowercase())
                .unwrap_or_default();
            let dir = extension_dir(Browser::Chrome);
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "hosts": hosts, "paired": paired, "codeWaiting": waiting, "domains": domains,
                        "backend": backend, "extensionDir": dir, "extensionId": nm::CHROME_EXTENSION_ID,
                    }))?
                );
                return Ok(true);
            }
            for (b, on) in &hosts {
                let key = if *on {
                    "chrome-status-host-on"
                } else {
                    "chrome-status-host-off"
                };
                println!("{}", l.t_args(key, &args!("browser" => *b)));
            }
            println!(
                "{}",
                l.t_args("chrome-status-paired", &args!("count" => paired))
            );
            if waiting {
                println!("{}", l.t("chrome-status-code-waiting"));
            }
            println!(
                "{}",
                l.t_args("chrome-status-backend", &args!("backend" => backend))
            );
            match dir {
                Some(d) => println!(
                    "{}",
                    l.t_args(
                        "chrome-status-build",
                        &args!("path" => d.display().to_string())
                    )
                ),
                None => println!("{}", l.t("chrome-no-build")),
            }
            print_domains(l, &domains);
            Ok(true)
        }
        ChromeCmd::Allow { domain } | ChromeCmd::Deny { domain }
            if policy::normalize(&domain).is_err() =>
        {
            eprintln!(
                "{}",
                l.t_args("chrome-bad-domain", &args!("domain" => domain))
            );
            Ok(false)
        }
        ChromeCmd::Allow { domain } => {
            let f = domains_file(l)?;
            let mut d = policy::Domains::load(&f);
            let name = d.allow(&domain).map_err(|e| anyhow::anyhow!(e))?;
            d.save(&f).map_err(|e| anyhow::anyhow!(e))?;
            println!(
                "{}",
                l.t_args("chrome-domain-allowed", &args!("domain" => name))
            );
            Ok(true)
        }
        ChromeCmd::Deny { domain } => {
            let f = domains_file(l)?;
            let mut d = policy::Domains::load(&f);
            let name = d.deny(&domain).map_err(|e| anyhow::anyhow!(e))?;
            d.save(&f).map_err(|e| anyhow::anyhow!(e))?;
            println!(
                "{}",
                l.t_args("chrome-domain-denied", &args!("domain" => name))
            );
            Ok(true)
        }
        ChromeCmd::Domains { remove } => {
            let f = domains_file(l)?;
            let mut d = policy::Domains::load(&f);
            if let Some(r) = remove {
                let removed = d.forget(&r).map_err(|e| anyhow::anyhow!(e))?;
                d.save(&f).map_err(|e| anyhow::anyhow!(e))?;
                let key = if removed {
                    "chrome-domain-removed"
                } else {
                    "chrome-domain-not-listed"
                };
                println!("{}", l.t_args(key, &args!("domain" => r)));
            }
            print_domains(l, &d);
            Ok(true)
        }
        ChromeCmd::Profile { command } => profile(l, &store, command),
    }
}

fn profile(
    l: &Localizer,
    store: &dyn kor_secrets::SecretStore,
    cmd: ProfileCmd,
) -> anyhow::Result<bool> {
    use kor_runtime::browser_assist::forms;
    match cmd {
        ProfileCmd::List => {
            let names = forms::list(store);
            if names.is_empty() {
                println!("{}", l.t("chrome-profile-none"));
            }
            for n in names {
                println!("{n}");
            }
            Ok(true)
        }
        ProfileCmd::Show { name } => {
            for (k, v) in forms::get(store, &name).map_err(|e| anyhow::anyhow!(e))? {
                println!("{k} = {v}");
            }
            Ok(true)
        }
        ProfileCmd::Set {
            name,
            values,
            unset,
        } => {
            let mut changes = BTreeMap::new();
            for kv in &values {
                let Some((k, v)) = kv.split_once('=') else {
                    eprintln!(
                        "{}",
                        l.t_args("chrome-profile-bad-pair", &args!("value" => kv.clone()))
                    );
                    return Ok(false);
                };
                changes.insert(k.trim().to_owned(), v.trim().to_owned());
            }
            for k in unset {
                changes.insert(k, String::new());
            }
            let p = forms::learn(store, &name, &changes).map_err(|e| anyhow::anyhow!(e))?;
            println!(
                "{}",
                l.t_args(
                    "chrome-profile-saved",
                    &args!("name" => name, "count" => p.len())
                )
            );
            Ok(true)
        }
        ProfileCmd::Remove { name } => {
            let existed = forms::remove(store, &name).map_err(|e| anyhow::anyhow!(e))?;
            let key = if existed {
                "chrome-profile-removed"
            } else {
                "chrome-profile-missing"
            };
            println!("{}", l.t_args(key, &args!("name" => name)));
            Ok(existed)
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::*;

    #[derive(Debug, clap::Parser)]
    struct T {
        #[command(subcommand)]
        c: ChromeCmd,
    }

    #[test]
    fn parses() {
        assert!(matches!(
            T::parse_from(["t", "pair"]).c,
            ChromeCmd::Pair { revoke: false }
        ));
        assert!(matches!(
            T::parse_from(["t", "pair", "--revoke"]).c,
            ChromeCmd::Pair { revoke: true }
        ));
        let ChromeCmd::Install {
            browser,
            store,
            no_open,
            extension_id,
        } = T::parse_from([
            "t",
            "install",
            "--browser",
            "firefox",
            "--no-open",
            "--extension-id",
            "abc",
        ])
        .c
        else {
            panic!("install")
        };
        assert_eq!(
            (browser.as_deref(), store, no_open, extension_id.len()),
            (Some("firefox"), false, true, 1)
        );
        assert!(matches!(
            T::parse_from(["t", "allow", "example.com"]).c,
            ChromeCmd::Allow { .. }
        ));
        assert!(matches!(
            T::parse_from(["t", "deny", "evil.com"]).c,
            ChromeCmd::Deny { .. }
        ));
        assert!(matches!(
            T::parse_from(["t", "domains", "--remove", "x.com"]).c,
            ChromeCmd::Domains { remove: Some(_) }
        ));
        assert!(matches!(
            T::parse_from(["t", "status", "--json"]).c,
            ChromeCmd::Status { json: true }
        ));
        let ChromeCmd::Profile {
            command: ProfileCmd::Set { values, unset, .. },
        } = T::parse_from([
            "t",
            "profile",
            "set",
            "is",
            "email=a@b.co",
            "--unset",
            "phone",
        ])
        .c
        else {
            panic!("profile set")
        };
        assert_eq!((values.len(), unset.len()), (1, 1));
        let _ = localize(
            <T as clap::CommandFactory>::command(),
            &Localizer::new(kor_i18n::Lang::Tr),
        );
        assert!(browsers(&Localizer::new(kor_i18n::Lang::En), Some("safari")).is_err());
        assert_eq!(
            browsers(&Localizer::new(kor_i18n::Lang::En), Some("all"))
                .unwrap()
                .len(),
            5
        );
    }
}
