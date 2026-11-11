//! Performance acceptance (spec §14, Faz 3): incremental update of one file in a 100K+ line repo
//! under 1 s. Ignored by default; run with
//! `cargo test -p kor-graph --release --test perf -- --ignored --nocapture`.

use kor_graph::Graph;
use std::fmt::Write as _;
use std::path::Path;
use std::time::Instant;

const FILES: usize = 500;
const FNS: usize = 20;

fn rust_file(i: usize) -> String {
    let dep = (i + 5) % FILES;
    let mut s = format!("use crate::m{dep}::f{dep}_0;\n\n");
    for k in 0..FNS {
        let _ = write!(
            s,
            "/// Synthetic function {k}.\npub fn f{i}_{k}(x: u64) -> u64 {{\n    let mut acc = x;\n    for n in 0..3 {{\n        acc = acc.wrapping_add(n);\n    }}\n    // NOTE: synthetic call chain\n    acc = f{dep}_{k}(acc);\n    if acc > 10 {{ acc -= 1; }}\n    acc\n}}\n\n"
        );
    }
    s
}

fn py_file(i: usize) -> String {
    let dep = (i + 5) % FILES;
    let mut s = format!("from .m{dep} import f{dep}_0\nimport os\n\n");
    for k in 0..FNS {
        let _ = write!(
            s,
            "def f{i}_{k}(x):\n    \"\"\"Synthetic function {k}.\"\"\"\n    acc = x\n    for n in range(3):\n        acc += n\n    acc = f{dep}_{k}(acc)\n    if acc > 10:\n        acc -= 1\n    return acc + len(os.environ.get('HOME', ''))\n\n"
        );
    }
    s
}

fn ts_file(i: usize) -> String {
    let dep = (i + 5) % FILES;
    let mut s = format!("import {{ f{dep}_0 }} from './m{dep}';\n\n");
    for k in 0..FNS {
        let _ = write!(
            s,
            "export function f{i}_{k}(x: number): number {{\n  let acc = x;\n  for (let n = 0; n < 3; n++) {{\n    acc += n;\n  }}\n  // WHY: synthetic\n  acc = f{dep}_{k}(acc);\n  if (acc > 10) {{ acc -= 1; }}\n  return acc;\n}}\n\n"
        );
    }
    s
}

fn go_file(i: usize) -> String {
    let dep = (i + 5) % FILES;
    let mut s = String::from("package main\n\nimport \"os\"\n\n");
    for k in 0..FNS {
        let _ = write!(
            s,
            "func F{i}_{k}(x int) int {{\n\tacc := x\n\tfor n := 0; n < 3; n++ {{\n\t\tacc += n\n\t}}\n\tacc = F{dep}_{k}(acc)\n\tif acc > 10 {{\n\t\tacc--\n\t}}\n\treturn acc + len(os.Getenv(\"HOME\"))\n}}\n\n"
        );
    }
    s
}

fn java_file(i: usize) -> String {
    let dep = (i + 5) % FILES;
    let mut s = format!("package gen;\n\nimport gen.M{dep};\n\npublic class M{i} {{\n");
    for k in 0..FNS {
        let _ = write!(
            s,
            "    public static int f{i}_{k}(int x) {{\n        int acc = x;\n        for (int n = 0; n < 3; n++) {{\n            acc += n;\n        }}\n        acc = M{dep}.f{dep}_{k}(acc);\n        if (acc > 10) {{ acc -= 1; }}\n        return acc;\n    }}\n\n"
        );
    }
    s.push_str("}\n");
    s
}

fn write_repo(root: &Path) -> usize {
    let mut lines = 0;
    for i in 0..FILES {
        let (path, text) = match i % 5 {
            0 => (format!("rs/src/m{i}.rs"), rust_file(i)),
            1 => (format!("py/pkg/m{i}.py"), py_file(i)),
            2 => (format!("ts/src/m{i}.ts"), ts_file(i)),
            3 => (format!("go/m{i}.go"), go_file(i)),
            _ => (format!("java/gen/M{i}.java"), java_file(i)),
        };
        lines += text.lines().count();
        let p = root.join(path);
        std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
        std::fs::write(p, text).expect("write");
    }
    lines
}

#[test]
#[ignore = "performance measurement; run with --release --ignored"]
fn incremental_update_100k_lines_under_1s() -> kor_graph::Result<()> {
    let dir = tempfile::tempdir()?;
    let lines = write_repo(dir.path());
    assert!(lines >= 100_000, "only {lines} lines");

    let t = Instant::now();
    let mut g = Graph::open(dir.path())?;
    let full = g.update()?;
    let full_ms = t.elapsed().as_millis();

    // Edit one file: add a function and a new cross-file call.
    let target = dir.path().join("rs/src/m0.rs");
    let mut text = std::fs::read_to_string(&target)?;
    text.push_str("pub fn added_now(x: u64) -> u64 { f10_3(x) }\n");
    std::fs::write(&target, text)?;

    let t = Instant::now();
    let inc = g.update_files(std::slice::from_ref(&target))?;
    let inc_ms = t.elapsed().as_millis();
    assert_eq!(inc.parsed, 1);
    assert!(g.node("rs/src/m0.rs#added_now").is_some());

    // Same edit seen by a full rescan (mtime/size fast path for the other files).
    std::fs::write(
        &target,
        std::fs::read_to_string(&target)? + "pub fn added_again() {}\n",
    )?;
    let t = Instant::now();
    let rescan = g.update()?;
    let rescan_ms = t.elapsed().as_millis();
    assert_eq!(rescan.parsed, 1);

    // Reopen from SQLite + AST cache.
    drop(g);
    let t = Instant::now();
    let g = Graph::open(dir.path())?;
    let open_ms = t.elapsed().as_millis();

    println!(
        "lines={lines} files={FILES} nodes={} edges={} communities={} | full build {full_ms} ms (parsed {}) | update_files(1) {inc_ms} ms | update() rescan {rescan_ms} ms | reopen {open_ms} ms",
        full.nodes, full.edges, full.communities, full.parsed
    );
    drop(g);
    if !cfg!(debug_assertions) {
        assert!(inc_ms < 1000, "incremental update took {inc_ms} ms");
        assert!(rescan_ms < 1000, "rescan update took {rescan_ms} ms");
    }
    Ok(())
}

/// Full build of an existing project: `KOR_GRAPH_BENCH_ROOT=<dir>`. Writes `<dir>/.kor/graph`.
#[test]
#[ignore = "measures a real project; set KOR_GRAPH_BENCH_ROOT"]
fn full_build_of_real_project() -> kor_graph::Result<()> {
    let Ok(root) = std::env::var("KOR_GRAPH_BENCH_ROOT") else {
        return Ok(());
    };
    let t = Instant::now();
    let mut g = Graph::open(&root)?;
    let s = g.update()?;
    let ms = t.elapsed().as_millis();
    let st = g.status();
    println!(
        "root={root} files={} nodes={} edges={} communities={} parsed={} cached={} full build {ms} ms languages={:?}",
        st.files, st.nodes, st.edges, st.communities, s.parsed, s.cached, st.languages
    );
    let t = Instant::now();
    let again = g.update()?;
    println!(
        "no-op update {} ms (parsed {})",
        t.elapsed().as_millis(),
        again.parsed
    );
    Ok(())
}
