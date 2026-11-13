#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Reproducible-build check (spec §22.3): build the `kor` CLI twice from the committed tree and
compare the binaries byte for byte. Standard library only.

  build [--target T] [--work DIR] [--package P]   two release builds, then compare (exit 1 if they differ)
  compare A B                                      compare two files (sha256, first differing byte)
  selftest                                         compare() against generated files

The two builds differ in source directory (different paths and depths) and wall-clock time, so a
path or a timestamp that leaks into the binary shows up. Both use:
  - SOURCE_DATE_EPOCH = the commit time (`git log -1 --format=%ct`), for build scripts that stamp dates;
  - `--remap-path-prefix` for CARGO_HOME (registry sources), so no machine path is embedded;
  - the same CARGO_TARGET_DIR, emptied between the builds. OUT_DIR paths (generated code, C objects)
    are then equal, while every RUSTFLAGS string stays identical: cargo hashes RUSTFLAGS into crate
    metadata on some versions, so a per-build remap of the source dir would itself change symbols.
    Workspace sources need no remap: cargo passes them to rustc relative to the workspace root;
  - on MSVC `-C link-arg=/Brepro` (no PE timestamp); on macOS ZERO_AR_DATE=1.
"""

from __future__ import annotations

import hashlib
import io
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def sha256(p: Path) -> str:
    h = hashlib.sha256()
    with p.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def compare(a: Path, b: Path) -> tuple[bool, str]:
    """(equal, report). The report names both hashes and, when they differ, the first differing offset."""
    ha, hb = sha256(a), sha256(b)
    lines = [f"{ha}  {a}", f"{hb}  {b}"]
    if ha == hb:
        return True, "\n".join(lines + ["identical"])
    da, db = a.read_bytes(), b.read_bytes()
    first = next((i for i, (x, y) in enumerate(zip(da, db)) if x != y), min(len(da), len(db)))
    lines.append(f"DIFFERENT: sizes {len(da)} / {len(db)}, first difference at byte {first:#x}")
    return False, "\n".join(lines)


def export(dest: Path) -> None:
    """The committed tree (HEAD) into dest, as a release checkout would see it."""
    data = subprocess.run(["git", "archive", "--format=tar", "HEAD"], cwd=ROOT, check=True, capture_output=True).stdout
    dest.mkdir(parents=True, exist_ok=True)
    with tarfile.open(fileobj=io.BytesIO(data)) as t:
        t.extractall(dest, filter="data")


def build_env(target: str | None) -> dict[str, str]:
    env = dict(os.environ)
    epoch = subprocess.run(["git", "log", "-1", "--format=%ct"], cwd=ROOT, check=True, capture_output=True, text=True).stdout.strip()
    env["SOURCE_DATE_EPOCH"] = epoch
    cargo_home = env.get("CARGO_HOME") or str(Path.home() / ".cargo")
    flags = [f"--remap-path-prefix={cargo_home}=/cargo"]
    triple = target or ""
    if "windows-msvc" in triple or (not triple and sys.platform == "win32"):
        flags += ["-C", "link-arg=/Brepro"]
    if sys.platform == "darwin":
        env["ZERO_AR_DATE"] = "1"
    env["RUSTFLAGS"] = " ".join(flags)
    # The fat-LTO link of the CLI needs more stack than rustc's default.
    env.setdefault("RUST_MIN_STACK", "268435456")
    return env


def build(target: str | None, work: Path, package: str) -> int:
    target_dir = work / "target"
    windows = "windows" in target if target else sys.platform == "win32"
    exe = "kor.exe" if windows else "kor"
    out = []
    env = build_env(target)
    env["CARGO_TARGET_DIR"] = str(target_dir)
    for name, sub in (("a", "a/kor"), ("b", "b/second/copy/kor")):
        src = work / sub
        shutil.rmtree(src, ignore_errors=True)
        export(src)
        shutil.rmtree(target_dir, ignore_errors=True)
        cmd = ["cargo", "build", "--release", "--locked", "-p", package] + (["--target", target] if target else [])
        print(f"[{name}] {src}: {' '.join(cmd)}", flush=True)
        subprocess.run(cmd, cwd=src / "backend", env=env, check=True)
        built = target_dir / (target or "") / "release" / exe
        kept = work / f"{name}-{exe}"
        shutil.copy2(built, kept)
        out.append(kept)
    same, report = compare(out[0], out[1])
    print(report)
    return 0 if same else 1


def selftest() -> None:
    with tempfile.TemporaryDirectory() as d:
        a, b, c = Path(d, "a"), Path(d, "b"), Path(d, "c")
        a.write_bytes(b"kor" * 1000)
        b.write_bytes(b"kor" * 1000)
        c.write_bytes(b"kor" * 500 + b"KOR" + b"kor" * 499)
        same, report = compare(a, b)
        assert same and "identical" in report, report
        same, report = compare(a, c)
        assert not same and f"byte {1500:#x}" in report, report
        Path(d, "short").write_bytes(b"kor" * 10)
        same, report = compare(a, Path(d, "short"))
        assert not same and f"byte {30:#x}" in report and "sizes 3000 / 30" in report, report
    print("repro_check selftest ok")


def main(argv: list[str]) -> int:
    if not argv or argv[0] in ("-h", "--help"):
        print(__doc__)
        return 0
    cmd, rest = argv[0], argv[1:]
    if cmd == "selftest":
        selftest()
        return 0
    if cmd == "compare" and len(rest) == 2:
        same, report = compare(Path(rest[0]), Path(rest[1]))
        print(report)
        return 0 if same else 1
    if cmd == "build":
        opts = {"--target": None, "--work": None, "--package": "kor-cli"}
        it = iter(rest)
        for a in it:
            if a not in opts:
                print(f"unknown option {a}", file=sys.stderr)
                return 2
            opts[a] = next(it, None)
        work = Path(opts["--work"] or tempfile.mkdtemp(prefix="kor-repro-"))
        return build(opts["--target"], work, opts["--package"] or "kor-cli")
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
