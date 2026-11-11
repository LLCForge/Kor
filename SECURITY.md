# Security policy

## Reporting a vulnerability

Report vulnerabilities privately through GitHub Security Advisories ("Report a vulnerability" on the
repository's Security tab). Do not open public issues for them.

- Acknowledgement within **72 hours**.
- A fix or mitigation plan within **14 days** for high and critical issues; a coordinated disclosure date is
  agreed with the reporter (90 days at most).
- We request a CVE for confirmed vulnerabilities and credit reporters unless they decline.

In scope: secret leakage, sandbox escape, prompt-injection paths that run tools without consent, unexpected
network traffic (privacy mode in particular), installer or updater tampering, and supply-chain issues in
released artifacts.

## Supported versions

Only the latest release on the `stable` channel receives fixes. `beta` and `nightly` get fixes by moving forward.

## Verifying what you downloaded

- Every release artifact has a Sigstore build provenance attestation: `gh attestation verify <file> -R LLCForge/Kor`.
- The raw `kor` binaries carry a minisign signature (`kor-<target>.minisig`); `kor update` and `scripts/install.sh`
  check it against the public key published with the release.
- Desktop installers are signed per OS (Authenticode, Developer ID + notarization, detached GPG `.asc` on Linux);
  unsigned builds are named `_UNSIGNED` and never published by the release workflow without an explicit override.
- Builds are reproducible from the tagged source with `just release` (pinned toolchain, `--locked` dependencies).
