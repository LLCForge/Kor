# kor-runtime

**What it does.** Session assembly shared by the CLI and the desktop app: trust, key handling, provider registry, privacy and offline gates, updates, analysis commands (audit, quality, deps), forges, triggers, offline bundles. Functions return localized messages instead of printing.

**What it does not do.** No terminal or window code: callers decide how to show results.

**Called by:** `kor-cli`, `kor-desktop`, `kor-proxy`, `kor-web`.

Module map: the `//!` comment at the top of `src/lib.rs`. Where it sits in the whole: [ARCHITECTURE.md](../../../ARCHITECTURE.md).

## Forge feature matrix (§28.7)

| | GitHub | GitLab | Gitea/Forgejo | Bitbucket Cloud | Bitbucket Server/DC | Azure DevOps |
| --- | --- | --- | --- | --- | --- | --- |
| PR create/list/diff/review/reply | yes | yes | yes | yes | yes | yes |
| `kor pr ci` checks | check runs + statuses | pipeline jobs | commit statuses | commit statuses | build-status API | commit statuses |
| `kor pr ci --fix` log tails | Actions job logs | job trace | link only | link only | link only | link only |
| `kor pr issue` | issues | issues | issues | issues | none (Jira) | work items |
| `kor pr release` | releases | releases | releases | no API | no API | no API |

Self-hosted: `[forge.hosts] "git.corp" = "gitlab"` (or `bitbucket-server`, `gitea`, `azure`…); `[forge] ca_cert = "corp-ca.pem"` (extra root certificates), `client_cert = "me.pem"` (certificate and key, for mTLS), `proxy = "http://proxy:3128"` (not used in privacy/offline mode). Bitbucket Server takes `BITBUCKET_SERVER_TOKEN` or `kor auth login bitbucket-server` (an HTTP access token, or `user:password`). Issue texts and CI logs go into tasks as quoted data, never as instructions.
