; SPDX-License-Identifier: Apache-2.0 OR MIT
; NSIS installer hooks (spec §24.4, ADR 0064). The per-user setup puts the bundled `kor` on the user
; PATH and registers the Chrome native messaging host; uninstall undoes exactly that. Both steps
; live in the CLI (`kor install-hooks`, kor_platform::cli_path / native_host) so the MSI, deb and
; rpm packages do the same thing. A failing step never fails the installation.

!macro NSIS_HOOK_POSTINSTALL
  nsExec::ExecToLog '"$INSTDIR\kor.exe" install-hooks post-install --path'
  Pop $0
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::ExecToLog '"$INSTDIR\kor.exe" install-hooks pre-uninstall --path'
  Pop $0
!macroend
