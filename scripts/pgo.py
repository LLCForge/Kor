#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Profile-guided release build of the `kor` CLI (spec §32.2, cargo-pgo style). Standard library only.

  profile [--target T] [--out FILE] [--records N]   instrumented build + training run + merge -> FILE
                                                    (default backend/target/pgo/merged.profdata)
  rustflags FILE                                    print the RUSTFLAGS that use FILE
  build [--target T] [--records N]                  profile, then the PGO release build of kor-cli
  selftest                                          argument and path handling, no build

Training run = the performance bench workload (`kor bench --cli --ui --memory`, a smaller knowledge
base by default) in a throwaway Kor home, so the hot paths the budgets measure are the ones the
optimizer sees. `llvm-profdata` comes from the rustup component `llvm-tools` (the same LLVM as rustc).
The instrumented build skips fat LTO (much faster to link; function profiles still match the LTO build).

A cross target the runner cannot execute (e.g. aarch64-apple-darwin on an Intel runner) cannot be
trained: `profile` then prints a notice and exits 3, and callers build without PGO.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BACKEND = ROOT / "backend"
NOT_RUNNABLE = 3


def host_triple() -> str:
    out = subprocess.run(["rustc", "-vV"], check=True, capture_output=True, text=True).stdout
    return next(l.split(": ", 1)[1] for l in out.splitlines() if l.startswith("host: "))


def llvm_profdata() -> Path:
    sysroot = subprocess.run(["rustc", "--print", "sysroot"], check=True, capture_output=True, text=True).stdout.strip()
    exe = "llvm-profdata.exe" if os.name == "nt" else "llvm-profdata"
    p = Path(sysroot, "lib", "rustlib", host_triple(), "bin", exe)
    if not p.is_file():
        raise SystemExit(f"{p} missing: rustup component add llvm-tools")
    return p


def runnable(target: str | None, host: str) -> bool:
    if not target or target == host:
        return True
    # A static musl binary runs on a glibc host of the same arch; Apple Silicon runs x86_64 macOS
    # binaries under Rosetta. Nothing else runs a foreign target here.
    if target.endswith("-unknown-linux-musl") and host == target.replace("-musl", "-gnu"):
        return True
    return host == "aarch64-apple-darwin" and target == "x86_64-apple-darwin"


def use_flags(profdata: Path) -> str:
    # Functions the training never reached have no profile: not a warning worth failing on.
    return f"-Cprofile-use={profdata} -Cllvm-args=-pgo-warn-missing-function=false"


def cargo_build(target: str | None, env: dict[str, str]) -> Path:
    cmd = ["cargo", "build", "--release", "--locked", "-p", "kor-cli"] + (["--target", target] if target else [])
    subprocess.run(cmd, cwd=BACKEND, env=env, check=True)
    tdir = Path(env.get("CARGO_TARGET_DIR") or BACKEND / "target")
    exe = "kor.exe" if (target and "windows" in target) or (not target and os.name == "nt") else "kor"
    return tdir / (target or "") / "release" / exe


def base_env() -> dict[str, str]:
    env = dict(os.environ)
    env.setdefault("RUST_MIN_STACK", "268435456")
    return env


def profile(target: str | None, out: Path, records: int) -> int:
    host = host_triple()
    if not runnable(target, host):
        print(f"::notice::PGO skipped for {target}: a {host} runner cannot run the training binary")
        return NOT_RUNNABLE
    work = BACKEND / "target" / "pgo"
    raw = work / "raw"
    shutil.rmtree(raw, ignore_errors=True)
    raw.mkdir(parents=True)
    env = base_env()
    env["CARGO_TARGET_DIR"] = str(work / "instrumented")
    env["RUSTFLAGS"] = (env.get("RUSTFLAGS", "") + f" -Cprofile-generate={raw}").strip()
    env["CARGO_PROFILE_RELEASE_LTO"] = "false"
    env["CARGO_PROFILE_RELEASE_CODEGEN_UNITS"] = "16"
    kor = cargo_build(target, env)
    with tempfile.TemporaryDirectory(prefix="kor-pgo-") as d:
        run_env = dict(os.environ)
        for k, sub in (("KOR_HOME", "home"), ("KOR_CONFIG_DIR", "config"), ("KOR_CACHE_DIR", "cache")):
            run_env[k] = str(Path(d, sub))
        # Every process the bench starts (`kor --version`, `kor help`, the REPL) writes its own profile.
        run_env["LLVM_PROFILE_FILE"] = str(raw / "kor-%p-%m.profraw")
        cmd = [str(kor), "--lang", "en", "bench", "--cli", "--ui", "--memory",
               "--records", str(records), "--history", str(Path(d, "history.json"))]
        print("training:", " ".join(cmd), flush=True)
        # Budgets may be over on a CI runner; the profile is what matters here.
        r = subprocess.run(cmd, cwd=d, env=run_env)
        print(f"training exit code {r.returncode}")
    profiles = sorted(raw.glob("*.profraw"))
    if not profiles:
        raise SystemExit("the training run wrote no .profraw files")
    out.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run([str(llvm_profdata()), "merge", "-o", str(out), *map(str, profiles)], check=True)
    print(f"merged {len(profiles)} profiles into {out}")
    return 0


def parse(rest: list[str]) -> dict[str, str | None]:
    opts: dict[str, str | None] = {"--target": None, "--out": None, "--records": "50000"}
    it = iter(rest)
    for a in it:
        if a not in opts:
            raise SystemExit(f"unknown option {a}")
        opts[a] = next(it, None)
    return opts


def selftest() -> None:
    assert runnable(None, "x86_64-unknown-linux-gnu")
    assert runnable("x86_64-pc-windows-msvc", "x86_64-pc-windows-msvc")
    assert not runnable("aarch64-apple-darwin", "x86_64-apple-darwin")
    assert runnable("x86_64-apple-darwin", "aarch64-apple-darwin")
    assert runnable("x86_64-unknown-linux-musl", "x86_64-unknown-linux-gnu")
    assert not runnable("aarch64-pc-windows-msvc", "x86_64-pc-windows-msvc")
    assert parse(["--target", "t", "--records", "10"]) == {"--target": "t", "--out": None, "--records": "10"}
    flags = use_flags(Path("/x/merged.profdata"))
    assert flags.startswith("-Cprofile-use=") and "merged.profdata" in flags
    print("pgo selftest ok")


def main(argv: list[str]) -> int:
    if not argv or argv[0] in ("-h", "--help"):
        print(__doc__)
        return 0
    cmd, rest = argv[0], argv[1:]
    if cmd == "selftest":
        selftest()
        return 0
    if cmd == "rustflags" and len(rest) == 1:
        print(use_flags(Path(rest[0]).resolve()))
        return 0
    o = parse(rest)
    out = Path(o["--out"] or BACKEND / "target" / "pgo" / "merged.profdata").resolve()
    records = int(o["--records"] or "50000")
    if cmd == "profile":
        return profile(o["--target"], out, records)
    if cmd == "build":
        try:
            code = profile(o["--target"], out, records)
        except (subprocess.CalledProcessError, SystemExit) as e:
            # A release is never blocked by its optimizer: build it without PGO and say so.
            print(f"::warning::PGO profile failed ({e}); building without PGO")
            code = 1
        env = base_env()
        if code == 0:
            env["RUSTFLAGS"] = (env.get("RUSTFLAGS", "") + " " + use_flags(out)).strip()
        print(cargo_build(o["--target"], env))
        return 0
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
