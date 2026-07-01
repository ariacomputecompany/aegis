# Aegis

Aegis is an agentic web browser.

It gives agents one persistent browser runtime, one local control plane, and one correct production path:

- run `aegis` for the local Linux dashboard
- start `aegis serve`
- control the browser over the local HTTP API
- run in `headless` or `headful` mode against the same session

## What It Is

Aegis is for agent workflows that need a real browser engine, not a fake DOM.

Core capabilities:

- real browser navigation
- live command execution against the page
- semantic matcher-based control targeting for reactive sites
- headless and headful control
- DOM snapshots
- ordered runtime events
- session import and export
- deterministic traces
- replayable browser runs

The browser engine is a CEF-backed native runtime. On Linux headful mode, the only supported UI is the bundled web dashboard backed by a local noVNC viewport.

## Production Model

There is one supported runtime model:

- one persistent `serve` process
- one local HTTP API
- one browser session per runtime

There is no production per-command relaunch path.

Local release rule:

- install one stable local release app at a platform-native path
- macOS: `~/Applications/Aegis.app`
- Linux: `~/.local/share/aegis/Aegis`
- use its bundled CLI as the canonical runtime entrypoint
- do not rebuild or reinstall during normal `aegis` usage
- use `./install.sh` as the canonical one-shot local install path

Runtime state rules:

- Chromium browser profiles are not a persistence API
- Chromium credential storage and autofill persistence are disabled in the production runtime
- persistent agent state lives under `~/.aegis` by default, or `$AEGIS_HOME` if set
- session persistence goes through `GET /session`, `POST /session`, `POST /session/save`, and `POST /session/load`
- `--profile <name>` selects `~/.aegis/profiles/<name>/session.json`
- `~/.aegis/settings/*.json` is the canonical home for concern-specific local settings
- `~/.aegis/settings/credentials.json` controls Aegis-owned login capture behavior
- `~/.aegis/secrets/profiles/<profile>/secrets.json` is the canonical home for Aegis-owned saved secrets
- saved browser credentials live under each profile's `secrets.credentials.entries`
- trace persistence goes through `POST /trace/enable`
- if `--start-url` is omitted, the runtime boots into a local no-network bootstrap page
- the canonical control style is semantic `match` targeting for `click` and `set_value`, not long-lived raw DOM ids
- Aegis does not use Chrome/Brave Safe Storage, browser login DBs, or external keychain-backed browser profile storage anywhere in the production path

## CLI Surface

The main binary is `aegis`.

Human-use shortcut:

- `aegis` with no arguments starts the headful Linux runtime and opens the local dashboard
- `aegis open` explicitly does the same
- `aegis ...` with arguments uses the installed bundled CLI
- macOS bundled CLI path: `~/Applications/Aegis.app/Contents/MacOS/aegis_cli`
- Linux bundled CLI path: `~/.local/share/aegis/Aegis/bin/aegis_cli`

Top-level commands:

- `open`
- `serve`
- `usage`
- `examples`
- `navigate`
- `search`
- `page inspect|text|markdown|actions|forms|headings|links|find|open-link`
- `config get`
- `config set`
- `config secrets-get`
- `config secrets-set`
- `config credentials-list`
- `config credentials-set`
- `config credentials-remove`
- `config credentials-clear`
- `trace inspect`
- `native status`
- `native doctor`
- `native configure`
- `native build`
- `native install`
- `native paths`

Global runtime flags:

- `--mode headless|headful`
- `--start-url <url>`
- `--host-lib <path>`
- `--profile <name>`
- `--server-addr <host:port>`

Built-in CLI guidance:

- `aegis --help` gives the high-level command map plus quick starts
- `aegis usage` prints the recommended production workflow
- `aegis examples` prints copy-pasteable commands for common tasks

## Install

One-shot local install:

```bash
./install.sh
```

What it does:

- builds the web dashboard
- builds the release binary
- installs the platform-native local app directory
- installs the bundled CLI into that app directory
- installs the canonical shell launcher at `~/.local/bin/aegis` or `~/bin/aegis`
- bootstraps and verifies the canonical `~/.aegis` state tree

The older helper path now delegates to the same installer:

```bash
./scripts/install_local_release.sh
```

Canonical verification uses Fozzy:

