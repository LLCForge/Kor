# Kor team server (`kor server`)

The team features of spec §11.1 on a machine you run: shared rules, templates, hooks and tools,
a shared graph and memory, a task queue with signed callbacks, a team vault, share links for
customer reports, a hash-chained audit log, single sign-on and a relay for peers and the browser
extension. It is the same `kor` binary; the code is `backend/crates/kor-server`.

## Set up

On the server machine:

```bash
kor server init --domain kor.example.com --admin ali@example.com --team acme
kor server add-member veli@example.com --role member     # admin | member (dev) | viewer
kor server start --background                            # or run in the foreground
kor server status
```

`init` writes `<kor home>/server/` (or `--dir`, `KOR_SERVER_DIR`):

| File | What |
| --- | --- |
| `server.toml` | settings (below) |
| `ca.pem`, `ca.key` | the team's certificate authority (P-256), key owner-only |
| `server.pem`, `server.key` | the server certificate for the domain, `localhost` and `127.0.0.1` |
| `vault.key` | the vault's 256-bit key (or `KOR_SERVER_VAULT_KEY`, 64 hex characters) |
| `server.sqlite` | the embedded database (unless `db` is Postgres) |
| `members/<email>.json` | each member's bundle: CA, client certificate, private key, member token |

A bundle holds a private key: hand it over privately. On the member's machine:

```bash
kor server join ali_at_example.com.json   # CA, certificate, key and token go to the OS keychain
kor server login                          # optional: sign in through the team's SSO
kor server status
```

The desktop does the same under Settings → Team (`team.connect` with the bundle path,
`team.login`). After joining, `kor tool share/pull`, team memory sync and the peer relay use the
client certificate automatically.

## `server.toml`

```toml
team = "acme"
domain = "kor.example.com"
listen = "0.0.0.0:8443"
db = "sqlite:server.sqlite"          # or "postgres://kor@db/kor?sslmode=require"
require_client_cert = true           # mTLS on every member route
session_hours = 12                   # SSO session token lifetime
member_days = 365                    # member certificate and member token lifetime
allow_private_callbacks = false      # task callbacks only to public https addresses
max_body = 8388608

[[oidc]]                             # Keycloak / Authentik / Google: ID tokens
name = "keycloak"
kind = "oidc"
issuer = "https://id.example.com/realms/acme"
client_id = "kor"

[[oidc]]                             # GitHub: access token, verified primary e-mail
name = "github"
kind = "github"
client_id = "Iv1.0123456789abcdef"
org = "acme"                         # optional: active members of this org only
```

Postgres: the connection uses TLS through rustls, checked against the OS trust store or a PEM in
`KOR_SERVER_DB_CA`; `sslmode=disable` is refused unless the database is on the same machine. The
SQL is the same for both engines (bound `$n` parameters only).

Docker Compose: run `kor server init` once into a volume, then
`command: ["kor", "server", "start", "--dir", "/data"]` with that volume at `/data` and port 8443
published; put Postgres next to it and set `db` accordingly.

## Identity and roles

