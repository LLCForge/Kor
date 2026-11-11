# Panel maps (spec §29.2)

One JSON file per hosting panel, provider client area or registrar. The Kor side panel uses them to
recognise the page the user is on and to offer contextual commands. Contributions are welcome: copy a
file, change `id`, `name`, `match` and `commands`, list it in `backend/crates/kor-web/src/site/panels.rs`
(`MAPS`) and add a detection case to its tests.

- `match`: `ports` (panel ports), `hosts` (host or parent domain), `paths` (URL path fragments),
  `markers` (lower-case words in the page title).
- `kind`: the `kor host` adapter for `api` commands (`cpanel`, `plesk`, …), or `null`.
- `commands[].action`: `api` (runs through the matching `kor host` profile: plan, then the user's
  approval in the side panel), `fill` (Kor fills form fields whose label, placeholder or name contains
  one of `labels`; password fields are never filled and the user presses submit), `explain` (known
  error messages and risky settings from `_common.json`).
