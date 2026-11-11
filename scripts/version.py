#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Kor version manager (ADR 0012).

Two kinds of versions:
  product    What users install and `kor update` compares (git tag vX.Y.Z). Lives in
             backend/Cargo.toml [workspace.package], kor-cli inherits it; the desktop app
             (frontend/src-tauri/Cargo.toml, tauri.conf.json, package.json, package-lock.json)
             must match it.
  component  Each backend/crates/<name> has its own version in its Cargo.toml, mirrored in the
             [workspace.dependencies] requirement.

backend/versions.toml records every version plus a content hash of each crate at the last
release, so `release` can tell which components changed and must be bumped.

Commands:
  show                          versions, and which crates changed since the last release
  check [--release] [--tag T]   consistency; with --release also: no unreleased changes, tag == product
  set <name|product> X.Y.Z      set one version (records that crate's current hash)
  baseline                      record current content as the current product version, without
                                bumping; only for work that ships in a product version not yet tagged
  release <patch|minor|major> [--minor C ...] [--major C ...]
                                bump every changed crate (patch unless listed), then the product;
                                the product bump must be at least the largest component bump
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BACKEND = ROOT / "backend"
CRATES = BACKEND / "crates"
LOCK = BACKEND / "versions.toml"
WS = BACKEND / "Cargo.toml"
DESKTOP_CARGO = ROOT / "frontend" / "src-tauri" / "Cargo.toml"
TAURI_CONF = ROOT / "frontend" / "src-tauri" / "tauri.conf.json"
PKG_JSON = ROOT / "frontend" / "package.json"
PKG_LOCK = ROOT / "frontend" / "package-lock.json"
SEMVER = re.compile(r"^(\d+)\.(\d+)\.(\d+)$")
LEVELS = ("patch", "minor", "major")


class Fail(Exception):
    pass


# ---------- files (byte-level, line endings preserved) ----------

def read(p: Path) -> str:
    return p.read_bytes().decode("utf-8")


def write(p: Path, text: str) -> None:
    p.write_bytes(text.encode("utf-8"))


def sub_once(text: str, pattern: str, repl: str, what: str) -> str:
    new, n = re.subn(pattern, repl, text, count=1, flags=re.M)
    if n != 1:
        raise Fail(f"could not find {what}")
    return new


def package_section_version(text: str) -> str | None:
    """`version = "x"` in [package]; None when it is `version.workspace = true`."""
    data = tomllib.loads(text)
    v = data.get("package", {}).get("version")
    return v if isinstance(v, str) else None


def set_package_version(p: Path, v: str) -> None:
    text = read(p)
    # The first `version` key at line start is the [package] one in every Kor manifest.
    text = sub_once(text, r'^version(\.workspace)?\s*=\s*[^\r\n]*', f'version = "{v}"', f"version in {p}")
    write(p, text)


def crates() -> list[str]:
    return sorted(d.name for d in CRATES.iterdir() if (d / "Cargo.toml").is_file())


def crate_hash(name: str) -> str:
    """Content hash of a crate, ignoring line endings, build output and its own version line."""
    root = CRATES / name
    h = hashlib.sha256()
    for f in sorted(p for p in root.rglob("*") if p.is_file() and "target" not in p.relative_to(root).parts):
        rel = f.relative_to(root).as_posix()
        data = f.read_bytes().replace(b"\r\n", b"\n")
        if rel == "Cargo.toml":
            data = re.sub(rb"(?m)^version(\.workspace)?\s*=[^\n]*\n", b"", data, count=1)
        h.update(rel.encode() + b"\0" + data + b"\0")
    return h.hexdigest()[:16]


# ---------- state ----------

def product_sources() -> dict[str, str | None]:
    ws = tomllib.loads(read(WS))
    out: dict[str, str | None] = {
        "backend/Cargo.toml [workspace.package]": ws["workspace"]["package"]["version"],
        "frontend/src-tauri/Cargo.toml": package_section_version(read(DESKTOP_CARGO)),
        "frontend/src-tauri/tauri.conf.json": json.loads(read(TAURI_CONF)).get("version"),
        "frontend/package.json": json.loads(read(PKG_JSON)).get("version"),
    }
    if PKG_LOCK.is_file():
        lock = json.loads(read(PKG_LOCK))
        out["frontend/package-lock.json"] = lock.get("version")
        out['frontend/package-lock.json packages[""]'] = lock.get("packages", {}).get("", {}).get("version")
    return out


def component_versions() -> dict[str, str | None]:
    return {c: package_section_version(read(CRATES / c / "Cargo.toml")) for c in crates()}


def workspace_reqs() -> dict[str, str | None]:
    deps = tomllib.loads(read(WS))["workspace"].get("dependencies", {})
    return {c: (deps.get(c) or {}).get("version") if isinstance(deps.get(c), dict) else None for c in crates()}


def load_lock() -> dict:
    if not LOCK.is_file():
        return {"product": None, "components": {}}
    data = tomllib.loads(read(LOCK))
    data.setdefault("components", {})
    return data


def save_lock(product: str, comps: dict[str, dict[str, str]]) -> None:
    lines = [
        "# Managed by scripts/version.py (ADR 0012). Do not edit by hand.",
        "# `hash` is each crate's content at its last release; a mismatch means unreleased changes.",
        f'product = "{product}"',
        "",
        "[components]",
    ]
    for c in sorted(comps):
        lines.append(f'{c} = {{ version = "{comps[c]["version"]}", hash = "{comps[c]["hash"]}" }}')
    LOCK.write_bytes(("\n".join(lines) + "\n").encode())


def changed() -> list[str]:
    lock = load_lock()["components"]
    return [c for c in crates() if c not in lock or lock[c].get("hash") != crate_hash(c)]


# ---------- writers ----------

def write_product(v: str) -> None:
    text = read(WS)
    head, sep, rest = text.partition("[workspace.package]")
    if not sep:
        raise Fail("no [workspace.package] in backend/Cargo.toml")
    write(WS, head + sep + sub_once(rest, r'^version\s*=\s*"[^"]*"', f'version = "{v}"', "workspace version"))
    set_package_version(DESKTOP_CARGO, v)
    for p, count in ((TAURI_CONF, 1), (PKG_JSON, 1), (PKG_LOCK, 2)):
        if not p.is_file():
            continue
        text, n = re.subn(r'"version"\s*:\s*"[^"]*"', f'"version": "{v}"', read(p), count=count)
        if n != count:
            raise Fail(f"could not find version in {p}")
        write(p, text)
    bad = {k: x for k, x in product_sources().items() if x != v}
    if bad:
        raise Fail(f"product version not applied everywhere: {bad}")


def write_component(c: str, v: str) -> None:
    set_package_version(CRATES / c / "Cargo.toml", v)
    text = read(WS)
    text = sub_once(
        text,
        rf'^({re.escape(c)}\s*=\s*\{{[^}}\r\n]*\bversion\s*=\s*")[^"]*(")',
        rf"\g<1>{v}\g<2>",
        f"[workspace.dependencies] entry for {c}",
    )
    write(WS, text)


def bump(v: str, level: str) -> str:
    m = SEMVER.match(v or "")
    if not m:
        raise Fail(f"not a plain X.Y.Z version: {v!r}")
    a, b, c = map(int, m.groups())
    return {"major": f"{a + 1}.0.0", "minor": f"{a}.{b + 1}.0", "patch": f"{a}.{b}.{c + 1}"}[level]


# ---------- commands ----------

def problems() -> list[str]:
    out = []
    prod = product_sources()
    want = prod["backend/Cargo.toml [workspace.package]"]
    out += [f"{k} is {x!r}, product is {want!r}" for k, x in prod.items() if x != want]
    lock = load_lock()
    if lock.get("product") != want:
        out.append(f"backend/versions.toml product is {lock.get('product')!r}, product is {want!r}")
    reqs = workspace_reqs()
    for c, v in component_versions().items():
        if v is None:
            out.append(f"crates/{c}: needs its own `version = \"X.Y.Z\"` (not version.workspace)")
            continue
        if reqs.get(c) != v:
            out.append(f"[workspace.dependencies] {c} requires {reqs.get(c)!r}, crate is {v!r}")
        if lock["components"].get(c, {}).get("version") != v:
            out.append(f"backend/versions.toml has {c} {lock['components'].get(c, {}).get('version')!r}, crate is {v!r}")
    return out


def cmd_show(_: argparse.Namespace) -> None:
    ch = set(changed())
    print(f"product {product_sources()['backend/Cargo.toml [workspace.package]']}")
    for c, v in component_versions().items():
        print(f"  {c:<16} {v}{'  (changed since last release)' if c in ch else ''}")


def cmd_check(a: argparse.Namespace) -> None:
    out = problems()
    if a.release:
        out += [f"crates/{c} changed since its last release: run scripts/version.py release" for c in changed()]
        prod = product_sources()["backend/Cargo.toml [workspace.package]"]
        if a.tag and a.tag.removeprefix("v") != prod:
            out.append(f"tag {a.tag} does not match product version {prod}")
    if out:
        raise Fail("\n".join(out))
    print("versions ok")


def cmd_set(a: argparse.Namespace) -> None:
    if not SEMVER.match(a.version):
        raise Fail(f"not a plain X.Y.Z version: {a.version}")
    lock = load_lock()
    comps = {c: dict(lock["components"].get(c, {})) for c in crates()}
    product = lock.get("product") or product_sources()["backend/Cargo.toml [workspace.package]"]
    if a.name == "product":
        write_product(a.version)
        product = a.version
    elif a.name in comps:
        write_component(a.name, a.version)
        comps[a.name] = {"version": a.version, "hash": crate_hash(a.name)}
    else:
        raise Fail(f"unknown component {a.name}; known: {', '.join(comps)}")
    for c, v in component_versions().items():  # keep the lock complete
        comps[c].setdefault("hash", crate_hash(c))
        comps[c]["version"] = v or comps[c].get("version", "")
    save_lock(product, comps)
    print(f"{a.name} = {a.version}")


def cmd_baseline(_: argparse.Namespace) -> None:
    """Record current content as part of the current, not yet tagged, product version."""
    if out := problems():
        raise Fail("fix these first:\n" + "\n".join(out))
    save_lock(load_lock()["product"], {c: {"version": v, "hash": crate_hash(c)} for c, v in component_versions().items()})
    print(f"recorded current content as {load_lock()['product']}")


def cmd_release(a: argparse.Namespace) -> None:
    if out := problems():
        raise Fail("fix these first:\n" + "\n".join(out))
    level_of = {c: "patch" for c in changed()}
    for lvl in ("minor", "major"):
        for c in getattr(a, lvl) or []:
            if c not in component_versions():
                raise Fail(f"unknown component {c}")
            level_of[c] = lvl
    top = max((LEVELS.index(l) for l in level_of.values()), default=0)
    if LEVELS.index(a.level) < top:
        raise Fail(f"a component has a {LEVELS[top]} bump; the product bump must be at least {LEVELS[top]}")
    lock = load_lock()
    comps = {c: dict(lock["components"].get(c, {})) for c in crates()}
    for c, lvl in sorted(level_of.items()):
        nv = bump(component_versions()[c], lvl)
        write_component(c, nv)
        print(f"  {c} -> {nv} ({lvl})")
    new_product = bump(product_sources()["backend/Cargo.toml [workspace.package]"], a.level)
    write_product(new_product)
    for c, v in component_versions().items():
        comps[c] = {"version": v, "hash": crate_hash(c)}
    save_lock(new_product, comps)
    print(f"product -> {new_product}\nnext: run the tests, then tag v{new_product}")


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sp = p.add_subparsers(dest="cmd", required=True)
    sp.add_parser("show").set_defaults(f=cmd_show)
    c = sp.add_parser("check")
    c.add_argument("--release", action="store_true")
    c.add_argument("--tag")
    c.set_defaults(f=cmd_check)
    s = sp.add_parser("set")
    s.add_argument("name")
    s.add_argument("version")
    s.set_defaults(f=cmd_set)
    sp.add_parser("baseline").set_defaults(f=cmd_baseline)
    r = sp.add_parser("release")
    r.add_argument("level", choices=LEVELS)
    r.add_argument("--minor", nargs="*", metavar="CRATE")
    r.add_argument("--major", nargs="*", metavar="CRATE")
    r.set_defaults(f=cmd_release)
    a = p.parse_args()
    try:
        a.f(a)
    except Fail as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