```bash
./scripts/run_fozzy_full.sh
```

## Config And Secrets

Inspect or set Aegis-owned config:

```bash
aegis config get agent
aegis config set agent --json '{"default_profile":"work"}'
aegis config get credentials
aegis config set credentials --json '{"auto_store":false}'
```

Inspect or set Aegis-owned per-profile secrets:

```bash
aegis config secrets-get --profile work
aegis config secrets-set --profile work --json '{"github":{"username":"saint","password":"..."},"api_keys":{"openai":"..."}}'
```

Manage Aegis-owned saved browser credentials:

```bash
aegis config credentials-list --profile work
aegis config credentials-set --profile work --json '{"origin":"https://github.com","username":"saint","password":"...","username_field":"login","password_field":"password","form_label":"Sign in"}'
aegis config credentials-remove --profile work --origin https://github.com --username saint
aegis config credentials-clear --profile work
```

Secrets rules:

- secrets live only in `~/.aegis/secrets/...`
- Aegis does not read or write Chrome/Brave cookies, login databases, or Safe Storage entries
- Aegis auto-stores credentials by default when it sees username/password entry followed by a submit-like click in the active profile
- users can disable that behavior in `~/.aegis/settings/credentials.json`
- users can inspect and clean up cached credentials through the CLI without touching browser-managed storage

## Start A Runtime

```bash
aegis --mode headful serve --addr 127.0.0.1:7878
```

Research through a running runtime:

```bash
aegis --server-addr 127.0.0.1:7878 search shopify app review
aegis --server-addr 127.0.0.1:7878 navigate https://shopify.dev/docs
aegis --server-addr 127.0.0.1:7878 page inspect
aegis --server-addr 127.0.0.1:7878 page text --scope main
aegis --server-addr 127.0.0.1:7878 page markdown --scope article
aegis --server-addr 127.0.0.1:7878 page actions
aegis --server-addr 127.0.0.1:7878 page forms
aegis --server-addr 127.0.0.1:7878 page find release an app version
aegis --server-addr 127.0.0.1:7878 page open-link release an app version
```

For live agent debugging:

- use `--mode headful` and open the dashboard at `http://127.0.0.1:7878/`

For unattended execution:

- use `--mode headless`

If `--start-url` is omitted, Aegis starts on the local bootstrap page and only pays network cost
when the agent explicitly navigates.

## HTTP API

Base address defaults to `http://127.0.0.1:7878`.

Core routes:

- `GET /healthz`
- `GET /runtime`
- `GET /telemetry`
- `GET /ui/bootstrap`
- `GET /ui/chrome/state`
- `GET /ui/vnc`
- `POST /search`
- `POST /navigate`
- `POST /execute`
- `GET /page`
- `GET /page/text`
- `GET /page/markdown`
- `GET /page/actions`
- `GET /page/forms`
- `GET /page/links`
- `GET /page/headings`
- `POST /page/find`
- `POST /page/open-link`
- `GET /dom`
- `GET /events`
- `GET /session`
- `POST /session`
- `POST /trace/enable`

`aegis serve` now fails fast if the browser command bridge cannot finish startup. A running
server implies the runtime has already completed an initial bridge roundtrip, so `/healthz`
should report `command_ready: true` and `bridge_healthy: true` as soon as the process reports
ready.

### `GET /runtime`

Returns:

- `host_library`
- `browser`
- `runtime`

The `runtime` object includes:

- `bootstrapped`
- `bootstrap_duration_ms`
- `dom_nodes`
- `latest_event_sequence`
- `current_url`
- `current_title`
- `document_ready_state`
- `last_live_state_refresh_at_ms`

The response also includes:

- `profile.profile`
- `profile.path`

### `GET /telemetry`

Return the full production telemetry snapshot for the live browser session.

This route is the canonical operator surface for:

- runtime health and owner-thread diagnostics
- recent operations and latency aggregates
- DOM and event telemetry, including native request method/status metadata, redirects, failures, response headers, duration, transferred bytes, cache/header coverage, scheme security, mixed-content detection, and aggregate breakdowns by domain, method, MIME type, status code, content encoding, and error signature
- page timing, paint, layout stability, responsiveness, resource, and JS heap telemetry sampled from the live page
- session inventory summaries for cookies, storage, and network overrides
- saved credential metadata and credential auto-store settings
- trace status and dashboard transport details

