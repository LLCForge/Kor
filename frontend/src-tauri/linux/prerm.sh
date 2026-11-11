#!/bin/sh
# SPDX-License-Identifier: Apache-2.0 OR MIT
# deb/rpm pre-remove (spec §24.4, ADR 0064): undo `post-install`, only on a real removal (rpm runs
# the old package's script after the new one's post-install on upgrade: $1 = 1; deb passes
# "upgrade"). Never fails the removal.
case "$1" in
  0|remove|purge) ;;
  *) exit 0 ;;
esac
if [ -x /usr/bin/kor ]; then
  /usr/bin/kor install-hooks pre-uninstall --system || true
fi
exit 0
