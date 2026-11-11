#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Build assets/builtin-skills.zip: third-party agent skills shipped with Kor and installed into
Kor's data folder on first use (kor_runtime::builtin_skills). Standard library only.

  python scripts/pack-skills.py <clones>

<clones> holds shallow clones of the sources below at the pinned commits:

  git clone --depth 1 https://github.com/addyosmani/agent-skills
  git clone --depth 1 https://github.com/sindresorhus/awesome
  git clone --depth 1 https://github.com/blader/humanizer
  git clone --depth 1 https://github.com/nateherkai/scroll-craft
  git clone --depth 1 --filter=blob:none --sparse https://github.com/heygen-com/hyperframes
  (cd hyperframes && git sparse-checkout set --skip-checks skills LICENSE)

A Kor skill may only read files inside its own folder, so two adaptations are made and listed
in SOURCES.md: agent-skills' shared `references/` is copied into each skill that cites it, and
HyperFrames' `../hyperframes/references/…` links point at the `hyperframes` skill's files
through the `skill` tool. Nothing else is changed.
"""

from __future__ import annotations

import re
import subprocess
import sys
import zipfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
OUT = REPO / "assets" / "builtin-skills.zip"

SOURCES = {
    "agent-skills": ("https://github.com/addyosmani/agent-skills", "301b02b", "MIT", "LICENSE"),
    "awesome": ("https://github.com/sindresorhus/awesome", "bc98e51", "CC0-1.0", "license"),
    "humanizer": ("https://github.com/blader/humanizer", "b48c832", "MIT", "LICENSE"),
    "scroll-craft": ("https://github.com/nateherkai/scroll-craft", "75d81f7", "MIT", "LICENSE"),
    "hyperframes": ("https://github.com/heygen-com/hyperframes", "c218b37", "Apache-2.0", "LICENSE"),
}

# Files a skill never needs at run time (and that would only grow the download).
SKIP = re.compile(r"(^|/)(\.git|node_modules|__pycache__|\.DS_Store)(/|$)")
MAX_FILE = 4 * 1024 * 1024

AWESOME_SKILL = """---
name: awesome-lists
description: |
  Find curated "awesome" lists (sindresorhus/awesome) for a topic: libraries, tools, learning
  resources, frameworks. Use when the user asks for the best tools, libraries or resources for
  something, wants alternatives, or needs a starting point in an unfamiliar area.
license: CC0-1.0
---

# Awesome lists

`awesome.md` in this skill is the index of curated lists from
https://github.com/sindresorhus/awesome (CC0-1.0), grouped by area (Platforms, Programming
Languages, Front-End Development, Back-End Development, Computer Science, Big Data, Theory,
Books, Editors, Gaming, Development Environment, Entertainment, Databases, Media, Learn,
Security, Content Management Systems, Hardware, Business, Work, Networking, Decentralized
Systems, Health and Social Science, Events, Testing, Miscellaneous, Related).

1. Read `awesome.md` with the `skill` tool (`file: awesome.md`) and find the entries for the
   user's topic; search it for the topic's words first, it is long.
2. Give the matching lists with their links. When the user wants concrete picks, open the
   chosen list (WebFetch on its GitHub URL) and recommend from it, saying which list it came
   from.
