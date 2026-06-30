# Aegis Agent Control Guide

This is the practical control surface for agents.

The production rule is simple:

- run `aegis` with no arguments for the local human browser app
- run one persistent `serve` process
- control that runtime over the HTTP API
- use `headless` or `headful` as modes of that same runtime

CLI guidance:

- `aegis --help` gives the high-level command map and quick starts
- `aegis usage` prints the recommended production workflow
- `aegis examples` prints copy-pasteable commands for common tasks

## Mental Model

Aegis exposes one browser runtime behind one control plane.

The agent loop is:

1. start the runtime
2. navigate
3. inspect DOM or events
4. execute commands
5. persist or restore session state
6. optionally enable trace recording for replay

The runtime exposes one production control plane:

- the local HTTP API backed by a persistent `serve` process

## Runtime Modes

- `headless`: automation without a visible window
- `headful`: automation with a visible browser window

Global flags:

- `--mode headless|headful`
- `--start-url <url>`
- `--host-lib <path>` overrides the resolved native host library; `serve` defaults to the workspace release runtime and refreshes it when sources are newer
- `--profile <name>`
- `--server-addr <host:port>` targets an already-running `aegis serve` process for CLI research commands

Production state model:

- Chromium browser profiles are ephemeral and not a persistence API
- Chromium credential storage and autofill are enabled by default in the runtime
- Aegis-owned state lives under `~/.aegis` by default, or `$AEGIS_HOME` if set
- session continuity goes through `GET /session`, `POST /session`, `POST /session/save`, and `POST /session/load`
- the active profile persists to `~/.aegis/profiles/<profile>/session.json`
- concern-specific local settings belong in `~/.aegis/settings/*.json`
- `~/.aegis/settings/credentials.json` controls Aegis-owned login capture
- Aegis-owned secrets belong in `~/.aegis/secrets/profiles/<profile>/secrets.json`
- saved browser credentials live under each profile's `secrets.credentials.entries`
- traces go through `POST /trace/enable`
- if `--start-url` is omitted, the runtime boots into a local no-network bootstrap page
- Aegis-owned credential capture is stored under `~/.aegis`; browser-managed credential storage and autofill use Chromium's runtime profile behavior

For local production-like use, the canonical path is one installed release app at a
platform-native location:

- macOS app path: `~/Applications/Aegis.app`
- macOS bundled CLI path: `~/Applications/Aegis.app/Contents/MacOS/aegis_cli`
- Linux app path: `~/.local/share/aegis/Aegis`
- Linux bundled CLI path: `~/.local/share/aegis/Aegis/bin/aegis_cli`
- installed shell launcher: `~/.local/bin/aegis` or `~/bin/aegis`

For production-signing workflows on macOS, the installer honors `AEGIS_CODESIGN_IDENTITY`,
`AEGIS_CODESIGN_OPTIONS`, and `AEGIS_CODESIGN_ENTITLEMENTS`. It signs nested
helpers/frameworks explicitly, verifies the installed bundle with `codesign --verify --strict`,
and runs `spctl --assess` when a real signing identity is configured.
For the strongest local verification workflow, run `./scripts/verify_local_release.sh`. It
installs the canonical app, runs platform-appropriate verification, and then runs the host-backed
smoke test.
Fozzy is the canonical verification driver:

- `fozzy.toml` sets `.fozzy/` as the runtime artifact root
- `tests/aegis_core.fozzy.json` covers the core deterministic gate
- `tests/aegis_host_backed.fozzy.json` covers the host-backed runtime path
- `tests/aegis_native_doctor.fozzy.json` covers the shared native preflight contract
- start deterministic verification with `fozzy doctor --deep --scenario <scenario> --runs 5 --seed 424242 --strict`
- record at least one real host-backed trace, then run `fozzy trace verify`, `fozzy replay`, and `fozzy ci` against it
- `./scripts/run_fozzy_full.sh` is the full local Fozzy gate
Use `aegis native doctor` whenever you need the canonical install paths, workspace artifact paths,
missing-tool report, or configure/build/install readiness from one source of truth.
Normal `aegis` usage should not rebuild or reinstall anything.

