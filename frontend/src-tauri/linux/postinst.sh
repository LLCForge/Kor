#!/bin/sh
# SPDX-License-Identifier: Apache-2.0 OR MIT
# deb/rpm post-install (spec §24.4, ADR 0064): `kor` is already /usr/bin/kor; register the Chrome
# native messaging host for every user. Never fails the package installation.
if [ -x /usr/bin/kor ]; then
  /usr/bin/kor install-hooks post-install --system || true
fi
exit 0
