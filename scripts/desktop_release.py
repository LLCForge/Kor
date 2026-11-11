#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Desktop release helpers for .github/workflows/release.yml (ADR 0032). Standard library only.

  gate <dir> [--allow-unsigned]        refuse to publish when an installer is not OS-signed
  latest-json <dir> <tag> <repo>       Tauri updater manifest (latest.json) from the *.sig files
  manifests <dir> <tag> <repo>         winget / Homebrew cask / Flatpak / AUR manifests (for manual PRs)
  webview <mode> <out.json> [dir]      Tauri config overlay for the WebView2 install mode (§24.1):
                                       bootstrapper (default, embedded), offline, fixed <runtime dir>
  selftest                             run against generated fake artifacts

<dir> holds the desktop installers, their *.sig (Tauri updater) / *.asc (GPG) files and one
signing-*.json report per build job: {"name": ..., "os_signed": bool, "updater_signed": bool}.
"""

from __future__ import annotations

import hashlib
import json
import sys
import tempfile
import tomllib
from datetime import datetime, timezone
from pathlib import Path

# branding.toml is the single source (ADR 0064); `just rebrand` rewrites it.
BRAND = tomllib.loads((Path(__file__).resolve().parent.parent / "branding.toml").read_text(encoding="utf-8"))
PRODUCT = BRAND["product"]["name"]
SLUG = BRAND["product"]["slug"]
IDENTIFIER = BRAND["product"]["identifier"]
PUBLISHER = BRAND["product"]["publisher"]
DESCRIPTION = BRAND["product"]["description"]
WINGET_ID = f"{PUBLISHER}.{PRODUCT}"


def reports(d: Path) -> list[dict]:
    return [json.loads(p.read_text(encoding="utf-8")) for p in sorted(d.glob("signing-*.json"))]


def gate(d: Path, allow_unsigned: bool) -> int:
    rs = reports(d)
    if not rs:
        print("no desktop signing reports: desktop builds are missing", file=sys.stderr)
        return 1
    unsigned = [r["name"] for r in rs if not r.get("os_signed")]
    if unsigned and not allow_unsigned:
        print(
            f"refusing to publish unsigned desktop installers: {', '.join(unsigned)}. "
            "Add the signing secrets (repository settings) or re-run the workflow on the tag with allow-unsigned.",
            file=sys.stderr,
        )
        return 1
    for name in unsigned:
        print(f"warning: publishing UNSIGNED desktop installers for {name} (allow-unsigned)")
    return 0


def one(d: Path, pattern: str) -> Path | None:
    found = sorted(p for p in d.glob(pattern) if not p.name.endswith((".sig", ".asc")))
    return found[0] if found else None


def latest_json(d: Path, tag: str, repo: str) -> dict:
    base = f"https://github.com/{repo}/releases/download/{tag}"

    def entry(pattern: str) -> dict | None:
        f = one(d, pattern)
        sig = f and f.with_name(f.name + ".sig")
        if not f or not sig.is_file():
            return None
        return {"signature": sig.read_text(encoding="utf-8").strip(), "url": f"{base}/{f.name}"}

    nsis, msi = entry("*-setup*.exe"), entry("*.msi")
    mac, appimage = entry("*.app.tar.gz"), entry("*.AppImage")
    platforms: dict[str, dict] = {}
    for key, value in [
        ("windows-x86_64", nsis or msi),
        ("windows-x86_64-nsis", nsis),
        ("windows-x86_64-msi", msi),
        # one universal .app serves both architectures
        ("darwin-aarch64", mac),
        ("darwin-x86_64", mac),
        ("darwin-aarch64-app", mac),
        ("darwin-x86_64-app", mac),
        ("linux-x86_64", appimage),
        ("linux-x86_64-appimage", appimage),
    ]:
        if value:
            platforms[key] = value
    return {
        "version": tag.removeprefix("v"),
        "notes": f"https://github.com/{repo}/releases/tag/{tag}",
        "pub_date": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "platforms": platforms,
    }


def sha256(p: Path) -> str:
    h = hashlib.sha256()
    with p.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def manifests(d: Path, tag: str, repo: str) -> list[Path]:
    """Templates for registry submissions; submitting them is a manual step."""
    version = tag.removeprefix("v")
    base = f"https://github.com/{repo}/releases/download/{tag}"
    out: list[Path] = []

    def write(name: str, text: str) -> None:
        p = d / name
        p.write_text(text, encoding="utf-8", newline="\n")
        out.append(p)

    setup = one(d, "*-setup*.exe")
    if setup:
        common = f"PackageIdentifier: {WINGET_ID}\nPackageVersion: {version}\n"
        installers = (
            "Installers:\n  - Architecture: x64\n    InstallerType: nullsoft\n    Scope: user\n"
            + f"    InstallerUrl: {base}/{setup.name}\n    InstallerSha256: {sha256(setup).upper()}\n"
        )
        msi = one(d, "*.msi")
        if msi:
            installers += (
                "  - Architecture: x64\n    InstallerType: wix\n    Scope: machine\n"
                + f"    InstallerUrl: {base}/{msi.name}\n    InstallerSha256: {sha256(msi).upper()}\n"
            )
        write(
            f"winget-{WINGET_ID}.installer.yaml",
            common + "UpgradeBehavior: install\n" + installers + "ManifestType: installer\nManifestVersion: 1.6.0\n",
        )
        write(
            f"winget-{WINGET_ID}.locale.en-US.yaml",
            common
            + f"PackageLocale: en-US\nPublisher: {PUBLISHER}\nPackageName: {PRODUCT}\n"
            + f"PackageUrl: https://github.com/{repo}\nLicense: Apache-2.0 OR MIT\n"
            + f"ShortDescription: {DESCRIPTION}\n"
            + "ManifestType: defaultLocale\nManifestVersion: 1.6.0\n",
        )
        write(
            f"winget-{WINGET_ID}.yaml",
            common + "DefaultLocale: en-US\nManifestType: version\nManifestVersion: 1.6.0\n",
        )

    dmg = one(d, "*.dmg")
    if dmg:
        app_cli = f"#{{appdir}}/{PRODUCT}.app/Contents/MacOS/{SLUG}"
        write(
            f"homebrew-{SLUG}.rb",
            f'cask "{SLUG}" do\n  version "{version}"\n  sha256 "{sha256(dmg)}"\n\n'
            f'  url "https://github.com/{repo}/releases/download/v#{{version}}/{dmg.name.replace(version, "#{version}")}"\n'
            f'  name "{PRODUCT}"\n  desc "{DESCRIPTION}"\n'
            f'  homepage "https://github.com/{repo}"\n\n'
            f'  depends_on macos: ">= :big_sur"\n\n  app "{PRODUCT}.app"\n'
            f'  binary "{app_cli}"\n\n'
            # §24.4 (ADR 0064): the Chrome native messaging host, like the Windows/Linux installers.
            f'  postflight do\n    system_command "{app_cli}", args: ["install-hooks", "post-install"]\n  end\n\n'
            f'  uninstall_preflight do\n    system_command "{app_cli}", args: ["install-hooks", "pre-uninstall"]\n  end\n\n'
            f'  zap trash: [\n    "~/Library/Application Support/{SLUG}",\n    "~/Library/Caches/{SLUG}",\n'
            f'    "~/Library/WebKit/{IDENTIFIER}",\n  ]\nend\n',
        )

    deb = one(d, "*.deb")
    if deb:
        write(
            f"flatpak-{IDENTIFIER}.yml",
            f"# Flatpak template (Flathub prefers building from source).\n"
            f"app-id: {IDENTIFIER}\nruntime: org.gnome.Platform\nruntime-version: '47'\n"
            f"sdk: org.gnome.Sdk\ncommand: {SLUG}-desktop\nfinish-args:\n"
            "  - --share=ipc\n  - --share=network\n  - --socket=wayland\n  - --socket=fallback-x11\n"
            f"  - --device=dri\n  - --filesystem=home\n  - --filesystem=~/{PRODUCT}:create\n"
            "  - --talk-name=org.freedesktop.Notifications\n"
            # §27.10 (ADR 0052): portals (screen, input, files), tray, and `flatpak-spawn --host`
            # for the host-side integrations (gnome-extensions, systemctl --user, xdg-mime).
            "  - --talk-name=org.freedesktop.portal.Desktop\n  - --talk-name=org.freedesktop.Flatpak\n"
            "  - --talk-name=org.kde.StatusNotifierWatcher\n"
            f"modules:\n  - name: {SLUG}\n    buildsystem: simple\n    build-commands:\n"
            f"      - ar x {SLUG}.deb && tar -xf data.tar.gz\n"
            "      - cp -a usr/bin usr/lib usr/share /app/ 2>/dev/null || cp -a usr/. /app/\n"
            f"    sources:\n      - type: file\n        url: {base}/{deb.name}\n"
            f"        sha256: {sha256(deb)}\n        dest-filename: {SLUG}.deb\n",
        )
        # AUR `<slug>-bin` (§24.4): the same files as the .deb; SRCINFO goes to the AUR repo as .SRCINFO.
        fields = {
            "pkgname": f"{SLUG}-bin",
            "pkgver": version.replace("-", "_"),
            "pkgrel": "1",
            "pkgdesc": DESCRIPTION,
            "arch": ["x86_64"],
            "url": f"https://github.com/{repo}",
            "license": ["Apache-2.0", "MIT"],
            "depends": ["webkit2gtk-4.1", "gtk3", "libayatana-appindicator"],
            "provides": [SLUG],
            "conflicts": [SLUG],
            "source": [f"{SLUG}-{version}.deb::{base}/{deb.name}"],
            "sha256sums": [sha256(deb)],
        }

        def arr(v: list[str]) -> str:
            return "(" + " ".join(f"'{x}'" for x in v) + ")"

        write(
            "PKGBUILD",
            f"# Maintainer: {PUBLISHER}\n"
            + "".join(f"{k}={arr(v) if isinstance(v, list) else repr(v)}\n" for k, v in fields.items())
            + "\npackage() {\n  bsdtar -xf data.tar.* -C \"$pkgdir\"\n"
            + f"  install -d \"$pkgdir/usr/share/licenses/{SLUG}-bin\"\n"
            + f"  printf '%s\\n' 'Apache-2.0 OR MIT: {fields['url']}' > \"$pkgdir/usr/share/licenses/{SLUG}-bin/LICENSE\"\n"
            + "}\n",
        )
        lines = [f"pkgbase = {fields['pkgname']}"]
        for k in ["pkgdesc", "pkgver", "pkgrel", "url", "arch", "license", "depends", "provides", "conflicts", "source", "sha256sums"]:
            for v in fields[k] if isinstance(fields[k], list) else [fields[k]]:
                lines.append(f"\t{k} = {v}")
        write("SRCINFO","\n".join(lines) + f"\n\npkgname = {fields['pkgname']}\n")
    return out


def webview(mode: str, out: Path, fixed_dir: str | None = None) -> dict:
    """Tauri config overlay for the WebView2 install mode (spec §24.1). The default build embeds the
    Evergreen bootstrapper (tauri.conf.json); `offline` embeds the full runtime installer; `fixed`
    ships a Fixed Version runtime folder (downloaded by the user from Microsoft) inside the app."""
    modes = {
        "bootstrapper": {"type": "embedBootstrapper", "silent": True},
        "offline": {"type": "offlineInstaller", "silent": True},
    }
    if mode == "fixed":
        if not fixed_dir or not (Path(fixed_dir) / "msedgewebview2.exe").is_file():
            raise SystemExit("fixed: give the extracted Fixed Version runtime folder (it contains msedgewebview2.exe)")
        install = {"type": "fixedRuntime", "path": str(Path(fixed_dir).resolve())}
    elif mode in modes:
        install = modes[mode]
    else:
        raise SystemExit(f"unknown WebView2 mode {mode!r}: bootstrapper, offline or fixed <dir>")
    cfg = {"bundle": {"windows": {"webviewInstallMode": install}}}
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(cfg, indent=2) + "\n", encoding="utf-8")
    return cfg


def selftest() -> None:
    with tempfile.TemporaryDirectory() as t:
        d = Path(t)
        for name, body in {
            "Kor_0.4.0_x64-setup.exe": b"nsis",
            "Kor_0.4.0_x64-setup.exe.sig": b"SIG-NSIS\n",
            "Kor_0.4.0_x64_en-US.msi": b"msi",
            "Kor_0.4.0_universal.dmg": b"dmg",
            "Kor_0.4.0_universal.app.tar.gz": b"app",
            "Kor_0.4.0_universal.app.tar.gz.sig": b"SIG-MAC",
            "Kor_0.4.0_amd64.deb": b"deb",
            "Kor_0.4.0_amd64.AppImage": b"img",
            "Kor_0.4.0_amd64.AppImage.asc": b"gpg",
        }.items():
            (d / name).write_bytes(body)
        (d / "signing-windows.json").write_text('{"name": "windows", "os_signed": true}')
        (d / "signing-linux.json").write_text('{"name": "linux", "os_signed": false}')
        assert gate(d, False) == 1 and gate(d, True) == 0
        j = latest_json(d, "v0.4.0", "LLCForge/Kor")
        p = j["platforms"]
        assert j["version"] == "0.4.0"
        assert p["windows-x86_64"]["signature"] == "SIG-NSIS"
        assert p["windows-x86_64"]["url"].endswith("/v0.4.0/Kor_0.4.0_x64-setup.exe")
        assert "windows-x86_64-msi" not in p, "no .sig, no entry"
        assert p["darwin-aarch64"] == p["darwin-x86_64"]
        assert "linux-x86_64" not in p, "a GPG .asc is not an updater signature"
        names = {f.name for f in manifests(d, "v0.4.0", "LLCForge/Kor")}
        assert names == {
            f"winget-{WINGET_ID}.installer.yaml",
            f"winget-{WINGET_ID}.locale.en-US.yaml",
            f"winget-{WINGET_ID}.yaml",
            f"homebrew-{SLUG}.rb",
            f"flatpak-{IDENTIFIER}.yml",
            "PKGBUILD",
            "SRCINFO",
        }, names
        cask = (d / f"homebrew-{SLUG}.rb").read_text()
        assert "Kor_#{version}_universal.dmg" in cask and hashlib.sha256(b"dmg").hexdigest() in cask
        assert '"install-hooks", "post-install"' in cask and "uninstall_preflight" in cask
        winget = (d / f"winget-{WINGET_ID}.installer.yaml").read_text()
        assert hashlib.sha256(b"nsis").hexdigest().upper() in winget and "InstallerType: wix" in winget
        flatpak = (d / f"flatpak-{IDENTIFIER}.yml").read_text()
        assert "--talk-name=org.freedesktop.Flatpak" in flatpak and f"--filesystem=~/{PRODUCT}:create" in flatpak
        pkgbuild = (d / "PKGBUILD").read_text()
        assert f"pkgname='{SLUG}-bin'" in pkgbuild and hashlib.sha256(b"deb").hexdigest() in pkgbuild
        assert "pkgver='0.4.0'" in pkgbuild and "package() {" in pkgbuild
        srcinfo = (d / "SRCINFO").read_text()
        assert srcinfo.startswith(f"pkgbase = {SLUG}-bin\n") and "\tdepends = webkit2gtk-4.1\n" in srcinfo
        out = d / "wv.json"
        assert webview("offline", out)["bundle"]["windows"]["webviewInstallMode"]["type"] == "offlineInstaller"
        assert json.loads(out.read_text())["bundle"]["windows"]["webviewInstallMode"]["silent"] is True
        rt = d / "rt"
        rt.mkdir()
        (rt / "msedgewebview2.exe").write_bytes(b"")
        assert webview("fixed", out, str(rt))["bundle"]["windows"]["webviewInstallMode"]["type"] == "fixedRuntime"
        for bad in [("fixed", None), ("nope", None)]:
            try:
                webview(bad[0], out, bad[1])
            except SystemExit:
                pass
            else:
                raise AssertionError(bad)
    print("desktop_release selftest ok")


def main(argv: list[str]) -> int:
    if not argv:
        print(__doc__)
        return 2
    cmd, args = argv[0], argv[1:]
    if cmd == "selftest":
        selftest()
        return 0
    if cmd == "gate":
        return gate(Path(args[0]), "--allow-unsigned" in args[1:])
    if cmd == "latest-json":
        d, tag, repo = Path(args[0]), args[1], args[2]
        data = latest_json(d, tag, repo)
        if not data["platforms"]:
            print("no updater signatures (TAURI_SIGNING_PRIVATE_KEY unset): no latest.json")
            return 0
        (d / "latest.json").write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
        print(f"latest.json: {', '.join(sorted(data['platforms']))}")
        return 0
    if cmd == "manifests":
        for p in manifests(Path(args[0]), args[1], args[2]):
            print(p.name)
        return 0
    if cmd == "webview":
        webview(args[0], Path(args[1]), args[2] if len(args) > 2 else None)
        return 0
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