A request proves who it is with the team client certificate (mTLS, checked against the team CA
and then against the server's list, so removing a member revokes their certificate at once)
and/or a bearer token (a member token from `add-member`, or a session token from SSO). When both
are present they must name the same member. Unknown e-mails are refused: SSO only signs in people
an admin added.

| Role | May |
| --- | --- |
| viewer | read shared documents, the queue, team memory and tools |
| member | also write memory, graph and reports, queue and work tasks, open vault secrets, share reports, use the relay |
| admin | also write rules, templates, hooks and tools (they run on every member's machine), the vault, read the audit log |

SSO: `kor server login` runs the OAuth device flow against the provider the server names
(`GET /api/v1/auth/providers`) and trades the result at `POST /api/v1/auth/token`. ID tokens are
checked for signature (RS256/384/512, ES256/384 from the issuer's JWKS; `none` and HMAC are
refused), exact issuer, audience (and `azp` with several audiences), `exp`/`nbf` with 60 s of skew,
and a verified e-mail. GitHub tokens are checked against `/user/emails` (verified primary e-mail)
and, with `org`, `/user/memberships/orgs/<org>`.

## API

| Route | Role |
| --- | --- |
| `GET /healthz` | none |
| `GET /api/v1/auth/providers`, `POST /api/v1/auth/token` | none (certificate still required with `require_client_cert`) |
| `GET /api/v1/me`, `GET /api/v1/members` | viewer |
| `GET /api/v1/docs/<ns>`, `GET /api/v1/docs/<ns>/<key>[?version=n]` | viewer |
| `PUT /api/v1/docs/<ns>/<key>` `{ body, version? }` | admin for `rules`, `templates`, `hooks`, `tools`; member for `memory`, `graph`, `reports` |
| `PUT/GET /team_memory?repo=`, `PUT/GET /team_tools` | the paths `kor` already uses |
| `GET /api/v1/vault`, `GET /api/v1/vault/<NAME>` | member (each opening is audited) |
| `PUT/DELETE /api/v1/vault/<NAME>` | admin |
| `POST /api/v1/shares` `{ key, hours }` → `/share/<token>` | member; the link is read-only, expiring, served as plain text |
| `POST /api/v1/queue` `{ prompt, repo?, callback? }` → `{ id, secret }` | member |
| `POST /api/v1/queue/claim`, `POST /api/v1/queue/<id>/finish` `{ ok, result }` | member (the worker) |
| `GET /api/v1/queue[?state=]`, `GET /api/v1/queue/<id>` | viewer |
| `GET /api/v1/audit?after=&limit=` | admin, with the chain check |
| `GET /api/v1/relay/listen/<name>`, `GET /api/v1/relay/dial/<name>` (WebSocket) | member |

Every document write keeps the previous versions (`doc_history`); a write with a stale `version`
gets 409 instead of overwriting someone else's change. Templates (§15.11) live in `templates`.

Task callbacks: when the worker finishes, the server POSTs `{ id, state, repo, result, finishedAt }`
to the callback with `x-kor-signature: sha256=<HMAC-SHA256 of the body with the task's secret>`,
up to three tries. The host is resolved once and pinned; private, loopback and link-local
addresses are refused unless `allow_private_callbacks`.

Vault (§6.6): values are sealed with ChaCha20-Poly1305 under the vault key, with the secret's name
as associated data; the database never holds plaintext.

Audit: every change, vault opening, share-link opening, SSO sign-in (and refusal) and relay
connection is appended to a SHA-256 hash chain; `kor server status` and `GET /api/v1/audit` verify
it.

Responses carry HSTS, `nosniff`, `X-Frame-Options: DENY`, `default-src 'none'` and
`Cache-Control: no-store`; requests are limited per address (token bucket), bodies to `max_body`,
connections to 512; internal errors are logged on the server and answered as `internal error`.

## Relay (§31.1, §18.4)

When `[team] server` is set and the machine joined the team, `kor peer serve` also parks a
WebSocket on the server (`relay/listen/peer:<its key>`), and `kor peer` tries the relay after the
direct, Tailscale and SSH routes. The Noise session runs inside, so the server forwards only
ciphertext; a name belongs to the first member who listens on it.

The browser extension on another computer reaches this machine's Kor the same way: in the side
panel's setup, "Kor on another machine" takes the server address, this machine's name
(`kor peer list`) and a member token, and connects to `relay/dial/ext:<name>`; `kor peer serve`
forwards it to the local Kor bridge, where pairing happens as usual.

## Not done here

- The task queue is an API: a member machine claims and finishes tasks. A built-in worker loop
  that runs claimed tasks with the agent (and triggers on the server) is not wired yet.
- The KaaS HTTP API (§11.2) and WASM plugins are separate items; they can build on the queue.
- The desktop member list reads `kor server status`; there is no member management UI beyond
  `server.addMember`.
- Postgres is compile-tested and shares every statement with the SQLite tests, but was not run
  against a live Postgres here.
