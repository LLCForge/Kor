#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Right-click menu check on a clean machine (spec §25.4, ADR 0052/0064). Standard library only.
Run by .github/workflows/clean-vm.yml around an installed package.

  installed --kor K --desktop D --save FILE   `kor integrate install --yes --extended`, then: every
                                              recorded entry exists and the menu commands name the
                                              installed kor / kor-desktop; "click": the desktop entry's
                                              command opens a project folder (the app stays up), the
                                              CLI entry's command starts `kor` (Windows; Linux runs the
                                              recorded script). Saves the entries to FILE.
  gone FILE [--strict]                        after uninstall: no saved entry is left (registry keys,
                                              files, merged lines). Without --strict leftovers are a
                                              warning (packages whose uninstall cannot run in the user's
                                              context: Linux system packages, a macOS app moved to the Trash).
  selftest                                    parsing and the leftover check against fake entries

Exit code: number of failed checks.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path

FAILED: list[str] = []


def step(name: str, ok: bool, detail: str = "", warn_only: bool = False) -> None:
    tag = "PASS" if ok else ("WARN" if warn_only else "FAIL")
    print(f"{tag} {name}{': ' + detail if detail else ''}", flush=True)
    if not ok and warn_only and os.environ.get("GITHUB_ACTIONS"):
        print(f"::warning::{name}: {detail}")
    if not ok and not warn_only:
        FAILED.append(name)


def reg_dump(key: str) -> str | None:
    """`reg query HKCU\\key /s` output, None when the key does not exist."""
    p = subprocess.run(["reg", "query", f"HKCU\\{key}", "/s"], capture_output=True, text=True, errors="replace")
    return p.stdout if p.returncode == 0 else None


def reg_commands(dump: str) -> list[str]:
    """Default values of every `...\\command` key in a `reg query /s` dump."""
    out, in_command = [], False
    for line in dump.splitlines():
        if line.startswith("HKEY_"):
            in_command = line.rstrip().lower().endswith("\\command")
            continue
        m = re.match(r"\s+\(Default\)\s+REG_(?:EXPAND_)?SZ\s+(.*)$", line)
        if in_command and m:
            out.append(m.group(1).strip())
    return out


def entry_target(line: str) -> tuple[str, str]:
    """(kind, what) of one record line: `reg <key>`, `file <path>`, `mime <path> <entry>`, …"""
    kind, _, rest = line.partition(" ")
    if kind == "mime":
        rest = rest.rsplit(" ", 1)[0]
    return kind, rest


def present(line: str, exes: list[str]) -> bool:
    kind, what = entry_target(line)
    if kind == "reg":
        return sys.platform == "win32" and reg_dump(what) is not None
    if kind in ("file", "dir"):
        return Path(what).exists()
    if kind in ("thunar", "tasks", "mime"):
        # Merged into a shared file: Kor's lines name the installed programs (or the kor:// handler).
        try:
            text = Path(what).read_text(encoding="utf-8", errors="replace")
        except OSError:
            return False
        marks = [e for e in exes if e] + (["x-scheme-handler/kor"] if kind == "mime" else [])
        return any(m in text for m in marks)
    # `sparse menu`, `finder-sync`: system registrations, checked through `kor integrate status`.
    return False


def processes() -> list[str]:
    """Command lines of running processes."""
    if sys.platform == "win32":
        p = subprocess.run(
            ["powershell", "-NoProfile", "-Command",
             "Get-CimInstance Win32_Process | ForEach-Object { $_.CommandLine } | ConvertTo-Json -Compress"],
            capture_output=True, text=True, errors="replace")
        try:
            v = json.loads(p.stdout or "[]")
        except json.JSONDecodeError:
            return []
        return [c for c in (v if isinstance(v, list) else [v]) if c]
    if Path("/proc").is_dir():
        out = []
        for d in Path("/proc").iterdir():
            try:
                out.append((d / "cmdline").read_bytes().replace(b"\0", b" ").decode(errors="replace"))
            except OSError:
                pass
        return out
    return subprocess.run(["ps", "-axo", "command"], capture_output=True, text=True).stdout.splitlines()


