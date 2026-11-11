//! `kor templates list|show|new` and `kor run template:<name> --input value` (spec 15.11).
//! Loading, validation and rendering live in `kor_compat::templates`; this file only prints.

use clap::Subcommand;
use kor_compat::Scope;
use kor_compat::templates::{self, Template};
use kor_i18n::{Localizer, args};

use crate::Ctx;
use crate::compat::{input_error, usage};

#[derive(Debug, Subcommand)]
pub enum TemplatesCmd {
    List,
    Show {
        name: String,
    },
    New {
        name: String,
        #[arg(long)]
        personal: bool,
    },
}

pub fn run(ctx: &Ctx, cmd: &TemplatesCmd) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let root = kor_runtime::project_root()?;
    match cmd {
        TemplatesCmd::List => {
            let c = kor_runtime::load_compat(&root);
            for t in templates::all(&c) {
                println!("  {:<24} {:<10} {}", t.name, source(l, &t), t.description);
            }
            for w in c.warnings.iter().filter(|w| w.contains("invalid template")) {
                println!(
                    "  {}",
                    l.t_args("templates-warning", &args!("text" => w.clone()))
                );
            }
            Ok(true)
        }
        TemplatesCmd::Show { name } => {
            let c = kor_runtime::load_compat(&root);
            let Some(t) = templates::find(&c, name) else {
                eprintln!(
                    "{}",
                    l.t_args("templates-not-found", &args!("name" => name.clone()))
                );
                return Ok(false);
            };
            show(l, &t);
            Ok(true)
        }
        TemplatesCmd::New { name, personal } => {
            let base = if *personal {
                dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home directory"))?
            } else {
                root
            };
            let dir = templates::dir(&base);
            match templates::create(&dir, name) {
                Ok(p) => {
                    println!(
                        "{}",
                        l.t_args(
                            "templates-created",
                            &args!("path" => kor_tools::display_path(&p), "name" => name.clone())
                        )
                    );
                    Ok(true)
                }
                Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => {
                    eprintln!(
                        "{}",
                        l.t_args("templates-bad-name", &args!("name" => name.clone()))
                    );
                    Ok(false)
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let p = dir.join(format!("{name}.md"));
                    eprintln!(
                        "{}",
                        l.t_args(
                            "templates-exists",
                            &args!("path" => kor_tools::display_path(&p))
                        )
                    );
                    Ok(false)
                }
                Err(e) => Err(e.into()),
            }
        }
    }
}

/// `kor run template:<name> --input value …`: validate, render, then one agent run with the
/// template's model (approvals are asked as in any interactive run).
pub fn run_template(ctx: &Ctx, target: &str, inputs: &[String]) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let Some(name) = target.strip_prefix("template:") else {
        // `kor run the tests` was a plain prompt before this subcommand existed: keep it one.
        let prompt = std::iter::once("run")
            .chain(std::iter::once(target))
            .chain(inputs.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        return crate::repl::run(
            ctx,
            &crate::repl::RunArgs {
                prompt: Some(prompt),
                ..crate::repl::RunArgs::default()
            },
        );
    };
    let root = kor_runtime::project_root()?;
    let c = kor_runtime::load_compat(&root);
    let Some(t) = templates::find(&c, name) else {
        eprintln!(
            "{}",
            l.t_args("templates-not-found", &args!("name" => name.to_owned()))
        );
        return Ok(false);
    };
    let values = match templates::values(&t, inputs) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{}", input_error(l, &t.name, &e));
            eprintln!(
                "{}",
                l.t_args("templates-usage", &args!("usage" => run_line(&t)))
            );
            return Ok(false);
        }
    };
    crate::repl::run(
        ctx,
        &crate::repl::RunArgs {
            prompt: Some(templates::render(&t, &values)),
            model: t.model.clone(),
            ..crate::repl::RunArgs::default()
        },
    )
}

fn source(l: &Localizer, t: &Template) -> String {
    let key = match &t.source {
        None => "templates-source-builtin",
        Some(s) if s.scope == Scope::Project => "templates-source-project",
        Some(s) => {
            let personal = dirs::home_dir().is_some_and(|h| s.path.starts_with(templates::dir(&h)));
            if personal {
                "templates-source-personal"
            } else {
                "templates-source-plugin"
            }
        }
    };
    l.t(key)
}

fn run_line(t: &Template) -> String {
    format!("kor run template:{} {}", t.name, usage(t))
        .trim_end()
        .to_owned()
}

fn show(l: &Localizer, t: &Template) {
    println!("{}  ({})", t.name, source(l, t));
    println!("  {}", t.description);
    if let Some(s) = &t.source {
        println!("  {}", kor_tools::display_path(&s.path));
    }
    if let Some(m) = &t.model {
        println!(
            "  {}",
            l.t_args("templates-model", &args!("model" => m.clone()))
        );
    }
    if !t.inputs.is_empty() {
        println!("\n{}", l.t("templates-inputs"));
        for i in &t.inputs {
            let state = match (&i.default, i.required) {
                (Some(d), _) => l.t_args("templates-default", &args!("value" => d.clone())),
                (None, true) => l.t("templates-required"),
                (None, false) => l.t("templates-optional"),
            };
            let d = i.description.as_deref().unwrap_or_default();
            println!(
                "  --{:<16} {:<22} {:<16} {d}",
                i.name,
                i.kind.to_string(),
                state
            );
        }
    }
    let list = |key: &str, items: &[String], numbered: bool| {
        if items.is_empty() {
            return;
        }
        println!("\n{}", l.t(key));
        for (n, s) in items.iter().enumerate() {
            if numbered {
                println!("  {}. {s}", n + 1);
            } else {
                println!("  - {s}");
            }
        }
    };
    list("templates-steps", &t.steps, true);
    list("templates-acceptance", &t.acceptance, false);
    println!(
        "\n{}",
        l.t_args("templates-usage", &args!("usage" => run_line(t)))
    );
}