The telemetry route is designed to be consumed directly by the Linux dashboard now and by Quilt later.

### `POST /navigate`

Navigate the live browser session:

```bash
curl -X POST http://127.0.0.1:7878/navigate \
  -H 'content-type: application/json' \
  -d '{"url":"https://example.com"}'
```

### `POST /search`

Run a first-class browser search without building the engine URL yourself:

```bash
curl -X POST http://127.0.0.1:7878/search \
  -H 'content-type: application/json' \
  -d '{"query":"shopify app review","engine":"duckduckgo"}'
```

### `POST /execute`

Run commands against the current page:

```bash
curl -X POST http://127.0.0.1:7878/execute \
  -H 'content-type: application/json' \
  -d '{
    "commands": [
      {"type":"eval","code":"document.title"}
    ]
  }'
```

Supported command types:

- `eval`
- `click`
- `hover`
- `set_value`
- `press_key`
- `wait_for`
- `scroll`

`eval.code` should be a JavaScript expression such as `document.title`.
Aegis also normalizes a leading `return ...;` if you accidentally send one.

Execution model:

- `navigate` returns ordered navigation/events quickly and invalidates the cached DOM tree
- one `POST /execute` batch is dispatched as one runtime batch instead of one bridge roundtrip per command
- matcher-based commands resolve against the live renderer DOM at execution time, so semantic `match` targets stay valid across reactive page updates better than cached node ids
- `GET /dom` still gives the stable materialized snapshot for inspection and text search
- `hover` and matcher-based `press_key` resolve against the live DOM with strict action-aware ranking
- `wait_for` executes inside the runtime owner loop, polling live page metadata and optional DOM conditions until the condition is satisfied or times out
- `execute` can omit the snapshot for low-latency commands like `eval` and `scroll`
- incremental state flows through `GET /events`

### `GET /dom`

Return the current DOM snapshot:

```bash
curl http://127.0.0.1:7878/dom
```

### `GET /page*`

Use the page research routes when you want semantic page reads instead of a raw DOM snapshot:

```bash
curl http://127.0.0.1:7878/page
curl 'http://127.0.0.1:7878/page/text?scope=main'
curl 'http://127.0.0.1:7878/page/markdown?scope=article'
curl http://127.0.0.1:7878/page/actions
curl http://127.0.0.1:7878/page/forms
curl http://127.0.0.1:7878/page/links
curl http://127.0.0.1:7878/page/headings
```

These routes expose:

- focused content scopes such as `main`, `article`, `controls`, and `overlays`
- primary links and controls ranked for likely next actions
- form inventories for auth and submission workflows
- blocker signals such as overlays and likely auth walls
- direct `page find` and `page open-link` helpers for agent navigation loops

### `GET /events`

Read ordered runtime events:

```bash
curl 'http://127.0.0.1:7878/events?since=0'
```

Event stream semantics:

- events are ordered
- `sequence` is monotonically increasing
- `GET /events` drains pending native/browser events into the runtime stream before it responds
- `since=<n>` returns events with `sequence > n`

Runtime event types:

- `dom_mutation`
- `navigation`
- `network`
- `log`

`GET /doctor` and `GET /runtime` also expose the live page truth the runtime is tracking:

- `runtime.current_url`
- `runtime.current_title`
- `runtime.document_ready_state`

`command_ready` now indicates whether the runtime can accept commands, even while it is currently busy executing one.

### `GET/POST /session`

Import or export session state:

- cookies
- local storage
- session storage
- network overrides

### `POST /session/save`

Persist the current runtime session to the active profile file:

```bash
curl -X POST http://127.0.0.1:7878/session/save
```

### `POST /session/load`

Reload the active profile file into the live runtime:

```bash
curl -X POST http://127.0.0.1:7878/session/load
```

### `POST /trace/enable`

Enable deterministic trace capture:

```bash
curl -X POST http://127.0.0.1:7878/trace/enable \
  -H 'content-type: application/json' \
  -d '{"path":"traces/run.json"}'
```

Inspect later with:

```bash
aegis trace inspect traces/run.json
```

## Agent Loop

The canonical control loop is:

