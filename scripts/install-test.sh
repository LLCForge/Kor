#!/bin/sh
# SPDX-License-Identifier: Apache-2.0 OR MIT
# Tests scripts/install.sh against a fake local release (no network): a good archive installs,
# a wrong SHA-256 and a required-but-uncheckable signature are refused. Run: sh scripts/install-test.sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT INT TERM

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) t=aarch64-apple-darwin ;;
  Darwin-*) t=x86_64-apple-darwin ;;
  Linux-aarch64) t=aarch64-unknown-linux-gnu ;;
  Linux-*) if ls /lib/ld-musl-* >/dev/null 2>&1; then t=x86_64-unknown-linux-musl; else t=x86_64-unknown-linux-gnu; fi ;;
  *) echo "install.sh is for macOS and Linux; running the test with a Linux name"; t=x86_64-unknown-linux-gnu ;;
esac

mkdir -p "$work/rel/kor-cli-$t" "$work/bin"
printf '#!/bin/sh\necho "kor 9.9.9"\n' >"$work/rel/kor-cli-$t/kor"
chmod 755 "$work/rel/kor-cli-$t/kor"
(cd "$work/rel" && tar -cJf "kor-cli-$t.tar.xz" "kor-cli-$t")
sum() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }
echo "$(sum "$work/rel/kor-cli-$t.tar.xz")  kor-cli-$t.tar.xz" >"$work/rel/kor-cli-$t.tar.xz.sha256"

# Git Bash turns /c/... into C:/...; curl wants file:///C:/...
url="file://$(cd "$work/rel" && pwd -W 2>/dev/null || pwd)"
case "$url" in file://[A-Za-z]:*) url="file:///${url#file://}" ;; esac
run() { KOR_TARGET="$t" KOR_DOWNLOAD_URL="$url" KOR_INSTALL_DIR="$work/bin" sh "$here/install.sh"; }

run 2>"$work/log" || { cat "$work/log"; echo "FAIL: good archive was refused"; exit 1; }
[ "$("$work/bin/kor")" = "kor 9.9.9" ] || { echo "FAIL: installed binary does not run"; exit 1; }
grep -q "SHA-256 ok" "$work/log" || { echo "FAIL: no checksum line"; exit 1; }

echo "0000000000000000000000000000000000000000000000000000000000000000  x" >"$work/rel/kor-cli-$t.tar.xz.sha256"
rm -f "$work/bin/kor"
if run 2>"$work/log"; then echo "FAIL: tampered archive was installed"; exit 1; fi
grep -q "SHA-256 mismatch" "$work/log" || { cat "$work/log"; echo "FAIL: wrong error"; exit 1; }
[ ! -e "$work/bin/kor" ] || { echo "FAIL: binary left behind"; exit 1; }

echo "$(sum "$work/rel/kor-cli-$t.tar.xz")  kor-cli-$t.tar.xz" >"$work/rel/kor-cli-$t.tar.xz.sha256"
if KOR_REQUIRE_SIGNATURE=1 run 2>"$work/log"; then echo "FAIL: unsigned install with KOR_REQUIRE_SIGNATURE=1"; exit 1; fi
grep -q "KOR_REQUIRE_SIGNATURE=1" "$work/log" || { cat "$work/log"; echo "FAIL: wrong error"; exit 1; }
echo "install.sh test ok ($t)"