## Fast Start

Inspect native paths:

```bash
aegis native paths
aegis native doctor
```

Install or refresh the stable local release app:

```bash
./install.sh
```

Inspect or set Aegis-owned config:

```bash
aegis config get agent
aegis config set agent --json '{"default_profile":"work"}'
aegis config get credentials
aegis config set credentials --json '{"auto_store":false}'
```

Inspect or set Aegis-owned secrets:

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
- Aegis never reads Chrome/Brave Safe Storage or browser login databases
- Aegis auto-stores credentials by default when it sees username/password entry followed by a submit-like click in the active profile
- users can disable that behavior in `~/.aegis/settings/credentials.json`
- users can inspect or clean up cached credentials entirely through the CLI

Start the API server:

```bash
aegis --mode headful serve --addr 127.0.0.1:7878
```

Research through a running runtime:

```bash
aegis --server-addr 127.0.0.1:7878 search shopify app review
aegis --server-addr 127.0.0.1:7878 navigate https://shopify.dev/docs
aegis --server-addr 127.0.0.1:7878 page inspect
aegis --server-addr 127.0.0.1:7878 page text
aegis --server-addr 127.0.0.1:7878 page markdown
aegis --server-addr 127.0.0.1:7878 page find redirect to your app's ui
aegis --server-addr 127.0.0.1:7878 page links
aegis --server-addr 127.0.0.1:7878 page open-link release an app version
```

Measure cold-start and first-command latency:

```bash
python3 scripts/measure_startup.py --mode headful
python3 scripts/measure_startup.py --mode headful --samples 5
```

The startup harness now measures the local bootstrap page by default so cold-start timings are not
inflated by external network fetches.

It also reports:

- `process_spawn_ms`
- `serve_ready_banner_ms`
- `runtime_poll_attempts`
- `runtime_ready_ms`
- `first_command_ms`

With `--samples > 1` it also prints median and max timings plus the full report for each sample.

## Session Shape

`SessionState` shape:

```json
{
  "cookies": [
    {
      "name": "sid",
      "value": "abc",
      "domain": ".example.com",
      "path": "/",
      "expires_unix": 1735689600,
      "secure": true,
      "http_only": true
    }
  ],
  "local_storage": {
    "theme": "dark"
  },
  "session_storage": {
    "flow": "checkout"
  },
  "network_overrides": [
    {
      "header": "x-agent-mode",
      "value": "test"
    }
  ]
}
```

Validation rules:

- cookie names must not be empty
- cookie domains must not be empty
- network override headers must not be empty

Event stream semantics:

- events are ordered
- each event has a monotonically increasing `sequence`
- `--since <n>` returns events with `sequence > n`

Runtime event types:

- `dom_mutation`
- `navigation`
- `network`
- `log`

## Traces

Inspect a recorded trace:

```bash
aegis trace inspect traces/run.json
```

Use trace recording for:

- deterministic regression capture
- replayable agent runs
- debugging browser/runtime mismatches

## HTTP API

Base address defaults to `http://127.0.0.1:7878`.

### Discovery

```bash
curl http://127.0.0.1:7878/
curl http://127.0.0.1:7878/manifest
```

These endpoints return a stable JSON route manifest plus supported command types, named schemas,
and concrete JSON request examples so agents can discover the control plane without probing for
undocumented routes.

The manifest now also advertises named multi-context routes. Root routes target the default
context, while `/contexts/:context_id/...` routes provide explicit isolation for multi-user and
multi-profile workflows behind one server process.

### Contexts

```bash
curl http://127.0.0.1:7878/contexts
curl -X POST http://127.0.0.1:7878/contexts \
  -H 'content-type: application/json' \
  -d '{"id":"guest","profile":"guest","mode":"headless"}'
curl http://127.0.0.1:7878/contexts/guest/runtime
curl -X POST http://127.0.0.1:7878/contexts/guest/navigate \
  -H 'content-type: application/json' \
  -d '{"url":"https://example.com"}'
```