3. The index is a snapshot; say so when recency matters, and check the live list.
"""

HF_PRIVACY = (
    "> Kor: HyperFrames' usage beacons and automatic feedback reports are removed. Do not run "
    "`npx hyperframes events` or `npx hyperframes feedback` unless the user asks for it.\n\n"
)

HF_NOTE = (
    "> Kor: paths written `hyperframes:<file>` are files of the `hyperframes` skill. Load them "
    "with the `skill` tool (`name: hyperframes`, `file: <file>`).\n\n"
)


def sha(src: Path) -> str:
    return subprocess.run(["git", "-C", str(src), "rev-parse", "--short=7", "HEAD"], capture_output=True, text=True, check=True).stdout.strip()


def files(d: Path):
    for p in sorted(d.rglob("*")):
        rel = p.relative_to(d).as_posix()
        if p.is_file() and not SKIP.search(rel) and p.stat().st_size <= MAX_FILE:
            yield p, rel


def front_name(text: str) -> str | None:
    m = re.search(r"^name:\s*['\"]?([\w.-]+)", text, re.M)
    return m.group(1) if m else None


def main(argv: list[str]) -> int:
    if len(argv) != 1:
        print(__doc__)
        return 2
    clones = Path(argv[0])
    for name, (_, pin, _, _) in SOURCES.items():
        got = sha(clones / name)
        if got != pin:
            print(f"{name}: clone is at {got}, the pinned commit is {pin}", file=sys.stderr)
            return 1

    entries: dict[str, bytes] = {}
    made: list[tuple[str, str]] = []  # (skill folder, source)

    def add_skill(folder: str, src_dir: Path, source: str, rewrite=None, extra=()):
        lic = clones / source / SOURCES[source][3]
        for p, rel in files(src_dir):
            data = p.read_bytes()
            if rel == "SKILL.md" and rewrite:
                data = rewrite(data.decode("utf-8")).encode("utf-8")
            entries[f"skills/{folder}/{rel}"] = data
        for rel, data in extra:
            entries[f"skills/{folder}/{rel}"] = data
        entries[f"skills/{folder}/LICENSE.upstream"] = lic.read_bytes()
        made.append((folder, source))

    # agent-skills: shared references copied in where cited.
    refs = clones / "agent-skills" / "references"
    ref_files = [(f"references/{p.name}", p.read_bytes()) for p in sorted(refs.glob("*.md"))]
    for d in sorted((clones / "agent-skills" / "skills").iterdir()):
        if not (d / "SKILL.md").is_file():
            continue
        text = (d / "SKILL.md").read_text(encoding="utf-8")
        cites = "../../references/" in text
        add_skill(
            d.name,
            d,
            "agent-skills",
            rewrite=(lambda t: t.replace("../../references/", "references/")) if cites else None,
            extra=ref_files if cites else (),
        )

    # humanizer: the repository root is the skill.
    hz = clones / "humanizer"
    entries.update({f"skills/humanizer/{rel}": p.read_bytes() for p, rel in files(hz) if rel == "SKILL.md" or rel.startswith("agents/")})
    entries["skills/humanizer/LICENSE.upstream"] = (hz / "LICENSE").read_bytes()
    made.append(("humanizer", "humanizer"))

    add_skill("scroll-craft", clones / "scroll-craft" / "plugins" / "nateherk-design" / "skills" / "scroll-craft", "scroll-craft")

    # HyperFrames: cross-skill links go through the `skill` tool.
    def hf_rewrite(t: str) -> str:
        # Usage beacons and automatic feedback reports would send data to HeyGen on every use
        # without the user asking; Kor drops them and says so at the top of the skill.
        kept = []
        for line in t.split("\n"):
            s = line.strip()
            if s.startswith(("**Usage beacon:**", "Usage beacon:")):
                continue
            if s.startswith("After a verified render") and "feedback --rating" in s:
                continue
            kept.append(line)
        t = "\n".join(kept)
        note = HF_PRIVACY
        if "../hyperframes/" in t:
            t = t.replace("../hyperframes/", "hyperframes:")
            note += HF_NOTE
        end = t.find("\n---", 3) + 4 if t.startswith("---") else 0
        return t[:end] + "\n\n" + note + t[end:].lstrip("\n")

    for d in sorted((clones / "hyperframes" / "skills").iterdir()):
        if (d / "SKILL.md").is_file():
            add_skill(d.name, d, "hyperframes", rewrite=hf_rewrite)

    entries["skills/awesome-lists/SKILL.md"] = AWESOME_SKILL.encode("utf-8")
    entries["skills/awesome-lists/awesome.md"] = (clones / "awesome" / "readme.md").read_bytes()
    entries["skills/awesome-lists/LICENSE.upstream"] = (clones / "awesome" / "license").read_bytes()
    made.append(("awesome-lists", "awesome"))

    # Names must be unique across sources (the loader keeps the first).
    names: dict[str, str] = {}
    for folder, source in made:
        n = front_name(entries[f"skills/{folder}/SKILL.md"].decode("utf-8")) or folder
        if n in names:
            print(f"skill name `{n}` from {source} clashes with {names[n]}", file=sys.stderr)
            return 1
        names[n] = source

    lines = [
        "# Built-in skills",
        "",
        "Shipped with Kor and installed into Kor's data folder on first use. Each skill keeps its",
        "upstream licence in `LICENSE.upstream`.",
        "",
        "| Source | Commit | Licence | Skills |",
        "| --- | --- | --- | --- |",
    ]
    for source, (url, pin, spdx, _) in SOURCES.items():
        skills = ", ".join(f for f, s in made if s == source)
        lines.append(f"| {url} | {pin} | {spdx} | {skills} |")
    lines += [
        "",
        "Changes made by Kor: agent-skills' shared `references/` folder is copied into each skill",
        "that cites `../../references/` (and the path shortened to `references/`); HyperFrames",
        "links `../hyperframes/…` are written `hyperframes:…` with a note to load them through the",
        "`skill` tool; HyperFrames' \"Usage beacon\" lines and its automatic post-render feedback",
        "instruction are removed from its SKILL.md files, with a note not to send either unless",
        "the user asks; `awesome-lists/SKILL.md` is written by Kor around the awesome index. All",
        "other files are unchanged.",
        "",
    ]
    entries["SOURCES.md"] = "\n".join(lines).encode("utf-8")

    OUT.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(OUT, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for name in sorted(entries):
            info = zipfile.ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0))  # reproducible
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o644 << 16
            z.writestr(info, entries[name])
    print(f"{OUT.relative_to(REPO)}: {len(made)} skills, {len(entries)} files, {OUT.stat().st_size // 1024} KiB")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