def wait_for(pred, secs: float) -> bool:
    end = time.time() + secs
    while time.time() < end:
        if pred():
            return True
        time.sleep(1)
    return False


def kill_matching(needle: str) -> None:
    if sys.platform == "win32":
        subprocess.run(["powershell", "-NoProfile", "-Command",
                        f"Get-CimInstance Win32_Process | Where-Object {{ $_.CommandLine -like '*{needle}*' }} | "
                        "ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }"])
    else:
        subprocess.run(["pkill", "-f", needle])


def click(name: str, command: list[str] | str, project: Path, expect: str, shell: bool = False) -> None:
    """Run a menu entry's command on `project`; a process whose command line has `expect` and the
    project must come up (the desktop with that folder, or `kor` for the CLI entry)."""
    before = set(processes())
    subprocess.Popen(command, shell=shell, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    proj = str(project)

    def up() -> bool:
        return any(expect in c and proj.lower() in c.lower() for c in set(processes()) - before)

    ok = wait_for(up, 30)
    step(name, ok, f"a {expect} process for {proj}" + ("" if ok else " did not start"))
    kill_matching(proj)


def installed(kor: str, desktop: str, save: Path) -> None:
    exes = [str(Path(kor).resolve()), str(Path(desktop).resolve()) if desktop else ""]
    p = subprocess.run([kor, "--lang", "en", "integrate", "install", "--yes", "--extended"], capture_output=True, text=True, errors="replace")
    print(p.stdout[-3000:], p.stderr[-3000:])
    step("menu-install", p.returncode == 0, f"exit {p.returncode}")
    s = json.loads(subprocess.run([kor, "--lang", "en", "integrate", "status", "--json"], capture_output=True, text=True, errors="replace").stdout or "{}")
    entries: list[str] = s.get("entries", [])
    save.write_text(json.dumps({"entries": entries, "exes": exes}, indent=2), encoding="utf-8")
    step("menu-recorded", bool(s.get("installed")) and bool(entries), f"{len(entries)} entries")
    checkable = [e for e in entries if entry_target(e)[0] in ("reg", "file", "dir", "thunar", "tasks", "mime")]
    missing = [e for e in checkable if not present(e, exes)]
    step("menu-entries-present", not missing, f"missing: {missing}" if missing else f"{len(checkable)} checked")
    if sys.platform == "win32" and s.get("modern_menu") is not None:
        step("menu-win11", bool(s["modern_menu"]), "Windows 11 top-level menu package registered")
    if sys.platform == "darwin":
        step("menu-finder-sync", s.get("finder_sync") is not False, f"finder_sync={s.get('finder_sync')}", warn_only=True)

    # Both items (§25.1) name the installed programs.
    texts: list[str] = []
    commands: list[str] = []
    for e in entries:
        kind, what = entry_target(e)
        if kind == "reg" and sys.platform == "win32":
            d = reg_dump(what) or ""
            texts.append(d)
            commands += reg_commands(d)
        elif kind in ("file", "thunar", "tasks") and Path(what).is_file():
            texts.append(Path(what).read_text(encoding="utf-8", errors="replace"))
        elif kind == "dir" and Path(what).is_dir():
            texts += [f.read_text(encoding="utf-8", errors="replace") for f in Path(what).rglob("*") if f.is_file() and f.suffix in ("", ".wflow", ".plist", ".sh")]
    blob = "\n".join(texts).lower()
    desktop_ok = not desktop or exes[1].lower() in blob or "--open" in blob
    step("menu-desktop-item", desktop_ok and "--open" in blob, "the desktop entry opens the folder with kor-desktop --open")
    step("menu-cli-item", "--terminal" in blob or "open-terminal" in blob, "the CLI entry opens kor in a terminal")

    # "Click": run what the menu runs, on a fresh project folder.
    with tempfile.TemporaryDirectory(prefix="kor-menu-") as d:
        project = Path(d, "menu project")
        project.mkdir()
        if sys.platform == "win32":
            for c in commands:
                cmd = c.replace("%V", str(project)).replace("%1", str(project))
                if "--open" in c:
                    click("menu-click-desktop", cmd, project, "kor-desktop")
                    break
            for c in commands:
                if "--terminal" in c or "open-terminal" in c:
                    cmd = c.replace("%V", str(project)).replace("%1", str(project))
                    click("menu-click-cli", cmd, project, "kor")
                    break
        elif sys.platform.startswith("linux"):
            scripts = [Path(entry_target(e)[1]) for e in entries if entry_target(e)[0] == "file"]
            scripts = [p for p in scripts if os.access(p, os.X_OK)]
            opened = [p for p in scripts if "--open" in p.read_text(errors="replace")]
            if opened:
                click("menu-click-desktop", [str(opened[0]), str(project)], project, "kor-desktop")
            else:
                step("menu-click-desktop", False, "no executable desktop entry (Nautilus/Caja script) to run", warn_only=True)
        else:
            print("SKIP menu-click: macOS Quick Actions run inside Automator/Finder Sync, not from a script")


def gone(saved: Path, strict: bool) -> None:
    data = json.loads(saved.read_text(encoding="utf-8"))
    left = [e for e in data["entries"] if present(e, data["exes"])]
    step("menu-removed", not left, f"left behind: {left}" if left else f"{len(data['entries'])} entries removed", warn_only=not strict)


def selftest() -> None:
    dump = (
        "HKEY_CURRENT_USER\\Software\\Classes\\Directory\\shell\\Kor.desktop\n"
        "    (Default)    REG_SZ    Kor ile aç\n\n"
        "HKEY_CURRENT_USER\\Software\\Classes\\Directory\\shell\\Kor.desktop\\command\n"
        '    (Default)    REG_SZ    "C:\\P\\Kor\\kor-desktop.exe" --open "%V"\n'
    )
    assert reg_commands(dump) == ['"C:\\P\\Kor\\kor-desktop.exe" --open "%V"'], reg_commands(dump)
    assert entry_target("mime /h/.config/mimeapps.list x-scheme-handler/kor=kor.desktop") == ("mime", "/h/.config/mimeapps.list")
    assert entry_target("reg Software\\Classes\\kor") == ("reg", "Software\\Classes\\kor")
    with tempfile.TemporaryDirectory() as d:
        f = Path(d, "Kor CLI ile aç")
        f.write_text("#!/bin/sh\nexec /usr/bin/kor integrate open-terminal \"$@\"\n")
        tasks = Path(d, "tasks.json")
        tasks.write_text('{"tasks": [{"command": "/usr/bin/kor-desktop"}]}')
        exes = ["/usr/bin/kor", "/usr/bin/kor-desktop"]
        assert present(f"file {f}", exes) and present(f"tasks {tasks}", exes)
        saved = Path(d, "saved.json")
        saved.write_text(json.dumps({"entries": [f"file {f}", f"tasks {tasks}"], "exes": exes}))
        FAILED.clear()
        gone(saved, strict=True)
        assert FAILED == ["menu-removed"]
        f.unlink()
        tasks.write_text('{"tasks": []}')
        FAILED.clear()
        gone(saved, strict=True)
        assert FAILED == [], FAILED
    print("menu_check selftest ok")


def main(argv: list[str]) -> int:
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    if not argv or argv[0] in ("-h", "--help"):
        print(__doc__)
        return 0
    cmd, rest = argv[0], argv[1:]
    if cmd == "selftest":
        selftest()
        return 0
    if cmd == "installed":
        o = {"--kor": None, "--desktop": "", "--save": None}
        it = iter(rest)
        for a in it:
            if a not in o:
                print(f"unknown option {a}", file=sys.stderr)
                return 2
            o[a] = next(it, None)
        if not o["--kor"] or not o["--save"]:
            print(__doc__, file=sys.stderr)
            return 2
        installed(o["--kor"], o["--desktop"] or "", Path(o["--save"]))
    elif cmd == "gone" and rest:
        gone(Path(rest[0]), "--strict" in rest[1:])
    else:
        print(__doc__, file=sys.stderr)
        return 2
    print(f"menu check: {len(FAILED)} failed {FAILED}")
    return len(FAILED)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