Use contexts when you need host and guest sessions, buyer and seller, admin and user, or any
other independent browser identities at the same time.

### Health

```bash
curl http://127.0.0.1:7878/healthz
curl http://127.0.0.1:7878/readyz
```

After `aegis serve` reports ready, health should reflect a verified operational runtime rather
than a merely bound control plane. Production startup should not claim readiness until the
browser, renderer context, and runtime API are all live.

Concretely, readiness requires:

- `runtime.bootstrapped = true`
- `runtime.host.browser_available = true`
- `runtime.host.renderer_ready = true`
- `runtime.host.runtime_ready = true`

`renderer_ready` means the main-frame renderer context exists. `runtime_ready` is stricter: it
means Aegis has verified the live `window.__aegis` automation API, including the page research
helpers, and can dispatch real DOM and
command work through it.

### Runtime Info

```bash
curl http://127.0.0.1:7878/runtime
```

Returns:

- `host_library`
- `browser`
- `runtime`
- `startup`
- `profile`

The embedded host state is the concrete readiness contract:

- `browser.renderer_ready`
- `browser.runtime_ready`
- `browser.browser_available`
- `browser.browser_closed`
- `browser.load_in_progress`
- `runtime.media`

### Inject Session

```bash
curl -X POST http://127.0.0.1:7878/session \
  -H 'content-type: application/json' \
  -d @session.json
```

### Snapshot Session

```bash
curl http://127.0.0.1:7878/session
```

### Save Active Profile

```bash
curl -X POST http://127.0.0.1:7878/session/save
```

### Load Active Profile

```bash
curl -X POST http://127.0.0.1:7878/session/load
```

### Navigate

```bash
curl -X POST http://127.0.0.1:7878/navigate \
  -H 'content-type: application/json' \
  -d '{"url":"https://example.com"}'
```

### Search

```bash
curl -X POST http://127.0.0.1:7878/search \
  -H 'content-type: application/json' \
  -d '{"query":"shopify app review","engine":"duckduckgo"}'
```

### Execute

```bash
curl -X POST http://127.0.0.1:7878/execute \
  -H 'content-type: application/json' \
  -d '{
    "commands": [
      {"type":"eval","code":"document.title"}
    ]
  }'
```

Canonical targeting rule:

- prefer semantic `match` targets for `click` and `set_value`
- prefer `name`, `role`, and `control_type` over raw text when a docs UI has repeated labels
- use raw node `id` only for short-lived follow-up actions against a freshly materialized DOM
- on reactive pages, node ids are not a stable long-term contract

Matcher example:

```bash
curl -X POST http://127.0.0.1:7878/execute \
  -H 'content-type: application/json' \
  -d '{
    "commands": [
      {
        "type": "set_value",
        "match": {
          "control_type": "searchbox",
          "name": "Search with DuckDuckGo",
          "actionable": true
        },
        "value": "browser automation"
      },
      {
        "type": "click",
        "match": {
          "control_type": "submit",
          "name": "Search",
          "actionable": true
        }
      },
      {
        "type": "press_key",
        "key": "Enter"
      },
      {
        "type": "wait_for",
        "url_contains": "search",
        "text": "browser automation",
        "ready_state": "complete",
        "timeout_ms": 5000
      }
    ]
  }'
```

Drag example:

```bash
curl -X POST http://127.0.0.1:7878/execute \
  -H 'content-type: application/json' \
  -d '{
    "commands": [
      {
        "type": "drag",
        "match": {
          "role": "slider",
          "name": "Timeline",
          "actionable": true
        },
        "delta_x": 240,
        "handle": "end",
        "steps": 8
      },
      {
        "type": "geometry",
        "match": {
          "role": "slider",
          "name": "Timeline",
          "actionable": true
        }
      }
    ]
  }'
```

Supported matcher fields:

- `role`
- `name`
- `label`
- `control_type`
- `tag`
- `text`
- `placeholder`
- `href_contains`
- `actionable`
- `disabled`
- `exact`