1. start `serve`
2. check `GET /runtime`
3. `POST /navigate`
4. inspect `GET /dom` or `GET /events`
5. `POST /execute`
6. continue from `GET /events?since=<latest_sequence>`
7. persist state with `GET /session` if needed
8. enable traces for important runs

## Startup Measurement

Measure cold-start and first-command latency with the real bundled runtime path:

```bash
python3 scripts/measure_startup.py --mode headful
python3 scripts/measure_startup.py --mode headful --samples 5
```

The report includes:

- `process_spawn_ms`
- `serve_ready_banner_ms`
- `runtime_poll_attempts`
- `runtime_ready_ms`
- `first_command_ms`
- `runtime_before`
- `first_execute`
- `runtime_after`

When `--samples` is greater than `1`, the harness prints median and max timings plus the full per-sample reports.

## Native Runtime

Aegis uses a local native host library:

- macOS packaged installs use `~/Applications/Aegis.app/Contents/Frameworks/libaegis_host.dylib`
- Linux packaged installs use `~/.local/share/aegis/Aegis/lib/libaegis_host.so`
- macOS workspace builds use `native/build/macos/Release/libaegis_host.dylib`
- Linux workspace builds use `native/build/linux/release/libaegis_host.so`

Native helper commands:

```bash
aegis native status
aegis native doctor
aegis native configure
aegis native build
aegis native install
aegis native paths
```

Install a stable local release app bundle:

```bash
./install.sh
```

Run the full local production-like verification flow:

```bash
./scripts/verify_local_release.sh
```

Run the Fozzy verification gate directly:

```bash
./scripts/run_fozzy_full.sh
```

Use `aegis native doctor` when you need one canonical preflight report for expected CEF paths,
canonical install paths, workspace build outputs, required tools, and configure/build/install
readiness.

That installs a stable local app, installs the canonical `aegis` launcher, and gives the runtime
a stable local app path. On macOS it also clears quarantine attributes and supports code signing.

The Fozzy gate is the canonical validation surface:

- deterministic verification starts with `fozzy doctor --deep ... --runs 5 --seed 424242 --strict`
- the canonical host-backed trace path is `fozzy run ... --det --strict-verify --record ...` followed by `fozzy trace verify`, `fozzy replay`, and `fozzy ci`
- `tests/aegis_core.fozzy.json` validates core Rust/test/clippy behavior
- `tests/aegis_host_backed.fozzy.json` validates the host-backed runtime flow
- `tests/aegis_native_doctor.fozzy.json` validates the shared native preflight contract
- `fozzy.toml` pins `.fozzy/` as the artifact root

For distribution-grade macOS installs, `aegis native install` and `./install.sh` also honor:

- `AEGIS_CODESIGN_IDENTITY="Developer ID Application: ..."` to sign the bundle with a real identity
- `AEGIS_CODESIGN_OPTIONS="runtime"` to opt into hardened runtime options during signing
- `AEGIS_CODESIGN_ENTITLEMENTS=/absolute/path/to/entitlements.plist` to attach app entitlements

On macOS, the installer signs nested helpers/frameworks explicitly and runs
`codesign --verify --strict` after installation. When a real signing identity is provided, it
also runs `spctl --assess` so the install fails immediately if Gatekeeper would reject the bundle.

The checked-in local entitlements template lives at `native/mac/aegis.entitlements`. The local
verification script exports that file automatically unless `AEGIS_CODESIGN_ENTITLEMENTS` is already
set.

## Local Signing Limits

Without a paid Apple Developer account, Aegis can still do a lot locally on macOS:

- build release binaries
- use one stable installed app path
- ad hoc sign the local app bundle
- clear quarantine attributes on that local bundle

What local-only setup cannot bypass:

- macOS Automation / Accessibility / similar privacy approvals
- the benefits of Developer ID signing and notarization

Those system approvals still require one user approval path in macOS.

## Dependencies

The native build expects the platform CEF SDK at:

- macOS: `third_party/cef/cef_binary_146.0.6+g68649e2+chromium-146.0.7680.154_macosarm64`
- Linux: `third_party/cef/cef_binary_146.0.6+g68649e2+chromium-146.0.7680.154_linux64`

That binary payload is intentionally not tracked in Git.

## Docs

The practical agent guide lives at:

- `docs/agent-control.md`
