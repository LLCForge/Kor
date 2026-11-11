//! Faz 7 acceptance test 1 (spec §14 item 8, ADR 0045): every `kor audit` finding is reported with
//! `file:line` and the rule that produced it.
//!
//! Four small fixture repositories (Rust, Python, JS, PHP/Laravel-ish) with planted issues are
//! written to a temp dir (secrets are assembled at runtime so this repository never holds one).
//! For each: every finding has a non-empty rule and source, names a real file and a 1-based line
//! inside it; the planted issues are found; SARIF passes the structural check.

use std::collections::BTreeSet;
use std::path::Path;

use kor_i18n::{Lang, Localizer};
use kor_runtime::analysis::compliance::{self, Regime};
use kor_runtime::analysis::security::{self, Options};
use kor_runtime::analysis::{Format, Report, check_sarif, render};

fn gh_pat() -> String {
    format!("ghp_{}", "x1Y2z3".repeat(6))
}

fn aws_key() -> String {
    format!("AKIA{}", "QWERTYUIOPASDFGH")
}

fn write(root: &Path, files: &[(&str, String)]) {
    for (rel, text) in files {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
}

fn rust_repo(root: &Path) {
    write(
        root,
        &[
            (
                "src/main.rs",
                format!(
                    r#"use std::process::Command;

fn handler(name: &str) {{
    let q = format!("SELECT * FROM users WHERE name = '{{name}}'");
    let _ = Command::new("sh").arg("-c").arg(name);
    let c = reqwest::Client::builder().danger_accept_invalid_certs(true);
    let token = "{}";
    tracing::info!("signup {{}}", user.email);
}}

fn app() -> Router {{
    Router::new().route("/users", post(create_user))
}}
"#,
                    gh_pat()
                ),
            ),
            (
                "migrations/001_init.sql",
                "CREATE TABLE users (\n  id INTEGER PRIMARY KEY,\n  email TEXT NOT NULL,\n  tc_kimlik_no TEXT,\n  blood_type TEXT\n);\n".into(),
            ),
            (
                "src/repo.rs",
                "pub fn load(db: &Db) {\n    db.query(\"SELECT email FROM users\");\n}\n".into(),
            ),
        ],
    );
}

fn python_repo(root: &Path) {
    write(
        root,
        &[
            (
                "app.py",
                format!(
                    r#"import hashlib, os, pickle, random, subprocess, logging
import requests
from flask import Flask, request

app = Flask(__name__)
AWS_KEY = "{}"


@app.route("/download")
def download():
    return open(os.path.join("files", request.args["name"])).read()


def run(cmd):
    subprocess.run(cmd, shell=True)


def find(cur, name):
    cur.execute(f"SELECT * FROM users WHERE name = '{{name}}'")


def load(blob):
    return pickle.loads(blob)


def hash_pw(password):
    return hashlib.md5(password.encode()).hexdigest()


def reset_token():
    reset_code = "".join(random.choice("abc123") for _ in range(20))
    requests.get("https://example.org", verify=False)
    logging.info("reset for %s", user.email)
    return reset_code
"#,
                    aws_key()
                ),
            ),
            (
                "shop/models.py",
                "from django.db import models\n\n\nclass Customer(models.Model):\n    full_name = models.CharField(max_length=100)\n    phone_number = models.CharField(max_length=20)\n    iban = models.CharField(max_length=34)\n".into(),
            ),
        ],
    );
}

fn js_repo(root: &Path) {
    write(
        root,
        &[(
            "server.js",
            r#"const { exec } = require('child_process');
const crypto = require('crypto');
const https = require('https');
const path = require('path');
const express = require('express');
const app = express();

app.post('/run', (req, res) => {
  exec(`ls ${req.body.dir}`);
});

app.get('/file', (req, res) => res.sendFile(path.join(__dirname, req.query.name)));

async function user(db, req) {
  return db.query(`SELECT * FROM users WHERE id = ${req.params.id}`);
}

const sessionToken = Math.random().toString(36);
const agent = new https.Agent({ rejectUnauthorized: false });
const iv = Buffer.from('0123456789abcdef');
const cipher = crypto.createCipheriv('aes-128-ecb', key, null);
console.log('login', user.email);
const password = "SuperSecret123!";
"#
            .into(),
        )],
    );
}

fn laravel_repo(root: &Path) {
    write(
        root,
        &[
            (
                "routes/web.php",
                "<?php\n\nuse App\\Http\\Controllers\\UserController;\n\nRoute::post('/users', [UserController::class, 'store']);\n".into(),
            ),
            (
                "app/Http/Controllers/UserController.php",
                r#"<?php

namespace App\Http\Controllers;

class UserController extends Controller
{
    public function show($id)
    {
        $users = DB::select("SELECT * FROM users WHERE id = $id");
        $raw = mysqli_query($conn, "SELECT * FROM t WHERE a = '" . $_GET['a'] . "'");
        shell_exec("convert " . $request->input('file'));
        $cart = unserialize($_COOKIE['cart']);
        include($_GET['page']);
        $hash = md5($password);
        curl_setopt($ch, CURLOPT_SSL_VERIFYPEER, false);
        $token = bin2hex(mt_rand());
        Log::info('registered', ['email' => $user->email]);
    }
}
"#
                .into(),
            ),
            (
                "database/seeders/UserSeeder.php",
                "<?php\n\n$rows = [['tc_kimlik_no' => '10000000146', 'iban' => 'TR33 0006 1005 1978 6457 8413 26']];\n".into(),
            ),
        ],
    );
}

fn scan(root: &Path, l: &Localizer) -> Report {
    let mut r = security::scan(root, &Options { no_external: true }, l);
    let c = compliance::scan(root, Regime::Kvkk, l);
    r.findings.extend(c.findings);
    r.normalize();
    r
}

fn check(root: &Path, r: &Report) {
    assert!(!r.findings.is_empty());
    for f in &r.findings {
        assert!(!f.rule.trim().is_empty(), "finding without rule: {f:?}");
        assert!(!f.source.trim().is_empty(), "finding without source: {f:?}");
        let path = root.join(&f.file);
        assert!(path.is_file(), "{} is not a real file", f.file.display());
        let lines = std::fs::read_to_string(&path).unwrap().lines().count();
        let line = f.line as usize;
        assert!(
            (1..=lines).contains(&line),
            "{}:{} outside 1..={lines}",
            f.file.display(),
            f.line
        );
        assert!(!f.message.trim().is_empty());
    }
    let l = Localizer::new(Lang::En);
    let sarif: serde_json::Value =
        serde_json::from_str(&render(r, Format::Sarif, &l)).expect("SARIF is JSON");
    check_sarif(&sarif).expect("SARIF 2.1.0 structure");
    assert_eq!(
        sarif["runs"][0]["results"].as_array().unwrap().len(),
        r.findings.len()
    );
    let md = render(r, Format::Markdown, &Localizer::new(Lang::Tr));
    for f in &r.findings {
        assert!(
            md.contains(&f.location().replace('|', "\\|")),
            "{}",
            f.location()
        );
    }
}

fn rules(r: &Report, file: &str) -> BTreeSet<String> {
    r.findings
        .iter()
        .filter(|f| f.file == Path::new(file))
        .map(|f| f.rule.clone())
        .collect()
}

fn expect(r: &Report, file: &str, want: &[&str]) {
    let got = rules(r, file);
    for w in want {
        assert!(got.contains(*w), "{file}: missing {w}; got {got:?}");
    }
}

#[test]
fn rust_fixture() {
    let dir = tempfile::tempdir().unwrap();
    rust_repo(dir.path());
    let r = scan(dir.path(), &Localizer::new(Lang::En));
    check(dir.path(), &r);
    expect(
        &r,
        "src/main.rs",
        &[
            "github-pat",
            "kor.sql-injection.rust",
            "kor.command-injection.rust-shell",
            "kor.tls-verify-disabled",
            "kor.open-endpoint.rust",
            "kor.pii.log",
        ],
    );
    expect(
        &r,
        "migrations/001_init.sql",
        &[
            "kor.pii.column",
            "kor.pii.special-column",
            "kor.pii.retention",
        ],
    );
    expect(&r, "src/repo.rs", &["kor.pii.table-access"]);
    let secret = r.findings.iter().find(|f| f.rule == "github-pat").unwrap();
    assert_eq!((secret.line, secret.source.as_str()), (7, "kor"));
    let kvkk = r
        .findings
        .iter()
        .find(|f| f.rule == "kor.pii.special-column")
        .unwrap();
    assert!(kvkk.message.contains("KVKK m.6"), "{}", kvkk.message);
}

#[test]
fn python_fixture() {
    let dir = tempfile::tempdir().unwrap();
    python_repo(dir.path());
    let r = scan(dir.path(), &Localizer::new(Lang::Tr));
    check(dir.path(), &r);
    expect(
        &r,
        "app.py",
        &[
            "aws-access-token",
            "kor.path-traversal.python",
            "kor.command-injection.python-shell",
            "kor.sql-injection.python",
            "kor.deserialization.python",
            "kor.weak-password-hash",
            "kor.weak-hash",
            "kor.insecure-random",
            "kor.tls-verify-disabled",
            "kor.pii.log",
        ],
    );
    expect(&r, "shop/models.py", &["kor.pii.column"]);
}

#[test]
fn js_fixture() {
    let dir = tempfile::tempdir().unwrap();
    js_repo(dir.path());
    let r = scan(dir.path(), &Localizer::new(Lang::Fr));
    check(dir.path(), &r);
    expect(
        &r,
        "server.js",
        &[
            "kor.command-injection.node",
            "kor.open-endpoint.express",
            "kor.open-endpoint.express-read",
            "kor.path-traversal.node",
            "kor.sql-injection.js",
            "kor.insecure-random",
            "kor.tls-verify-disabled",
            "kor.hardcoded-iv",
            "kor.ecb-mode",
            "kor.hardcoded-credential",
            "kor.pii.log",
        ],
    );
}

#[test]
fn laravel_fixture() {
    let dir = tempfile::tempdir().unwrap();
    laravel_repo(dir.path());
    let r = scan(dir.path(), &Localizer::new(Lang::En));
    check(dir.path(), &r);
    expect(&r, "routes/web.php", &["kor.open-endpoint.laravel"]);
    expect(
        &r,
        "app/Http/Controllers/UserController.php",
        &[
            "kor.sql-injection.php",
            "kor.command-injection.php",
            "kor.deserialization.php",
            "kor.path-traversal.php",
            "kor.weak-password-hash",
            "kor.tls-verify-disabled",
            "kor.insecure-random",
            "kor.pii.log",
        ],
    );
    expect(
        &r,
        "database/seeders/UserSeeder.php",
        &["kor.pii.literal-national-id", "kor.pii.literal-iban"],
    );
}

#[test]
fn gdpr_references_differ_from_kvkk() {
    let dir = tempfile::tempdir().unwrap();
    rust_repo(dir.path());
    let l = Localizer::new(Lang::En);
    let g = compliance::scan(dir.path(), Regime::Gdpr, &l);
    let f = g
        .findings
        .iter()
        .find(|f| f.rule == "kor.pii.retention")
        .unwrap();
    assert!(f.message.contains("GDPR Art. 5(1)(e)"), "{}", f.message);
    assert_eq!(g.tool, "compliance-gdpr");
}