Additional command types:

- `hover`
- `press_key`
- `wait_for`
- `scroll`
- `drag`
- `geometry`

Notes:
- `navigate` returns quickly with ordered navigation/events and invalidates the cached DOM tree
- `GET /dom` or a DOM-targeting command such as `click` / `set_value` repopulates the DOM snapshot on demand
- one `execute` request is sent through the runtime as one command batch, which preserves command ordering without paying one bridge roundtrip per command
- semantic `match` targets are resolved against the live renderer DOM at execution time, so they are the preferred production contract on reactive pages
- `click`, `hover`, and `set_value` use strict action-aware target resolution and return richer match diagnostics
- `press_key` can target the currently focused element or an explicit nested `target`
- `wait_for` can poll `url_contains`, `title_contains`, `text`, `ready_state`, an optional `target`, a CSS `selector`, scroll position/change, media readiness, known duration, and animation idle time
- `drag` supports drag-by-delta and drag-to-point requests, plus optional `handle` hints such as `start`, `end`, `left`, and `right`
- `delta_x` / `delta_y` are relative to the matched target anchor, while `to_x` / `to_y` are absolute viewport coordinates
- native `input[type=range]` targets update their value and emit `input` / `change` events during drag
- `geometry` returns a first-class element geometry snapshot without forcing custom `eval`
- `execute` may return `"snapshot": null` for low-latency commands such as `eval` and `scroll`
- agents should treat the event stream as the incremental source of truth between full snapshots
- the most reliable loop on live sites is `navigate -> /dom -> execute(match...)`

### Page Snapshot

```bash
curl http://127.0.0.1:7878/page
curl http://127.0.0.1:7878/page/text
curl http://127.0.0.1:7878/page/markdown
curl http://127.0.0.1:7878/page/links
curl http://127.0.0.1:7878/page/headings
```

The page research routes are the first-class alternative to dumping `/dom` when you need:

- normalized visible page text
- headings
- links
- canonical URL
- likely-404 diagnostics and a suggested recovery search query

### Find On Page

```bash
curl -X POST http://127.0.0.1:7878/page/find \
  -H 'content-type: application/json' \
  -d '{"text":"Submit an app version for review","exact":false}'
```

### Open Link By Text

```bash
curl -X POST http://127.0.0.1:7878/page/open-link \
  -H 'content-type: application/json' \
  -d '{"text":"Release an app version","exact":false}'
```

### Snapshot DOM

```bash
curl http://127.0.0.1:7878/dom
```

### Events

```bash
curl 'http://127.0.0.1:7878/events?since=0'

curl -N 'http://127.0.0.1:7878/events/live?since=0&poll_ms=100'
```

Notes:
- `GET /events` drains pending native/browser events into the runtime stream before it responds

### Enable Trace

```bash
curl -X POST http://127.0.0.1:7878/trace/enable \
  -H 'content-type: application/json' \
  -d '{"path":"traces/run.json"}'
```

## Recommended Agent Pattern

For robust control, use this sequence:

1. start in `headless` for unattended tasks or `headful` for live debugging
2. navigate to the target URL
3. use `eval` / `scroll` immediately if you do not need node IDs yet
4. call `GET /dom` when you need a fresh structural view of the page
5. locate target node IDs from the snapshot
6. execute `click` / `set_value` / `eval`
   `scroll` is also available as a first-class command for viewport movement without ad hoc JS
7. read incremental events with `since=<latest_sequence>`
8. snapshot session if login or state matters
9. enable traces for runs you may need to replay

## Constraints

- `serve` defaults to the workspace release host library and refreshes the workspace runtime when sources are newer; use `--host-lib` for an explicit override
- the canonical local command path uses the installed bundled CLI, not an on-demand rebuild
- native builds require the platform CEF SDK under `third_party/cef/...`
- the published GitHub repo intentionally excludes the vendored CEF binary payload
- local ad hoc signing can reduce repeated local trust noise on macOS, but it does not bypass macOS
  Automation, Accessibility, or other privacy approvals
