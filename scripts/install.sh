#!/bin/sh
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# Kor CLI installer (spec §24.4): `curl -fsSL https://get.kor.dev | sh`.
# Read it before running it; it is short on purpose. It
#   1. picks the release archive for this OS and CPU (glibc or musl on Linux),
#   2. downloads it and its .sha256 from GitHub Releases and refuses a mismatch,
#   3. verifies the binary's minisign signature when `minisign` is installed (the public key is
#      written into this file when a release is published; KOR_REQUIRE_SIGNATURE=1 makes it mandatory),
#   4. copies `kor` into ~/.local/bin (KOR_INSTALL_DIR to change). It never edits shell profiles
#      and never asks for sudo; it prints the PATH line to add if the folder is not on PATH.
#
# Environment: KOR_VERSION (default: latest, e.g. 0.9.0), KOR_INSTALL_DIR, KOR_REQUIRE_SIGNATURE,
# KOR_TARGET (Rust target triple, e.g. x86_64-unknown-linux-musl, instead of detecting it),
# KOR_DOWNLOAD_URL (a mirror or a local folder: file:///path; used by the tests).
set -eu

REPO="LLCForge/Kor"
BIN="kor"
# Replaced by the release workflow with vars.KOR_UPDATE_PUBKEY; empty = no minisign check possible.
PUBKEY=""

say() { printf '%s\n' "$*" >&2; }
die() { say "error: $*"; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "needs '$1'"; }

need uname
need tar
need mkdir

target() {
  os=$(uname -s)
  cpu=$(uname -m)
  case "$cpu" in
    x86_64 | amd64) cpu=x86_64 ;;
    arm64 | aarch64) cpu=aarch64 ;;
    *) die "no prebuilt kor for CPU $cpu; build from source: https://github.com/$REPO" ;;
  esac
  case "$os" in
    Darwin) echo "$cpu-apple-darwin" ;;
    Linux)
      # musl (Alpine) gets the static build; it is x86_64 only.
      if ls /lib/ld-musl-* >/dev/null 2>&1 || (ldd --version 2>&1 | grep -qi musl); then
        [ "$cpu" = x86_64 ] || die "no prebuilt musl kor for $cpu"
        echo "x86_64-unknown-linux-musl"
      else
        echo "$cpu-unknown-linux-gnu"
      fi
      ;;
    *) die "this script is for macOS and Linux; on Windows use winget install $REPO or the .msi" ;;
  esac
}

fetch() {
  if command -v curl >/dev/null 2>&1; then
    curl --proto '=https,file' --tlsv1.2 -fsSL "$1" -o "$2"
  elif command -v wget >/dev/null 2>&1; then
    wget -q "$1" -O "$2"
  else
    die "needs curl or wget"
  fi
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d' ' -f1
  else
    die "needs sha256sum or shasum to verify the download"
  fi
}

main() {
  t=${KOR_TARGET:-$(target)}
  version=${KOR_VERSION:-latest}
  if [ -n "${KOR_DOWNLOAD_URL:-}" ]; then
    base=$KOR_DOWNLOAD_URL
  elif [ "$version" = latest ]; then
    base="https://github.com/$REPO/releases/latest/download"
  else
    base="https://github.com/$REPO/releases/download/v${version#v}"
  fi
  archive="kor-cli-$t.tar.xz"
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT INT TERM

  say "Downloading $archive ($version)"
  fetch "$base/$archive" "$tmp/$archive"
  fetch "$base/$archive.sha256" "$tmp/sum"
  want=$(cut -d' ' -f1 <"$tmp/sum" | tr 'A-F' 'a-f')
  got=$(sha256_of "$tmp/$archive")
  [ -n "$want" ] && [ "$want" = "$got" ] || die "SHA-256 mismatch for $archive (expected $want, got $got)"
  say "SHA-256 ok"

  mkdir -p "$tmp/x"
  tar -xJf "$tmp/$archive" -C "$tmp/x"
  exe=$(find "$tmp/x" -type f -name "$BIN" | head -n 1)
  [ -n "$exe" ] || die "$archive has no $BIN binary"

  if [ -n "$PUBKEY" ] && command -v minisign >/dev/null 2>&1; then
    fetch "$base/kor-$t.minisig" "$tmp/sig"
    minisign -Vm "$exe" -x "$tmp/sig" -P "$PUBKEY" >/dev/null || die "minisign signature does not match"
    say "Signature ok"
  elif [ "${KOR_REQUIRE_SIGNATURE:-0}" = 1 ]; then
    die "KOR_REQUIRE_SIGNATURE=1 but the signature cannot be checked (install minisign; release key: ${PUBKEY:-none})"
  else
    say "Signature not checked (install minisign to check it); the SHA-256 matched the release"
  fi

  dir=${KOR_INSTALL_DIR:-$HOME/.local/bin}
  mkdir -p "$dir"
  cp "$exe" "$dir/$BIN.new"
  chmod 755 "$dir/$BIN.new"
  mv -f "$dir/$BIN.new" "$dir/$BIN"
  say "Installed $dir/$BIN"
  case ":$PATH:" in
    *":$dir:"*) "$dir/$BIN" --version || true ;;
    *) say "Add it to PATH: export PATH=\"$dir:\$PATH\"" ;;
  esac
}

main "$@"
