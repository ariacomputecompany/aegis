# TODAYBUGS

Date: August 26, 2026
Repo: `/Users/deepsaint/Desktop/aegis`
Scope: Bugs, DX gaps, productization risks, and usability failures identified during a real production-shaped Aegis session driving Zapier end to end.

This document starts with verified issues from the Zapier workflow audit completed today. It should be expanded as the deeper source and runtime pass continues.

## Severity Guide

- `P1`: trust, correctness, or production-blocking issue
- `P2`: serious usability, reliability, or workflow issue
- `P3`: notable DX or product quality issue

## Verified Today

### 1. Visible editor changes did not reliably persist to actual app state

- Severity: `P1`
- Surface: Aegis page interaction against Zapier's workflow editor
- What happened:
  - A visible webhook URL field was edited successfully in the rendered page.
  - The page visually reflected the new value.
  - The underlying workflow state still retained the old value.
  - Zapier's own test summary and published workflow continued to use the previous URL.
- Why this matters:
  - Aegis made it too easy to believe a stateful edit succeeded when the effective app state had not actually changed.
  - For production browser automation, "visible DOM changed" is not a strong enough success criterion for structured editors.
- Aegis gap:
  - No first-class "committed application state changed" affordance.
  - No built-in helper for distinguishing contenteditable / rich editor mutation from true application commit.

### 2. Rich editor fields behaved unlike normal inputs and did not save reliably

- Severity: `P2`
- Surface: Zapier Slate/contenteditable inputs for URL and JSON step configuration
- What happened:
  - Standard text-setting behavior appeared to succeed.
  - The rich field often reverted or failed to update the actual workflow state.
  - Repeated retries were needed using alternate interaction styles.
- Why this matters:
  - Many modern SaaS apps use rich text or contenteditable controls instead of plain inputs.
  - If Aegis does not provide a reliable path for those fields, core browser automation tasks become brittle.
- Aegis gap:
  - `set_value` semantics are not obviously trustworthy for structured contenteditable editors.
  - There is no prominent guidance for when agents should prefer raw keypress simulation or a higher-confidence commit strategy.

### 3. The runtime encouraged overreliance on unstable node ids

- Severity: `P2`
- Surface: repeated Zapier editor interactions
- What happened:
  - DOM node ids changed frequently across rerenders, route transitions, and editor states.
  - Some ids worked briefly and then disappeared or stopped being actionable.
  - The workflow often required repeated DOM rediscovery before each action.
- Why this matters:
  - The runtime itself documents semantic matching as canonical, but practical discovery still pushes agents toward raw ids.
  - That creates fragile automation by default.
- Aegis gap:
  - The low-level API surface makes id-based control very convenient even when it is the wrong abstraction.
  - The product needs stronger nudges and better primitives for semantic action targeting and re-resolution.

### 4. "Node not targetable" failures were too opaque

- Severity: `P2`
- Surface: buttons and controls that existed visually but could not be clicked
- What happened:
  - Aegis returned failures such as "node is not targetable for click" even when the element was clearly visible in the page.
  - The reason was often hydration timing, modal layering, rerender drift, or a mismatch between visible and actionable elements.
- Why this matters:
  - The current failure mode is technically true but not actionable enough for agents or developers.
  - It slows diagnosis and makes the product feel unpredictable.
- Aegis gap:
  - Needs richer error categories and guidance:
    - stale node
    - hidden by overlay
    - non-actionable wrapper
    - rerendered control
    - wrong frame/context

### 5. The platform made it hard to tell whether a click changed runtime state or only page presentation

- Severity: `P1`
- Surface: save, continue, publish, and retest flows
- What happened:
  - There were several cases where a click advanced the UI, but the persisted application state still reflected old values.
  - The only safe verification path was to re-read test summaries, published state, or backend side effects.
- Why this matters:
  - Production browser automation needs higher-confidence state transitions than "the button click returned ok."
- Aegis gap:
  - Missing higher-level workflow assertions for:
    - page state committed
    - request emitted
    - route changed and draft mutated
    - published state matches intended config

### 6. Product discoverability is too weak from the CLI and docs surface

- Severity: `P3`
- Surface: agent/operator DX
- What happened:
  - Even after using Aegis extensively, the practical control model and feature boundaries were not obvious quickly.
  - Important functionality existed, but the packaging did not make it legible enough under pressure.
- Why this matters:
  - If a highly capable agent still has to rediscover the control surface mid-task, the product's affordances are not strong enough.
- Aegis gap:
  - Capability packaging and quick-start guidance need improvement.
  - The product should make "what exists, what is stable, and what is canonical" immediately obvious.

### 7. Installation and product packaging leave room for confusion about the app vs CLI relationship

- Severity: `P3`
- Surface: product and install DX
- Expected product flow:
  - install Aegis once
  - get the local browser app
  - get the canonical CLI that agents use
  - use the same installed runtime model from there
- Observed gap:
  - The repo and runtime conceptually support this, but the packaging is still easy to misunderstand.
  - It is not yet obvious enough that the installed app and installed CLI are meant to form one production path.
- Why this matters:
  - This is especially important for the future cloud/containerized model where a clear runtime contract matters.

### 8. Credential capture and reuse did not feel reliable enough

- Severity: `P1`
- Surface: login assistance and saved credential behavior
- What happened:
  - In a session where credential persistence should have materially helped, Aegis did not obviously save and then automatically reuse credentials in a way that reduced friction.
  - Manual credential entry was still required.
- Why this matters:
  - The docs explicitly claim Aegis auto-stores credentials by default after username/password entry plus submit-like action.
  - If that is not happening reliably, it is both a product bug and a trust issue.
- Aegis gap:
  - Credential save and replay need explicit verification coverage.
  - The user and agent need much clearer signals when credentials were captured, matched, or intentionally not replayed.

### 8d. Credential auto-store failed on a minimal standards-shaped login form

- Severity: `P1`
- Surface: source + runtime repro
- What happened:
  - A local test page used:
    - `autocomplete="username"`
    - `autocomplete="current-password"`
    - a real `type="submit"` button labeled `Sign in`
    - `set_value` for both fields followed by a click on the submit button
  - The interaction executed successfully through `/execute`.
  - `aegis config credentials-list --profile default` did not show a new saved credential entry for that origin.
- Why this matters:
  - This is close to the simplest realistic login flow Aegis should support.
  - If auto-store misses even this case, the capture system is not dependable enough to market as default behavior.
- Current assessment:
  - This is now a concrete runtime repro, not just a suspicion.

### 8e. Saved credentials were not automatically replayed on same-origin revisit in the runtime

- Severity: `P1`
- Surface: runtime repro
- What happened:
  - After the minimal login interaction above, the test page was revisited on the same origin with a fresh query string and `Cache-Control: no-store`.
  - The username and password fields came back empty.
  - The page status reset to idle, so this was not just the previous filled form state being carried forward.
- Why this matters:
  - This is the practical user promise behind credential persistence.
  - If the runtime neither stores nor visibly replays credentials on revisit, the feature does not reduce operator friction.
- Current assessment:
  - This runtime behavior is consistent with the earlier source-level concern that stored credentials have no effective automatic replay path.

### 8f. Revisited sites do not surface any "credentials available" metadata back to the agent

- Severity: `P1`
- Surface: credential assist UX / runtime metadata contract
- Evidence from source:
  - Stored credential entries include an `origin` and secret payload in the Aegis-owned store.
  - Current source inspection did not reveal a browser-time path that, on navigation or page load, checks the current site against saved credentials and exposes that match as agent-visible metadata.
  - Existing credential surfaces are oriented around storage/listing, not page-time match signaling.
- Why this matters:
  - Even when automatic credential use is disabled, the agent still needs a clean signal that credentials already exist for the current site.
  - Without that signal, the agent has to rediscover or guess whether login help is available.
  - This turns a potentially strong assist feature into hidden state.
- Current assessment:
  - This is a real product-gap bug in the credential assist contract, not just a missing nicety.

### 8g. The settings model cannot express "store and recognize credentials, but do not auto-use them"

- Severity: `P1`
- Surface: config contract / product behavior policy
- Evidence from source:
  - `CredentialsSettings` in [src/config_store.rs](/Users/deepsaint/Desktop/aegis/src/config_store.rs) currently exposes `auto_store`, but current source inspection did not reveal a separate `auto_use`, `auto_replay`, or similarly named replay/autofill policy toggle.
  - That means the present config surface does not clearly distinguish:
    - capture credentials
    - announce credentials are available
    - automatically inject/use credentials
- Why this matters:
  - The desired product behavior needs at least two distinct modes:
    - passive recognition plus agent-visible availability when auto-use is off
    - automatic replay/use when auto-use is on
  - If the config model only represents storage, the product cannot cleanly implement or document those modes.
- Current assessment:
  - This is a real contract gap between the desired UX and the current configuration model.

### 8h. The runtime has no clear page-load recognizer that binds the current site to saved credentials

- Severity: `P1`
- Surface: runtime credential matching / navigation lifecycle
- Evidence from source:
  - `AutoCredentialCapture` persists credentials after qualifying interactions.
  - Current source inspection did not reveal a complementary navigation/page-load hook that evaluates the active URL against saved Aegis-owned credentials and produces a match decision at page time.
  - `load_profile_credentials()` appears inspection-oriented rather than part of the live browser lifecycle.
- Why this matters:
  - Site recognition has to happen automatically during navigation if Aegis is going to:
    - tell the agent credentials are available
    - auto-apply them when policy allows
    - report failures when replay does not work
  - Without a page-load recognizer, credential storage remains disconnected from actual browsing.
- Current assessment:
  - This is a core implementation gap behind the broader "saved credentials do not help enough" complaint.

### 8i. The product does not define or expose its site-matching policy clearly enough for saved credentials

- Severity: `P2`
- Surface: credential matching semantics / product contract
- Evidence from source:
  - Stored credential entries currently carry an `origin`.
  - Current source inspection did not reveal a clearly surfaced contract for whether matching is supposed to occur by:
    - exact origin
    - registrable domain
    - subdomain family
    - some broader site-level rule
- Why this matters:
  - The user-facing behavior depends heavily on these semantics.
  - Login flows routinely bounce across:
    - `www`
    - app subdomains
    - auth subdomains
    - federated or redirect-heavy paths
  - If matching policy is implicit or too narrow, credential availability will feel random and hard to trust.
- Current assessment:
  - This is a real product-contract gap that should be made explicit before the credential assist story is considered production-ready.

### 8j. There is no clear runtime outcome channel for credential availability, auto-use, success, or failure

- Severity: `P1`
- Surface: agent feedback / observability / login UX
- Evidence from source:
  - Existing source inspection did not reveal an agent-facing runtime status model that reports events such as:
    - credentials available for this site
    - credentials auto-applied
    - credentials withheld because auto-use is disabled
    - credentials attempted but rejected or ineffective
  - The current bug history already shows that capture and replay are not self-evident during use.
- Why this matters:
  - A silent credential system is hard for agents to use safely.
  - If replay fails quietly, the agent cannot distinguish:
    - no credentials exist
    - credentials exist but auto-use is off
    - credentials were used but were rejected
    - credentials were never matched in the first place
  - Production browser automation needs these states to be explicit.
- Current assessment:
  - This is a real observability and product-trust gap in the login assistance workflow.

### 8a. Saved credentials appear to have no automatic replay path in the runtime

- Severity: `P1`
- Surface: source-backed product gap
- Evidence from source:
  - `AegisSecretStore` supports storing, listing, removing, and clearing saved credentials.
  - `AutoCredentialCapture` in `src/api/server.rs` captures and persists credentials.
  - Current source inspection did not reveal a corresponding runtime path that loads saved credentials and proactively applies them back into matching login forms.
  - `load_profile_credentials()` appears wired to CLI/config inspection, not browser-time autofill or agent assist behavior.
- Why this matters:
  - The product docs imply credential storage should materially help future sessions.
  - If the runtime stores credentials but never reuses them automatically, the product promise is overstated and the UX is misleading.
- Current assessment:
  - This looks like a real product bug or at minimum a severe contract/DX mismatch.

### 8b. Credential auto-capture only tracks `set_value` writes, not general input entry

- Severity: `P1`
- Surface: source-backed login capture behavior
- Evidence from source:
  - `AutoCredentialCapture::capture_fields()` only inspects `Command::SetValue`.
  - It does not capture text entered via `PressKey`, browser autofill, paste-like behavior outside `set_value`, or site-native value changes that occur after focus/input handling.
  - Pre-execution snapshot capture is only attempted when the command batch contains `SetValue` or `Click`.
- Why this matters:
  - Modern login flows often use:
    - typed key entry
    - masked/reactive inputs
    - browser autofill
    - multi-step flows where password submission happens separately
  - Restricting capture to `set_value` makes credential persistence fragile by design.
- Current assessment:
  - This is a concrete implementation gap, not just a vague UX complaint.

### 8c. Credential submit detection is heuristic-heavy and likely too narrow for production SaaS flows

- Severity: `P2`
- Surface: source-backed runtime behavior
- Evidence from source:
  - Persistence depends on submit-like click or enter-key heuristics.
  - Heuristics include text checks such as `sign in`, `log in`, `continue`, `submit`, and password-field matching.
  - The persistence decision is tightly coupled to a single pre-execution DOM snapshot.
- Why this matters:
  - Many auth flows use:
    - icon-only submit buttons
    - federated auth redirects
    - multi-screen auth
    - OTP or password-next flows
    - nonstandard control labels
  - Heuristic-only submit detection will miss valid login commits or create inconsistent capture behavior.
- Current assessment:
  - This area needs stronger coverage and likely a broader model of auth completion.

### 9. The runtime lacks strong built-in guidance for high-confidence verification

- Severity: `P2`
- Surface: debugging and completion confidence
- What happened:
  - Confidence came from manually combining DOM reads, route checks, network inference, backend validation, and published-state inspection.
  - Aegis enabled this, but did not package it into a stronger first-class verification workflow.
- Why this matters:
  - Productized browser control needs confidence loops, not just action primitives.
- Aegis gap:
  - Missing ergonomic patterns or commands for:
    - "did this save"
    - "did this publish"
    - "did this form submit"
    - "did this interaction mutate app state"

### 10. Production-scale browser control needs more obvious support for SaaS-style stateful editors

- Severity: `P2`
- Surface: productization risk
- What happened:
  - Zapier is exactly the kind of modern app Aegis should handle well in production:
    - dynamic rerenders
    - rich editors
    - async save flows
    - hidden modals
    - delayed publish gates
  - The session was successful, but it took too much manual recovery and inference.
- Why this matters:
  - At cloud scale, brittle editor workflows become reliability incidents and operator toil.
- Aegis gap:
  - Needs stronger first-class support for:
    - reactive app commits
    - async control stabilization
    - semantic re-resolution
    - high-confidence state verification

### 11. Canonical verification is weaker in practice than the green unit suite suggests

- Severity: `P1`
- Surface: repo verification workflow
- What happened:
  - `cargo test` passed cleanly.
  - The repo's host-backed runtime smoke path did not become command-ready and failed with connection refused.
  - Later detached runtime launches also failed to become reachable within the built-in timeout.
- Why this matters:
  - The project presents host-backed verification as the production-shaped confidence path.
  - If that path fails while the unit suite stays green, the main quality signal can be misleading.
- Aegis gap:
  - Runtime boot verification is not strong enough relative to the claims made by the verification surface.
  - More of the "real browser actually came up and served the documented control plane" contract needs to be covered by default gates.

### 12. Detached `serve` startup failed repeatedly after a clean install and gave almost no diagnostic signal

- Severity: `P1`
- Surface: `aegis serve --detach`
- What happened:
  - Fresh detached launches on new ports returned:
    - `detached Aegis serve did not become reachable ... within 15s`
  - The referenced log files were effectively empty or not helpful enough to explain why startup failed.
  - Explicitly passing the installed host library did not rescue the failure.
- Why this matters:
  - Detached serve is a first-class advertised workflow.
  - When it fails silently, operators and agents lose the main unattended runtime path and cannot self-diagnose quickly.
- Aegis gap:
  - Startup observability is too weak.
  - The timeout-only failure mode hides whether the runtime is:
    - blocked before binding
    - crashed very early
    - stuck in native bootstrap
    - waiting on browser readiness

### 13. Runtime/API version skew was easy to end up in and hard to recognize

- Severity: `P1`
- Surface: installed app/CLI/runtime contract
- What happened:
  - Before reinstalling, the installed `aegis_cli` lacked `navigate`, `search`, and `page` subcommands that the repo source clearly defines.
  - A long-running installed runtime exposed `/navigate` but not `/search` or `/page*` routes.
  - The docs and examples therefore described capabilities that were not actually present in the active installed product.
  - Re-running `./install.sh` brought the installed CLI surface back into alignment.
- Why this matters:
  - Agents and users can believe a feature exists because the repo and docs say it does, while the installed product is still serving an older control plane.
  - This creates a trust problem that looks like "user error" until someone realizes the install/runtime is stale.
- Aegis gap:
  - The product does not make version skew obvious enough.
  - The runtime and CLI could expose stronger self-identification or compatibility warnings when the installed surface lags the repo expectations.

### 14. `native doctor` can present a mixed-source runtime picture that is confusing in practice

- Severity: `P2`
- Surface: native/install diagnostics
- What happened:
  - After install and workspace build, `native doctor` reported:
    - installed app executable paths under `~/Applications/Aegis.app`
    - canonical install host library under the installed app
    - but `default_host_library` resolving to the workspace build output
  - That means the "default" runtime story can appear to straddle installed and workspace artifacts at once.
- Why this matters:
  - This makes it harder to answer a basic operator question:
    - "what exact runtime will Aegis use if I just run it?"
  - Mixed-source diagnostics increase the risk of half-installed or accidentally non-production execution paths.
- Aegis gap:
  - Diagnostic naming and source-of-truth explanation need to be clearer.
  - If workspace-preferred behavior is intentional, the product should say that much more explicitly.

### 15. The `/execute` API is still too easy to misuse even for simple local tasks

- Severity: `P2`
- Surface: raw HTTP control plane usability
- What happened:
  - A simple local credential-capture test page was reachable through `/navigate`.
  - A naive but reasonable `/execute` payload using `target: { selector: ... }` failed with HTTP `422`.
  - The returned error was a generic enum-deserialization message:
    - `data did not match any variant of untagged enum CommandTarget`
  - The live manifest exposed only a short summary for `/execute`, not enough concrete shape guidance to prevent this quickly.
- Why this matters:
  - This is the core low-level API.
  - If basic command payloads are easy to shape incorrectly and the recovery hint is this opaque, API ergonomics are not strong enough for agent-facing use.
- Aegis gap:
  - The manifest should provide stronger request examples and target-shape documentation on the live route surface.
  - Validation errors should translate payload-shape mistakes into much more actionable messages.

### 16. CLI and docs ergonomics still leak implementation details at the wrong layer

- Severity: `P2`
- Surface: command-line DX
- What happened:
  - The product advertises higher-level flows like `search`, `navigate`, and `page`.
  - In practice, operators still have to reason about:
    - host library provenance
    - workspace-vs-installed runtime resolution
    - stale installed surfaces
    - whether a given running port belongs to an older control plane
  - Even the `--server-addr` workflow is easy to get wrong when the runtime behind that address is stale.
- Why this matters:
  - A polished agent browser should collapse these distinctions for normal usage.
  - Instead, users can get pulled into runtime topology and install-state debugging before doing useful work.
- Aegis gap:
  - More of the runtime provenance and compatibility complexity should be absorbed by the tool itself.

### 17. Fresh foreground `serve` attempts could hang without binding a port or emitting useful progress

- Severity: `P1`
- Surface: non-detached runtime boot
- What happened:
  - A fresh foreground `cargo run ... serve --addr 127.0.0.1:7889` attempt did not bind the port within the first few seconds.
  - Polling `/healthz`, `/readyz`, and `/manifest` returned connection refused.
  - The foreground session did not produce enough visible output to explain what stage it was stuck in.
- Why this matters:
  - This is the manual fallback path when detached mode is flaky.
  - If both detached and foreground boot can fail opaquely, operability degrades fast.
- Aegis gap:
  - Serve bootstrap needs better phase logging and clearer readiness-state emission before the full control plane is available.

### 18. Capability validation overclaims for research workflows relative to actual integration coverage

- Severity: `P2`
- Surface: manifest + verification contract
- What happened:
  - The manifest marks `research_workflow_primitives` as a supported capability with validation references such as:
    - `/search`
    - `/page/text`
    - `/page/find`
    - `/page/links`
    - `/page/open-link`
  - The repo-owned host-backed smoke path does not exercise those research endpoints.
  - In live use, an active runtime at `127.0.0.1:7878` returned `404` for `GET /page`, which also caused `aegis page inspect` to fail.
- Why this matters:
  - The product is signaling a stronger validation story than the integration path actually proves.
  - This increases the chance that green verification and real operator experience diverge.
- Aegis gap:
  - Capability-status wording should track real exercised coverage more honestly.
  - Research workflow primitives need dedicated host-backed verification, not just documentation and route registration.

### 19. Failed `serve` launches can linger as non-listening processes

- Severity: `P1`
- Surface: runtime lifecycle / operability
- What happened:
  - Several failed launches remained visible in `ps`, including runtimes for ports such as:
    - `7881`
    - `7882`
    - `7884`
    - `7886`
    - `7888`
    - `7889`
  - Those processes were still present at the OS level but returned connection refused on both `/healthz` and `/manifest`.
- Why this matters:
  - This creates a confusing in-between state:
    - the process looks alive
    - the control plane is dead
    - the port is not serving
  - At product scale, this becomes scheduler waste, noisy diagnostics, and hard-to-reason-about failure recovery.
- Aegis gap:
  - Failed runtimes should either:
    - exit decisively
    - or expose explicit crash/bootstrap-failed diagnostics
  - The current half-alive state is operationally hazardous.

### 20. Updated CLI and stale runtime can disagree without a strong compatibility warning

- Severity: `P1`
- Surface: client/runtime compatibility
- What happened:
  - After reinstall, the local CLI clearly supported `page inspect`.
  - Pointing that CLI at the long-running runtime on `127.0.0.1:7878` produced:
    - `Aegis API GET /page failed (404)`
  - The runtime was still healthy enough to answer `/healthz` and `/manifest`, but it did not support the route the newer CLI expected.
- Why this matters:
  - This is a very plausible real-world state:
    - CLI updated
    - old serve process still running
    - user assumes the system is coherent
  - Without a strong mismatch warning, the failure presents as a confusing missing-feature bug.
- Aegis gap:
  - The CLI should detect stale runtime manifests and explain the compatibility problem directly.
  - Runtime identity likely needs stronger feature-version signaling than the current basic version fields.

### 21. Stale runtimes can expose only route summaries, not enough machine-readable request-shape guidance

- Severity: `P2`
- Surface: manifest discoverability / DX
- What happened:
  - On the long-running runtime at `127.0.0.1:7878`, `/manifest` returned route summaries but no `schemas` array and no route-level request examples.
  - For example, `POST /contexts` was described only as:
    - `Create a new isolated browser context`
  - That forced manual guesswork about the correct request shape.
- Why this matters:
  - Aegis is explicitly agent-facing.
  - If a live runtime does not expose enough machine-readable guidance to form requests correctly, agents will make avoidable mistakes and appear flaky.
- Aegis gap:
  - Manifest richness appears version-sensitive enough to become a real product issue.
  - The compatibility story between newer docs/CLI and older live manifests needs to be much more explicit.

### 22. `DELETE /contexts/default` returns a bridge-style `502` instead of a cleaner client-facing contract error

- Severity: `P2`
- Surface: context management semantics
- What happened:
  - Attempting to delete the default context returned:
    - HTTP `502`
    - `bridge error: default context cannot be deleted`
- Why this matters:
  - This is not an upstream-bridge failure in the ordinary user sense.
  - It is a stable product rule.
  - Surfacing it as a bridge-flavored `502` makes the API feel less intentional than it should.
- Aegis gap:
  - Product-rule violations like this should likely map to a clearer 4xx class contract error.

### 23. `/events/live` replays the entire retained backlog by default, which is risky for scale and ergonomics

- Severity: `P2`
- Surface: SSE runtime events
- What happened:
  - Opening `/events/live` with no cursor immediately streamed a giant `runtime_events` payload from sequence `0`, including a very large retained history blob.
  - In the observed runtime, that included thousands of events and a bulky DOM mutation/error backlog.
- Why this matters:
  - This is expensive and noisy for:
    - interactive debugging
    - agent startup
    - massively parallel cloud session consumers
  - Defaulting to a full backlog dump is not a great product posture at scale.
- Aegis gap:
  - Event streaming likely needs a cleaner default tail behavior, stronger cursor ergonomics, or clearer warnings about backlog size.

### 24. Live runtime event history can be dominated by stale bootstrap/noise state

- Severity: `P3`
- Surface: event usability / diagnostics
- What happened:
  - The long-running default runtime's retained event history started with a large `ERR_CONNECTION_REFUSED` page trace tied to `localhost`.
  - That noisy historical state was the first thing exposed through `/events` and `/events/live`.
- Why this matters:
  - Even when technically correct, this reduces signal quality for current debugging.
  - It makes the product feel "dirty" unless the consumer already knows how to reason about event cursors and historical baggage.
- Aegis gap:
  - The product should make it easier to distinguish:
    - current meaningful state
    - historical retained noise
    - startup/bootstrap leftovers

### 25. Session snapshot, save, inject, and load disagree about actual browser state

- Severity: `P1`
- Surface: session persistence contract
- What happened:
  - In a named context, a local test page set:
    - `localStorage.theme = "dark"`
    - `sessionStorage.flow = "checkout"`
    - a normal `document.cookie`
  - `GET /contexts/{context_id}/session` reported:
    - `local_storage: {"theme":"dark"}`
    - `session_storage: {"flow":"checkout"}`
  - But immediately after that, `POST /contexts/{context_id}/session/save` wrote a profile file whose contents were empty:
    - no cookies
    - empty local storage
    - empty session storage
  - Then `POST /contexts/{context_id}/session` with an explicitly empty session returned `204` but did not actually clear the live page state.
  - `POST /contexts/{context_id}/session/load` likewise did not restore or change anything meaningful, because the saved file was already empty.
- Why this matters:
  - This breaks the core persistence contract in multiple directions:
    - snapshot says state exists
    - save does not persist it
    - inject does not clear it
    - load does not reconcile it
  - For productized agent sessions, this is a major correctness issue.
- Evidence:
  - live snapshot returned non-empty storage state
  - saved file at `~/.aegis/profiles/session-audit/session.json` remained empty
  - post-inject probe still saw `theme=dark` and `flow=checkout`

### 26. `session/save` can silently write an empty profile even after a non-empty live snapshot

- Severity: `P1`
- Surface: disk persistence
- What happened:
  - After a non-empty `GET /contexts/session-audit/session`, the saved profile file contained:
    - empty cookies
    - empty local storage
    - empty session storage
  - There was no explicit error or warning that persistence had dropped the current state.
- Why this matters:
  - Silent state loss is worse than a hard failure.
  - It encourages users and agents to believe a recoverable session exists when it does not.
- Aegis gap:
  - `session/save` needs stronger integrity checks between the live snapshot and the persisted artifact.

### 27. Injecting an empty session did not clear live local/session storage

- Severity: `P1`
- Surface: session injection
- What happened:
  - `POST /contexts/{context_id}/session` with a fully empty payload returned success (`204`).
  - A follow-up `eval` still observed:
    - `localStorage.theme = "dark"`
    - `sessionStorage.flow = "checkout"`
    - page status still `"seeded"`
- Why this matters:
  - Session injection is supposed to be a control surface, not a best-effort suggestion.
  - If clearing injected state is ineffective, session reconciliation is not trustworthy.

### 27a. Live session snapshot dropped a simple page cookie that was present in `document.cookie`

- Severity: `P1`
- Surface: session snapshot fidelity
- What happened:
  - A local test page set `document.cookie = 'sid=abc123; path=/'`.
  - A same-context `eval` confirmed the cookie was present in `document.cookie`.
  - `GET /contexts/{context_id}/session` still returned `cookies: []`.
- Why this matters:
  - This suggests the snapshot contract is incomplete even before save/load is involved.
  - If ordinary page cookies are not captured reliably, session portability and restoration will be incomplete by design.

### 28. Deleted/transient contexts leave profile artifacts behind in `~/.aegis/profiles`

- Severity: `P2`
- Surface: local state hygiene / scale ergonomics
- What happened:
  - After multiple context creation and deletion cycles, many context profile files remained under `~/.aegis/profiles/`, including:
    - `context-2`
    - `context-3`
    - `context-4` through `context-11`
    - other transient audit/smoke profiles
  - The active `/contexts` list no longer included those contexts.
- Why this matters:
  - For a platform intended to support large numbers of sessions, profile/state artifact buildup becomes real operational drag.
  - Even if technically intentional, the current product does not make the lifecycle obvious enough.
- Aegis gap:
  - Context teardown and profile retention policy need to be clearer and likely more manageable.

### 29. `media_state` accepts top-level `match` but fails for `target.match` on the same node

- Severity: `P1`
- Surface: execute command contract
- What happened:
  - On a simple page with `<audio id="player" ...>`, these two shapes behaved differently:
    - worked: `{"type":"media_state","match":{"selector":"#player"}}`
    - failed: `{"type":"media_state","target":{"match":{"selector":"#player"}}}`
  - The failing form returned:
    - `bridge error: no node matched {"selector":"#player"}`
  - On that exact same page and node:
    - `geometry` with `match` succeeded
    - `press_key` with `target.match` succeeded
- Why this matters:
  - This is a true contract inconsistency, not a vague flaky interaction.
  - Agents cannot reliably infer which command families want top-level targeting versus nested `target` targeting when the product behaves inconsistently.
- Source note:
  - `execute_media_state()` takes `Option<&CommandTarget>`, so the runtime model itself suggests `target` should be valid.

### 30. `press_key` reports `media_toggled: true` even when playback is immediately rejected by autoplay policy

- Severity: `P1`
- Surface: media interaction correctness
- What happened:
  - `press_key` targeting an audio element returned success with:
    - `media_toggled: true`
  - A follow-up `media_state` on the same element showed:
    - `play_rejected: 1`
    - `last_play_error: "play() failed because the user didn't interact with the document first..."`
    - `likely_failure_cause: "autoplay_policy_blocked"`
    - `paused: true`
  - The event stream also recorded:
    - `media:play_rejected`
    - `page:unhandled_rejection`
- Why this matters:
  - Returning `media_toggled: true` strongly implies user-visible playback state changed.
  - In reality, the media never left the paused state.
- Aegis gap:
  - The command result should distinguish:
    - attempted toggle
    - actually playing
    - rejected by policy
  - Otherwise agents will over-credit synthetic keyboard interaction as successful playback control.

### 31. Keychain-related behavior is still surfacing in the product even though the intended model is “Aegis-owned state only”

- Severity: `P1`
- Surface: macOS runtime behavior + docs/product contract
- What happened:
  - The native app explicitly enables Chromium-managed credential features in [native/aegis_app.cc](/Users/deepsaint/Desktop/aegis/native/aegis_app.cc) and the host-backed native path under `native/src/aegis_cef_host.cpp`:
    - `credentials_enable_service = true`
    - `profile.password_manager_enabled = true`
    - `profile.password_manager_leak_detection = true`
    - `autofill.profile_enabled = true`
    - `autofill.credit_card_enabled = true`
  - First-party docs still explicitly mention Chrome/Brave `Safe Storage`:
    - [README.md](/Users/deepsaint/Desktop/aegis/README.md)
    - [docs/agent-control.md](/Users/deepsaint/Desktop/aegis/docs/agent-control.md)
  - Live Aegis runtime logs under `~/.aegis/logs/` repeatedly show macOS keychain-adjacent errors such as:
    - `Touch ID authenticator unavailable because keychain-access-group entitlement is missing or incorrect`
  - The app entitlement file [aegis.entitlements](/Users/deepsaint/Desktop/aegis/native/mac/aegis.entitlements) is empty, so there is no explicit first-party entitlement path here; the behavior is still leaking through the bundled browser/runtime stack.
  - The startup switch list in [native/aegis_app.cc](/Users/deepsaint/Desktop/aegis/native/aegis_app.cc) disables some browser features, but it does not appear to suppress WebAuthn/passkey/keychain-adjacent behavior. The active `disable-features` list only includes:
    - `LocationProviderManager`
    - `NewMacNotificationAPI`
- Why this matters:
  - From a product perspective, users still experience keychain-related prompts/errors/behavior whether or not Aegis itself "owns" the state.
  - For a product that wants one canonical Aegis-owned persistence path, lingering keychain/Safe Storage references are still product debt.
  - More concretely, the current product contract is internally inconsistent:
    - docs say Aegis-owned state is canonical
    - the native layer still turns Chromium credential services on
    - the runtime still emits keychain/WebAuthn entitlement noise on macOS
- Current assessment:
  - Even if some of the remaining behavior is coming from Chromium/CEF rather than Aegis-authored code, it is still a valid product bug because the user-facing effect has not been eviscerated.

### 31a. The product still ships a mixed credential-storage model instead of one canonical Aegis-owned path

- Severity: `P1`
- Surface: source contract / product architecture / DX
- What happened:
  - First-party docs say the canonical local state model is Aegis-owned under `~/.aegis`.
  - In the same docs, Aegis also says Chromium credential storage and autofill are enabled by default in the runtime.
  - The native app source explicitly enables Chromium credential and autofill preferences.
  - The CLI and config surface also expose Aegis-owned credential capture and storage under `~/.aegis/secrets/...`.
- Why this matters:
  - That leaves the product with two overlapping mental models:
    - Aegis-owned credentials are canonical
    - Chromium-managed credential services are also on and may influence runtime behavior
  - For humans and agents, this makes it unclear which layer is actually supposed to save, replay, or prompt for credentials.
  - For the user's stated product goal, this is not just wording debt; it is a core contract bug.
- Current assessment:
  - If the intended direction is "one canonical way only," Chromium-managed credential services should be removed or explicitly fenced off, not left enabled beside Aegis-owned secrets.

### 32. Current docs still preserve “Safe Storage” wording instead of fully eviscerating that concept from the product surface

- Severity: `P2`
- Surface: docs / DX / packaging
- What happened:
  - The docs say Aegis does not read or write browser login databases or `Safe Storage` entries.
  - That is directionally good, but it still preserves the concept in the product surface instead of presenting one canonical mental model cleanly:
    - Aegis-owned state only
  - Given the goal of eliminating Apple keychain/Safe Storage ambiguity entirely, those mentions are still part of the problem.
- Why this matters:
  - A polished product contract should not require users to reason about excluded browser-secret mechanisms at all unless there is a migration/debugging reason.
  - Keeping the terminology alive in docs reinforces exactly the ambiguity the product is trying to remove.

### 32a. Docs currently teach the mixed-model behavior instead of cleanly declaring one persistence contract

- Severity: `P2`
- Surface: README / agent docs / operator DX
- What happened:
  - Both [README.md](/Users/deepsaint/Desktop/aegis/README.md) and [docs/agent-control.md](/Users/deepsaint/Desktop/aegis/docs/agent-control.md) currently say:
    - Chromium credential storage and autofill are enabled by default
    - Aegis-owned credential capture is stored under `~/.aegis`
    - browser-managed credential storage and autofill use Chromium runtime profile behavior
  - This is stronger than an incidental mention of excluded legacy systems; it actively teaches a split-brain storage model.
- Why this matters:
  - Even if engineering later consolidates behavior, the current docs teach operators and future contributors to think in terms of two credential systems.
  - That directly works against the desired packaging where Aegis installation should present one obvious, canonical runtime contract.
- Current assessment:
  - This is a real DX/product bug, not just a wording nit.

### 33. Live runtime artifacts are still writing legacy `session.json` version `2` while current source expects version `3`

- Severity: `P1`
- Surface: persisted state / version drift
- What happened:
  - Current source in [src/session/profile.rs](/Users/deepsaint/Desktop/aegis/src/session/profile.rs) defines:
    - `const PROFILE_VERSION: u32 = 3;`
  - But live runtime-generated profiles such as:
    - `/Users/deepsaint/.aegis/profiles/session-audit/session.json`
    - `/Users/deepsaint/.aegis/profiles/session-audit-2/session.json`
    were written with:
    - `"version": 2`
  - Meanwhile the current default profile file on disk was version `3`.
- Why this matters:
  - This is concrete evidence of runtime/source drift in a persisted artifact format.
  - A newer build reading older live-written profiles can reset or discard them, which is especially dangerous for long-lived installs and future distributed session portability.
- Source consequence:
  - The current `SessionProfileStore::load()` logic resets profiles whose stored version does not equal the current `PROFILE_VERSION`.
- Current assessment:
  - This is not theoretical; the live product is actively producing older-version session artifacts alongside newer-version expectations.

### 33a. Current source treats version-mismatched session profiles as disposable and resets them to empty state

- Severity: `P1`
- Surface: backward compatibility / destructive migration behavior
- What happened:
  - The current source in [src/session/profile.rs](/Users/deepsaint/Desktop/aegis/src/session/profile.rs) explicitly does this on load:
    - if `stored.version != PROFILE_VERSION`
    - write a default empty profile payload
    - return `SessionState::default()`
  - The unit test `load_resets_legacy_session_profile_versions` confirms this reset behavior.
  - Combined with the live evidence above that some runtime-created profiles are still `version: 2`, this means older live-written session state is vulnerable to silent wipeout by newer code.
- Why this matters:
  - This is a destructive compatibility strategy, not a migration strategy.
  - For long-lived installs and any future distributed session portability story, that is extremely risky.

### 34. `serve --detach` can report success while `/readyz` is still failing on the synthetic bootstrap shell

- Severity: `P1`
- Surface: runtime lifecycle / readiness contract / automation startup
- What happened:
  - Fresh detached launches on new ports returned success JSON immediately, for example:
    - `127.0.0.1:7892`
    - `127.0.0.1:7893`
  - The corresponding logs also said:
    - `Aegis serve ready on http://127.0.0.1:<port>`
  - But an immediate `GET /readyz` on those same fresh runtimes returned `503` with:
    - `code: "not_ready"`
    - `stage: "stuck_on_bootstrap_shell"`
    - `error: "runtime is attached, but is still on the synthetic bootstrap shell"`
  - The runtime only became ready after an explicit navigation away from `https://bootstrap.aegis/`.
- Why this matters:
  - The current README says:
    - `aegis serve` fails fast if the browser cannot reach an operational runtime
    - a running server means the active page reached a verified automation-ready state
    - `/healthz` and `/readyz` stay false until the runtime is actually usable
  - In practice, detached startup currently overclaims readiness:
    - the command succeeds
    - the log says "serve ready"
    - the runtime still fails the readiness endpoint
  - For automation systems orchestrating many sessions, this is a serious contract bug because process-start success is being conflated with automation-readiness.
- Current assessment:
  - Either detached startup needs to block until `/readyz` is truly healthy, or the command/logging contract needs to say it only guarantees attachment to the bootstrap shell, not an operational automation page.

### 35. First-class `search` can return before the runtime has actually transitioned to the search results page

- Severity: `P1`
- Surface: high-level research workflow / page-state consistency
- What happened:
  - On a fresh runtime at `127.0.0.1:7894`, the browser was first on `https://example.com/`.
  - Running:
    - `aegis --server-addr 127.0.0.1:7894 search mdn input file accept`
    returned success immediately.
  - But the next immediate high-level inspections still described the old page:
    - `page actions` returned `title: "Example Domain"`
    - `page inspect` returned `url: "https://example.com/"`
  - After a short delay, the same runtime finally reflected the DuckDuckGo results page and the page APIs changed accordingly.
  - On a second fresh runtime at `127.0.0.1:7895`, the race was severe enough to affect a follow-up action:
    - after `search` returned, an immediate `page open-link "Learn more"` still matched the old Example Domain page
    - the runtime navigated to `https://iana.org/domains/example`
    - it did not operate on the intended search results page at all
- Why this matters:
  - `search` is a first-class workflow primitive. Users and agents reasonably expect its successful return to mean the active page is now the search results page.
  - When `search` completes early, the next action may be grounded in stale page state:
    - stale links
    - stale controls
    - stale summaries
    - wrong follow-up actions entirely
  - In a multi-step agent loop, that kind of race is much worse than a normal navigation lag because it contaminates the semantic layer, not just the transport layer.
  - The follow-up-command repro shows this is not just a read-after-write consistency issue; it can actively drive the wrong site.
- Current assessment:
  - `search` should wait until the target results page is actually the active page for the page-research APIs, or it should return an explicit transitional state instead of presenting the navigation as complete.

### 36. Successful file uploads leave staged copies behind in `~/.aegis/files/uploads` with no evident cleanup path

- Severity: `P1`
- Surface: file-upload lifecycle / disk hygiene / cloud-session productization
- What happened:
  - The runtime code path in [src/runtime/executor.rs](/Users/deepsaint/Desktop/aegis/src/runtime/executor.rs) stages uploads by copying them into the Aegis-owned upload area via [src/transfers.rs](/Users/deepsaint/Desktop/aegis/src/transfers.rs).
  - That code reads the staged file back for injection, but current source inspection did not reveal a corresponding cleanup path after successful attachment.
  - Live runtime evidence matches that concern:
    - before one successful `set_files` call on a `file://` upload fixture page, `~/.aegis/files/uploads` contained `12` files
    - after that one successful upload, it contained `13`
    - the new staged artifact appeared as `1787353337197-aegis-upload-four.txt`
  - Previously staged upload artifacts also remained on disk after the runtime that created them had already been killed.
- Why this matters:
  - For a product intended to run massively parallel browsing sessions, persistent upload staging without cleanup becomes:
    - silent disk growth
    - cross-session artifact buildup
    - avoidable leakage of previously uploaded local material
  - This is especially problematic because the product’s canonical state area under `~/.aegis` is supposed to be intentional and inspectable, not an unbounded garbage pile.
- Current assessment:
  - Upload staging should be explicitly lifecycle-managed:
    - either delete staged copies after successful injection
    - or track and garbage-collect them per context/session/profile with a clear retention policy

### 37. The page-research surface does not teach agents the canonical `set_files` action for file inputs

- Severity: `P2`
- Surface: file-upload DX / high-level agent affordances
- What happened:
  - On a minimal page containing a labeled file input, `page inspect` correctly identified the control as:
    - `control_type: "file"`
    - `label: "Upload"`
  - But the surfaced actions were only:
    - `type`
    - `focus`
    - `hover`
    - `press_key`
  - The canonical first-class upload command is actually `set_files`, and the manifest explicitly documents that `set_files` takes `paths`.
- Why this matters:
  - Aegis claims first-class file-upload support and even marks that capability as validated.
  - But the high-level page surface nudges the agent toward the wrong primitives for file inputs:
    - typing into a file input is not the real workflow
    - the correct action is a specialized upload command
  - This creates unnecessary manifest-diving, trial-and-error, or failed guesses in exactly the workflow the product says should be first-class.
- Current assessment:
  - File inputs should advertise `set_files` directly in the page-research/action layer so the semantic surface and the actual command model agree.

### 38. `eval` silently drops primitive return values, and the manifest example demonstrates the broken path

- Severity: `P2`
- Surface: `/execute` contract / manifest truthfulness / agent DX
- What happened:
  - The `/manifest` route currently documents `/execute` with this example:
    - `{"commands":[{"type":"eval","code":"document.title"}]}`
  - The manifest also describes `eval` as:
    - `Execute JavaScript and return the result`
  - On a fixture page titled `Eval Fixture`, actual runtime behavior was:
    - `code: "document.title"` -> `{ "ok": true }`
    - `code: "return document.title"` -> `{ "ok": true }`
    - `code: "return 42"` -> `{ "ok": true }`
    - `code: "return ({title: document.title, body: document.body.innerText})"` -> returned the object correctly
    - thrown errors do propagate as errors
- Why this matters:
  - The documented example leads agents directly into a no-value success response.
  - Primitive-return loss is especially confusing because:
    - the command still reports success
    - object returns work
    - the manifest promises a result either way
  - This is exactly the kind of subtle contract bug that causes wasted debugging time in agent loops.
- Current assessment:
  - Either `eval` should preserve primitive results consistently, or the manifest/help text must explicitly document the current limitation and stop presenting broken examples as canonical usage.

### 39. Plain `navigate` can return before the page-research layer has semantically settled

- Severity: `P2`
- Surface: high-level navigation workflow / page classification stability
- What happened:
  - On a fresh runtime, navigating directly to the MDN `accept` attribute page returned success immediately.
  - An immediate follow-up `page actions` response already had the correct URL and title, but still classified the page as:
    - `page_type: "dashboard"`
  - After a short delay on the same unchanged page, the same high-level inspection reclassified it as:
    - `page_type: "documentation"`
  - The delayed snapshot also surfaced a much fuller semantic view of the page than the immediate one.
- Why this matters:
  - This shows the semantic-settling race is not limited to `search`; it affects plain `navigate` too.
  - For agents, that means the same page can present two materially different high-level interpretations depending only on how quickly the next step runs.
  - Even when follow-up actions are not outright stale, unstable page typing and partial semantic extraction make planning less trustworthy.
- Current assessment:
  - `navigate` should either block until the page-research layer has reached a stable post-navigation state, or it should return an explicit transitional status so callers know the semantic snapshot is still warming up.

### 40. Context identity is split between internal `primary` and API-visible `default`, and the wrong one fails seeding with a bridge-style `502`

- Severity: `P2`
- Surface: named-context API / multi-context DX / error semantics
- What happened:
  - Runtime diagnostics expose the active context internally as `primary`.
  - The public `/contexts` API, however, lists the default context as `default`.
  - Creating a seeded context with:
    - `{"id":"clone-a","seed_from_context":"primary"}`
    failed with HTTP `502` and:
    - `bridge error: context 'primary' is not available for seeding`
  - Using the API-visible id instead:
    - `{"id":"clone-c","seed_from_context":"default"}`
    succeeded, and the resulting context correctly inherited the source context's local and session storage.
- Why this matters:
  - This is a real contract split in one of the most important product surfaces for parallel agent sessions.
  - An agent inspecting `/doctor` or `/runtime` can reasonably infer that `primary` is the canonical default context id, then immediately fail when using that id in the public context API.
  - Returning a bridge-style `502` for what is effectively an identifier-contract mismatch makes the failure harder to interpret than it should be.
- Current assessment:
  - The product should present one canonical identifier for the default context everywhere, and misuse of a non-public/internal id should surface as a clearer client-side validation error rather than an opaque bridge error.

### 41. The manifest still overclaims some capability status as `validated` even when the audited workflow contract is demonstrably broken

- Severity: `P2`
- Surface: self-discovery / capability trust / platform orchestration
- What happened:
  - The manifest currently marks `research_workflow_primitives` as:
    - `status: "validated"`
    - `runtime_validated: true`
    - `validated_by: "/search + /page/text + /page/find + /page/links + /page/open-link"`
  - But this audit already verified that:
    - `search` can return before the runtime has actually transitioned to the search results page
    - immediate follow-up actions can still execute against the old page
    - plain `navigate` can also return before the page-research layer has semantically settled
  - So the exposed primitives exist, but the actual workflow contract is weaker than the manifest implies.
- Why this matters:
  - In a productized agent platform, capability manifests become planning inputs.
  - If the system self-reports a workflow as fully validated when the real contract still has stale-page and stale-semantics races, orchestration layers will over-trust it.
  - That leads to harder-to-debug failures than simply labeling the workflow experimental or partially validated.
- Current assessment:
  - Capability status should reflect end-to-end workflow reliability, not just route existence plus a narrow happy-path probe.

### 42. Apple keychain and `Safe Storage` concepts have still not been fully eviscerated from the product contract

- Severity: `P1`
- Surface: macOS storage model / docs / product contract clarity
- What happened:
  - The current first-party docs still explicitly mention browser-managed secrets and Apple-adjacent browser storage surfaces:
    - [README.md](/Users/deepsaint/Desktop/aegis/README.md) says `Chromium credential storage and autofill are enabled by default in the production runtime`
    - [README.md](/Users/deepsaint/Desktop/aegis/README.md) also says `browser-managed credential storage and autofill use Chromium's runtime profile behavior`
    - [README.md](/Users/deepsaint/Desktop/aegis/README.md) still mentions `Safe Storage`
    - [docs/agent-control.md](/Users/deepsaint/Desktop/aegis/docs/agent-control.md) repeats the same split-model guidance and also mentions `Safe Storage`
  - Live runtime logs still emit macOS keychain/WebAuthn-adjacent errors such as:
    - `Touch ID authenticator unavailable because keychain-access-group entitlement is missing or incorrect`
  - The macOS entitlement file [native/mac/aegis.entitlements](/Users/deepsaint/Desktop/aegis/native/mac/aegis.entitlements) is empty, so there is no clean first-party entitlement story here either.
- Why this matters:
  - The intended product direction is one canonical Aegis-owned state path, not "Aegis plus some browser/keychain-shaped side channel."
  - Even when the remaining behavior is inherited from Chromium/CEF, users still experience it as Aegis behavior.
  - From a DX perspective, the presence of these terms in first-party docs means the old model has not actually been removed from the product's mental model.
- Current assessment:
  - This should be treated as an explicit product bug until:
    - Apple keychain / `Safe Storage` references are removed from first-party product docs
    - browser-managed credential storage is disabled or clearly fenced off from the canonical path
    - macOS runtime behavior stops surfacing keychain-adjacent prompts/errors in normal operation

### 43. The macOS desktop shell still presents itself as `aegis_native` instead of a polished `Aegis` app

- Severity: `P2`
- Surface: desktop packaging / identity / operator trust
- What happened:
  - A fresh headful runtime launched successfully on `127.0.0.1:7903` and reported healthy diagnostics.
  - A live screen capture of that runtime showed the menu bar app label as:
    - `aegis_native`
  - At the same time, a visible-process query did not surface a clean visible app named `Aegis`.
  - The installed bundle metadata at [Aegis.app/Contents/Info.plist](/Users/deepsaint/Applications/Aegis.app/Contents/Info.plist) includes `CFBundleIdentifier`, but the usual polished name fields such as `CFBundleName` were absent in the inspected plist.
- Why this matters:
  - This is a shipping product surface, not just an internal runtime detail.
  - Users should not feel like they are interacting with an internal host binary or dev artifact when they open the desktop browser.
  - The mismatch between product name, visible process identity, and on-screen branding makes the app feel unfinished even when the runtime itself is healthy.
- Current assessment:
  - The installed macOS app should present a consistent first-party identity everywhere:
    - bundle metadata
    - visible app/process naming
    - menu bar app name
    - on-screen shell branding

### 44. The headful app ships a visible new-tab button that is intentionally disabled

- Severity: `P2`
- Surface: desktop UX / affordance honesty / discoverability
- What happened:
  - The macOS shell creates a plus-shaped new-tab button in [native/aegis_browser_host_mac.inc](/Users/deepsaint/Desktop/aegis/native/aegis_browser_host_mac.inc).
  - That same source immediately marks it non-functional:
    - `[s.new_tab_button setEnabled:NO];`
    - `s.new_tab_button.alphaValue = 0.5;`
  - So the product renders a standard browser affordance that looks like a capability tease, not a working feature.
- Why this matters:
  - Users reasonably interpret a visible tab strip plus button as "multi-tab exists here."
  - Shipping the control in a permanently disabled state creates confusion instead of clarity.
  - This is especially awkward in a product that already talks about multi-context workflows elsewhere.
- Current assessment:
  - If tabs are not supported in the headful shell, the control should be removed entirely rather than displayed as a dead affordance.

### 45. Aegis leaves behind large numbers of orphaned helper processes after runtimes should already be gone

- Severity: `P1`
- Surface: lifecycle management / memory pressure / local host stability
- What happened:
  - Repeated Aegis use across fresh detached runtimes produced a steadily growing helper fleet in `ps`:
    - earlier in the pass: `renderer_helpers 27`, `gpu_helpers 10`, `utility_helpers 23`
    - later in the same pass, after spinning up fresh runtimes on new ports: `renderer_helpers 32`, `gpu_helpers 12`, `utility_helpers 27`
    - after additional scripted search-loop stress: `renderer_helpers 33`, `gpu_helpers 14`, `utility_helpers 29`, `runtime 15`
  - Those helpers were spread across many historical `~/.aegis/runtime/serve-headless/instances/...` directories rather than one obviously current runtime only.
  - Aggregate resident memory also remained materially large during the same audit window:
    - renderer helpers: about `1.5 GB` RSS combined
    - GPU helpers: about `413 MB` RSS combined
    - utility helpers: about `397 MB` RSS combined
    - runtime processes: about `856 MB` RSS combined
  - This means Aegis is at minimum accumulating process-heavy state across runs, and strongly suggests stale helper retention rather than a clean one-runtime-in one-runtime-out lifecycle.
- Why this matters:
  - Long-lived renderer/GPU/utility helper accumulation is a concrete resource-leak risk, not just subjective sluggishness.
  - On a machine used heavily for agent-driven automation, this will compound into:
    - memory pressure
    - process-table clutter
    - hard-to-debug runtime interference
    - reduced trust that a "fresh" run is actually fresh
- Current assessment:
  - Runtime shutdown needs a stricter process-reaping contract, and startup diagnostics should probably warn when the local machine already has stale Aegis helper fleets alive.

### 46. The CLI still underserves the actual automation surface, forcing deep scripting users to drop to raw HTTP instead of one canonical command interface

- Severity: `P2`
- Surface: scriptability / CLI DX / product packaging
- What happened:
  - The live `/manifest` route exposes a much richer automation surface than the shipped CLI presents directly, including:
    - `/execute`
    - `/contexts`
    - `/session`
    - `/downloads`
    - `/events`
    - `/trace/enable`
  - But the top-level CLI help still primarily exposes a page-by-page workflow:
    - `navigate`
    - `search`
    - `page ...`
    - `trace` replay
    - `config`
  - There is no first-class CLI subcommand for important runtime primitives like:
    - command-batch execution
    - named-context management
    - session snapshot/load/save
    - event streaming/inspection
  - In practice, this meant the deeper scripted audit had to keep falling back to raw HTTP calls against the local control plane rather than staying inside one canonical Aegis CLI workflow.
- Why this matters:
  - The product vision here is not just "manual browser helper"; it is an agentic browser that should support serious automation.
  - If advanced users and agents have to discover `/manifest` and hand-roll HTTP calls to access core primitives, the claimed one-binary local workflow is incomplete.
  - This is especially important for the user's stated direction that deeper research flows should be scriptable end to end.
- Current assessment:
  - The CLI should expose the core automation primitives that the local runtime already supports, so scripting users do not have to split their mental model between:
    - the branded CLI
    - undocumented or semi-hidden raw HTTP orchestration

### 47. Search completion still races the final result-page URL and semantic interpretation, which makes scripted research loops branch on unstable state

- Severity: `P2`
- Surface: search workflow reliability / deep-research scripting
- What happened:
  - On a fresh headless runtime, posting `/search` for `openai responses api docs` returned success with:
    - `url: https://duckduckgo.com/?q=openai+responses+api+docs`
  - An immediate page snapshot on that same page reported:
    - `url: https://duckduckgo.com/?q=openai+responses+api+docs`
    - `page_type: content`
    - suggestions including `Open the primary link "Protection. Privacy. Peace of mind."`
  - One second later, without issuing a new search:
    - the page URL had changed to `https://duckduckgo.com/?q=openai+responses+api+docs&ia=web`
    - `page_type` had changed to `documentation`
    - the suggested primary link had changed to an OpenAI developer-doc result
- Why this matters:
  - This is not just cosmetic page settling.
  - A scripted research loop that consumes the immediate result can branch into the wrong action path before the actual search-results interpretation is stable.
  - That makes Aegis less trustworthy for higher-level automation where the whole value proposition is to let the runtime drive multi-step research coherently.
- Current assessment:
  - `search` should not be considered semantically complete until the result page's canonical URL and research snapshot have stabilized enough for downstream automation to branch safely.

### 48. There are real `aegis_cli` crash reports on disk, and at least one points directly into cookie/session snapshot logic

- Severity: `P1`
- Surface: runtime stability / session handling / background reliability
- What happened:
  - macOS diagnostic reports currently exist for Aegis under:
    - `/Users/deepsaint/Library/Logs/DiagnosticReports/aegis_cli-2026-08-21-163731.ips`
    - `/Users/deepsaint/Library/Logs/DiagnosticReports/aegis_cli-2026-08-21-171348.ips`
  - The `2026-08-21 16:37:31` report shows:
    - `procName: "aegis_cli"`
    - `exception.type: "EXC_CRASH"`
    - `signal: "SIGABRT"`
    - `termination.indicator: "Abort trap: 6"`
    - `asi.libsystem_c.dylib: ["abort() called"]`
  - The captured stack for that crash includes a path through:
    - `CookieCollector::Visit`
    - `CefBridge::eval_js`
    - `AegisRuntime::refresh_live_state`
    - `AegisRuntime::snapshot_session`
    - `MainThreadContext::handle_command`
  - The same report also includes a thrown C++ length error in the cookie-collection path before the abort.
- Why this matters:
  - This is not just anecdotal “sometimes the runtime blows up.”
  - There is concrete OS-level evidence that `aegis_cli` has crashed in a code path tied to session/cookie snapshotting, which is one of the product’s canonical persistence/control features.
  - For a browser meant to be left running and reused by agents, aborting inside session-state handling is especially serious because it undermines one of the core long-lived-runtime promises.
- Current assessment:
  - Session/cookie snapshot paths need a direct stability pass and explicit regression coverage, because the production artifact has already emitted real crash reports in that area.

### 49. The installed `aegis` launcher rejects `--host-lib` overrides even though the CLI help still advertises `--host-lib` as a normal top-level option

- Severity: `P2`
- Surface: installer/launcher contract / CLI DX / packaging honesty
- What happened:
  - The installed launcher at [~/.local/bin/aegis](/Users/deepsaint/.local/bin/aegis) is a shell wrapper around the bundled app CLI, not the binary itself.
  - That wrapper explicitly rejects any `--host-lib` value that does not resolve to the installed production host library and exits with:
    - `The canonical aegis launcher only supports the installed production host library.`
  - A direct runtime check confirmed the behavior:
    - `aegis --host-lib /tmp/not-the-installed-host.dylib usage`
    - result: hard failure with exit code `2`
  - But the public CLI help still advertises `--host-lib` as a normal global option:
    - `Path to the native host library. By default serve uses the workspace release runtime and refreshes it when sources are newer.`
- Why this matters:
  - The issue is not that the production launcher wants guardrails; that can be reasonable.
  - The issue is that the installed command surface still presents `--host-lib` as if it were broadly supported, while the canonical launcher secretly narrows the contract.
  - For users and agents, that creates a confusing split between:
    - what `aegis --help` says is available
    - what the installed launcher will actually allow
- Current assessment:
  - The product should either:
    - stop advertising `--host-lib` on the canonical installed launcher path
    - or support the option consistently there with a clearer production-vs-dev mode model

### 50. Runtime diagnostics report `dom_snapshot_available: false` even while `GET /dom` is already returning a full DOM snapshot

- Severity: `P2`
- Surface: diagnostics truthfulness / operator recovery / agent planning
- What happened:
  - On both the default runtime and a freshly created named context, `/runtime` reported:
    - `inspectable_dom_ready: true`
    - `dom_snapshot_available: false`
    - a real `current_title` and completed document state
  - On those same contexts at the same time:
    - `GET /dom` returned a valid DOM tree
    - `GET /page` returned the expected page summary for `https://example.com/`
  - So the observable control-plane truth and the diagnostics flag disagreed.
- Why this matters:
  - Diagnostics are supposed to help agents and operators decide whether a capability is usable right now.
  - A false negative on DOM availability can trigger unnecessary retries, restarts, or degraded-path behavior even when the DOM surface is already working.
  - This is especially harmful in a system that leans heavily on `/doctor` and `/runtime` as operator trust surfaces.
- Current assessment:
  - `dom_snapshot_available` should reflect the real availability of `GET /dom`, not lag behind successful DOM reads.

### 51. The high-level page summary can claim `useful_text_available: false` on an obvious readable content page

- Severity: `P2`
- Surface: page research semantics / agent decision quality
- What happened:
  - On the plain `https://example.com/` page, `GET /page` returned:
    - `title: "Example Domain"`
    - `visible_text` with the expected body content
    - heading and link inventories
    - `page_type: "content"`
  - But that same response also marked:
    - `useful_text_available: false`
  - Meanwhile `GET /page/text?scope=full` returned the expected readable text for the same page.
- Why this matters:
  - This is a semantic contradiction inside one of the core high-level research surfaces.
  - Agents may use `useful_text_available` to decide whether to read, search again, or treat a page as effectively empty.
  - Mislabeling a simple content page as not having useful text can push automation into unnecessary fallback behavior.
- Current assessment:
  - The high-level usefulness flag should align with the actual readable text surface exposed by `/page` and `/page/text`.

### 52. Browser activation is exposed as a first-class route, but activating an already attached browser id can still fail with a protocol-level `502`

- Severity: `P2`
- Surface: multi-browser control plane / route reliability / manifest trust
- What happened:
  - A fresh named context `audit-c1` reported an attached browser id:
    - `attached_browser_ids: [2]`
  - Calling the documented activation route for that exact attached browser:
    - `POST /contexts/audit-c1/browsers/2/activate`
    returned HTTP `502`.
  - The returned body was:
    - `protocol error: unexpected message kind 9, expected 10`
    - with `code: "aegis_error"`
- Why this matters:
  - This is not a misuse of the route; the request targeted the one browser id the runtime itself said was attached to that context.
  - A protocol-kind mismatch on such a direct control-plane action undermines trust in the multi-browser API surface.
  - Combined with the dead new-tab affordance in the headful shell, this makes the browser-activation story feel exposed before it is truly dependable.
- Current assessment:
  - The activation route should be treated as unstable until it can successfully no-op or confirm activation of an already attached browser without protocol-level failure.

### 53. `events/live` advertises `type` filtering as `string[]`, but the common single-string form fails deserialization with a raw `400`

- Severity: `P2`
- Surface: SSE event-stream DX / query contract / manifest correctness
- What happened:
  - The manifest and route docs describe the `type` query for `/events/live` as a filter such as:
    - repeated or comma-separated event categories
    - manifest kind: `string[]`
  - The backing query type in [src/api/server.rs](/Users/deepsaint/Desktop/aegis/src/api/server.rs) is:
    - `event_types: Vec<String>` with `rename = "type"`
  - A normal request using a single event type:
    - `GET /events/live?since=314&poll_ms=100&type=navigation`
    failed with HTTP `400` and a plain-text body:
    - `Failed to deserialize query string: type: invalid type: string "navigation", expected a sequence`
- Why this matters:
  - A single filter value is an extremely natural caller shape, especially for curl-based debugging and agent-generated requests.
  - Returning a raw query-deserializer error instead of accepting the value or normalizing it into a one-element list makes the stream surface more brittle than the docs imply.
  - This is also inconsistent with the product’s broader goal of giving agents ergonomic high-level control surfaces rather than parser footguns.
- Current assessment:
  - The route should accept the common single-string form for `type`, or the docs/manifest must stop implying a more permissive query contract than the server actually honors.

## Remaining Follow-Ups

### 54. The browser surface has no zoom capability at all

- Severity: `P2`
- Surface: headful browser usability / accessibility / production-browser completeness
- What happened:
  - A local source pass found no zoom command, route, CLI subcommand, or native host wiring for page zoom.
  - Searching the repo for browser zoom hooks did not reveal any use of CEF/browser zoom APIs.
  - The headful shell supports back, forward, and reload, but there is no comparable zoom-in, zoom-out, or reset path.
- Why this matters:
  - Zoom is a baseline browser affordance for readability, accessibility, demos, and debugging across modern SaaS apps.
  - Without it, users and agents lose an important recovery tool on dense UIs, high-DPI displays, and small-text enterprise products.
  - For a product positioning itself as a serious browser runtime, complete absence of zoom is a noticeable capability gap.
- Current assessment:
  - This appears to be a real missing browser feature, not just a docs gap.

### 55. The headful shell visually suggests tabs, but new-tab behavior is not actually implemented

- Severity: `P2`
- Surface: headful browser product completeness / affordance trust
- What happened:
  - The macOS browser host renders a tab strip and a visible plus button with the accessibility description `New tab`.
  - That same button is explicitly disabled in the native UI code.
  - The product docs also describe the supported runtime model as one browser session per runtime, reinforcing that there is no real multi-tab workflow behind the chrome.
- Why this matters:
  - Showing a standard browser tab affordance that cannot be used creates expectation debt immediately.
  - Multi-tab work is a normal part of production browsing for research, auth flows, cross-checking state, and operator debugging.
  - Even if single-session is the intentional runtime model, the current UI reads as "browser with tabs" more than "single-session agent shell."
- Current assessment:
  - This is a genuine product gap and also an affordance-mismatch bug: the shell advertises a browser behavior it does not actually provide.

### 56. The advertised "one execute batch" model is not actually true once `wait_for` or `media_state` appear in the command list

- Severity: `P1`
- Surface: batch execution contract / runtime semantics / selling-point integrity
- What happened:
  - The docs describe `POST /execute` as one command batch sent through the runtime in one go.
  - The executor does not preserve that model when a batch contains `wait_for` or `media_state`.
  - Instead, it:
    - flushes pending bridge commands
    - runs `wait_for` or `media_state` separately in host-side control flow
    - then resumes later commands in another internal flush
  - So a single user-visible execute request can become multiple internal execution phases and multiple bridge/eval interactions.
- Why this matters:
  - Batch semantics are part of the product story here, not just an implementation detail.
  - Agents may rely on "single ordered batch" as a latency, consistency, or atomicity assumption when composing action pipelines.
  - If the runtime silently splits the batch, timing and failure behavior can diverge from what the API contract implies.
- Evidence from source:
  - `src/runtime/executor.rs` special-cases `Command::WaitFor` and `Command::MediaState` and flushes pending commands before and after them instead of keeping one bridge batch.
- Current assessment:
  - This is a real contract mismatch between the documented batch model and the implemented runtime behavior.

### 57. Batch execution is fail-open: later commands still run after earlier commands fail

- Severity: `P1`
- Surface: multi-action safety / production correctness / destructive workflow risk
- What happened:
  - The batch path does not stop on first failure.
  - If one command errors, the runtime still continues evaluating later commands in the same batch.
  - There is no visible `abort_on_error`, transactional mode, or explicit partial-failure guard in the command model.
- Why this matters:
  - In production browser automation, a failed early step often invalidates the assumptions behind every later step.
  - Continuing after failure can turn a recoverable targeting problem into data corruption, wrong-form submission, or interactions against the wrong page state.
  - This is especially risky because batch execution is marketed as a strength, which encourages agents to compose more work per request.
- Evidence from source:
  - `assets/js/aegis_runtime.js` batch `exec(commands)` maps over every command independently and returns a result per command instead of failing fast.
  - `native/aegis_app.cc` likewise loops over every command in the request and keeps building `results_json` even after individual command failures.
  - `src/runtime/executor.rs` preserves per-command errors in-place but continues assembling the remainder of the batch.
- Current assessment:
  - This is a product-level safety gap, not merely an implementation preference.

### 58. Multi-command batches lose the per-command event attribution that the executor expects for interaction diagnostics

- Severity: `P1`
- Surface: batch observability / trust in click-and-submit diagnostics
- What happened:
  - The browser runtime script has an `exec(commands)` helper that annotates each successful command result with `_aegis.event_span`.
  - The Rust executor explicitly looks for that span and uses it to attribute emitted events and network activity back to the specific command.
  - But the native `send_batch` implementation does not call `window.__aegis.exec(commands)`.
  - It calls `window.__aegis.click(...)`, `setValue(...)`, `pressKey(...)`, `drag(...)`, and related helpers one by one directly.
  - That means the `_aegis.event_span` metadata the executor expects is not actually present on normal multi-command native batch results.
- Why this matters:
  - This quietly weakens the core selling point of higher-confidence interaction diagnostics.
  - In multi-command batches, click and keypress results can lose accurate per-command:
    - emitted event attribution
    - network request attribution
    - submit outcome attribution
    - prior-event sequencing context
  - The system can therefore look more confident than it really is while giving under-scoped diagnostics.
- Evidence from source:
  - `assets/js/aegis_runtime.js` adds `_aegis.event_span` only inside `exec(commands)`.
  - `src/runtime/executor.rs` extracts `_aegis.event_span` through `extract_command_event_span(...)`.
  - `native/aegis_app.cc` bypasses `exec(commands)` and individually invokes helper functions per command.
- Current assessment:
  - This looks like a real implementation bug in the diagnostic plumbing for normal multi-command native batches.

### 59. A credential-capture preflight can fail the entire execute request before any batch command runs

- Severity: `P2`
- Surface: batch reliability / unrelated feature coupling
- What happened:
  - For execute requests containing `set_value` or `click`, the API layer may attempt a pre-execution DOM snapshot for credential auto-store.
  - If that snapshot call fails, the whole execute request returns the error immediately and none of the commands run.
  - This happens even when the user’s actual goal is unrelated to credential persistence.
- Why this matters:
  - Batch execution should not become more fragile because an auxiliary feature wants a preflight snapshot.
  - This couples ordinary multi-action workflows to credential-capture readiness in a way that can create surprising all-or-nothing failure.
  - For production automation, observability or capture helpers should degrade gracefully, not block unrelated action batches outright unless explicitly required.
- Evidence from source:
  - `src/api/server.rs` captures a pre-execution snapshot before running matching command batches when auto-store is enabled.
  - On snapshot failure, it records the execute failure and returns early instead of running the batch.
- Current assessment:
  - This is a real batch-path reliability problem caused by cross-cutting feature coupling.

### 60. Trace and replay flatten multi-phase execute behavior into one synthetic batch, which can hide the real runtime shape

- Severity: `P2`
- Surface: trace fidelity / debugging accuracy / deterministic analysis
- What happened:
  - The executor synthesizes one `BatchResponse` and one trace record for the whole user-visible execute call.
  - But some execute requests are internally split across:
    - one or more bridge flushes
    - host-side `wait_for`
    - later bridge flushes
  - The resulting trace loses the sub-batch boundaries and does not clearly preserve the actual execution phases that occurred.
- Why this matters:
  - Trace quality is part of the production confidence story.
  - If the recorded artifact compresses a multi-phase batch into one logical response, replay and diagnosis can miss the exact point where timing, state drift, or failure semantics changed.
  - This is especially relevant for debugging batch-action flakiness, where phase boundaries often matter.
- Evidence from source:
  - `src/runtime/executor.rs` builds a single synthetic `BatchResponse` at the end of `execute_command_stream(...)` even when the command stream was split internally.
  - The trace recorder persists that synthesized request/response shape as one batch record.
- Current assessment:
  - This is a trace-fidelity gap that makes batch-path debugging weaker than it appears.

### 61. The visible test surface does not appear to exercise real multi-command batch behavior end to end

- Severity: `P2`
- Surface: verification coverage / confidence in a flagship feature
- What happened:
  - The visible test coverage around batching is strong on:
    - wire encoding
    - protocol round-tripping
    - trace persistence
    - replay semantics
  - But the source pass did not reveal equivalent end-to-end verification for the most important real batch behaviors:
    - partial failure handling
    - fail-fast vs continue-on-error semantics
    - per-command event attribution in multi-command requests
    - `wait_for` splitting behavior inside one execute request
    - mixed command batches such as `set_value -> click -> press_key -> wait_for`
- Why this matters:
  - Batch execution is being treated as a major capability and selling point.
  - If the tests mostly prove static wire shape while missing live multi-command semantics, the quality signal will overstate how safe the feature is in production.
  - This kind of gap is exactly how subtle action-ordering bugs survive until a real SaaS workflow exposes them.
- Evidence from source:
  - `tests/runtime_flow.rs` strongly covers encoding/decoding and trace artifacts.
  - The visible test pass does not show comparable end-to-end assertions for live multi-command execution semantics.
- Current assessment:
  - This is a verification gap around one of the product’s most important advertised behaviors.

### 62. Aegis verification still behaves too much like a manual probe and not enough like a first-class acceptance layer

- Severity: `P2`
- Surface: browser verification workflow / regression confidence
- What happened:
  - In the pasted field report, Aegis was useful for checking rendered links, profile navigation, and credits, but the workflow still felt like:
    - open page
    - inspect visually
    - maybe capture a screenshot
    - manually decide whether the user flow "looked right"
  - The natural workflow did not automatically produce a durable, repeatable acceptance artifact unless the operator explicitly built that around Aegis.
- Why this matters:
  - A serious browser verification product should not rely on operator memory or ad hoc screenshot habits for high-value flows.
  - If the tool is strongest only as a manual inspection assistant, it underdelivers on regression confidence and repeatability.
  - This becomes especially costly on flows that are visually convincing but semantically wrong.
- Distinction from existing issues:
  - This is broader than the current "high-confidence verification" gap.
  - The problem here is not just missing assertions inside a session, but that Aegis is not yet productized as a repeatable acceptance-test layer in its own ecosystem.
- Current assessment:
  - This is a real productization gap around how Aegis is meant to be used for verification at scale.

### 63. The repo does not appear to provide a first-class Aegis scenario suite for core browser-user flows

- Severity: `P2`
- Surface: verification product completeness / dogfooding gap
- What happened:
  - The pasted report highlights that Aegis is available as a live verification tool, but not as a clearly integrated regression suite for the kinds of user journeys it is supposed to validate.
  - The current repo has strong Fozzy and runtime-test surfaces, but not an obvious first-class Aegis-owned scenario catalog for:
    - homepage browsing
    - route transitions
    - profile links
    - copy-link UX
    - media/playback checks
    - layout and scroll behavior
- Why this matters:
  - "Aegis first" is much stronger when the product can verify itself through stable, scripted browser scenarios.
  - Without that layer, browser verification remains too dependent on one-off operator runs and local interpretation.
  - This also weakens the credibility of Aegis as a production browser-verification system rather than just an interactive tool.
- Distinction from existing issues:
  - Entry `61` is about missing end-to-end semantic coverage for command batches.
  - This issue is about missing first-class product-level scenario coverage for real browser flows.
- Current assessment:
  - This is a meaningful dogfooding and verification-strategy gap.

### 64. Aegis verification is too vulnerable to dirty backend or cached application state

- Severity: `P1`
- Surface: browser verification truthfulness / false-confidence risk
- What happened:
  - The pasted report calls out a recurring problem in browser verification work:
    - the UI can appear correct because of previously mutated backend state
    - cached data
    - leftover seeded content
    - prior test/session side effects
  - In that mode, Aegis may "verify" a page successfully even though the current migration, seed, route, or API path is actually broken.
- Why this matters:
  - This is a classic false-positive failure mode for browser automation.
  - A visually correct page is not strong evidence if the environment was not freshly and deterministically prepared.
  - Because Aegis often sits at the end of the stack, it needs stronger conventions or support for proving that the verified state came from the intended setup path.
- Distinction from existing issues:
  - Existing entries cover interaction confidence and persisted app state after a click.
  - This issue is about environment cleanliness and backend truth contaminating browser verification before the interaction even begins.
- Current assessment:
  - This is a real verification-integrity problem and should be treated as high severity because it can create false trust in broken systems.

### 65. Backend/API failures can remain invisible to Aegis when the page still renders or falls back cleanly

- Severity: `P1`
- Surface: browser verification blind spots / masked backend failure
- What happened:
  - The pasted report notes that Aegis did not naturally expose backend API errors unless those failures became visible in-page.
  - If the UI:
    - fell back silently
    - rendered cached state
    - partially hydrated
    - or otherwise masked the failure
    Aegis could produce a superficially successful browser verification result while the backend route or API contract was broken.
- Why this matters:
  - Browser verification alone is not enough if it cannot make backend breakage legible when the UI degrades gracefully.
  - This can produce especially dangerous "pass" results on regressions where the user sees something plausible but the data path is wrong.
  - A productized browser verifier should make it easier to correlate visible UI state with underlying request/error truth.
- Distinction from existing issues:
  - This is related to, but distinct from, missing high-confidence verification.
  - The emphasis here is masked backend failure, not just ambiguous UI success.
- Current assessment:
  - This is a serious verification blind spot that can inflate trust in passing browser checks.

### 66. Layout-heavy interactions such as horizontal row scrolling are still awkward to verify deterministically with Aegis

- Severity: `P2`
- Surface: visual/layout verification / scroll-state confidence
- What happened:
  - The pasted report specifically calls out horizontally scrolling rows, fade edges, arrow controls, and density/overlap behavior as awkward to verify "purely by eye."
  - These UI patterns are vulnerable to:
    - viewport-specific breakage
    - pointer-event blocking
    - overlay/fade interference
    - bunching on smaller screens
    - visually subtle regressions that are easy to miss during a manual pass
- Why this matters:
  - This is exactly the kind of product surface where browser verification should be strong and reproducible.
  - If these flows still depend on visual eyeballing instead of repeatable viewport assertions and artifact capture, layout regressions are likely to slip through.
  - It also limits Aegis’s usefulness for validating modern media-heavy or carousel-heavy frontends.
- Distinction from existing issues:
  - Existing entries already cover some general product discoverability and verification gaps.
  - This one is specifically about the weak ergonomics for deterministic layout/scroll verification of visually complex browser flows.
- Current assessment:
  - This is a real gap in the current browser-verification workflow and worth tracking separately because layout regressions are common and expensive.

### 67. `page actions` can omit the actually relevant visible controls from the primary control set

- Severity: `P2`
- Surface: page research prioritization / control discovery
- What happened:
  - In the reported row-scroll validation flow, `page actions` surfaced hero episode selectors and content links as primary controls.
  - The scroll-arrow buttons that were actually needed for the verification task were not surfaced as primary controls, even though `page find "Scroll right"` could find them.
- Why this matters:
  - This makes control discovery inconsistent across Aegis’s own high-level page primitives.
  - If `page actions` is meant to summarize the most relevant next steps, omitting the visible control the operator actually needs is a serious relevance failure.
  - It is especially harmful in UI QA workflows where the purpose is not semantic page reading but validating a specific interactive affordance.
- Evidence from source:
  - The page-research layer clearly has enough control metadata to find such buttons.
  - The control-ranking behavior in `assets/js/aegis_runtime.js` appears tuned toward general semantic relevance rather than task-oriented UI QA control discovery.
- Current assessment:
  - This is a real ranking/product bug in the `page actions` surface.

### 68. Aegis exposes no clean high-level action path from found controls to actual button activation

- Severity: `P1`
- Surface: page-find/action workflow / CLI ergonomics / product completeness
- What happened:
  - `page find` can return repeated control matches such as `Scroll right` along with indices.
  - But the visible CLI workflow does not expose an obvious symmetric primitive for acting on those controls the way `page open-link` acts on links.
  - `page open-link` is link-oriented and therefore not the right tool for button controls like scroll arrows.
  - There is no obvious high-level `page click` / `click-control` workflow in the visible CLI help.
- Why this matters:
  - Aegis can successfully discover a control but still leave the operator stranded with no first-class way to act on it.
  - That creates a broken workflow loop:
    - find
    - inspect
    - manually fall back to lower-level execution
  - For production UX, discovery and action need to compose naturally.
- Evidence from source:
  - CLI help and docs expose `page actions`, `page find`, and `page open-link`, but not a similarly direct page-level button/control activation primitive.
- Current assessment:
  - This is a real control-surface gap, not just a documentation omission.

### 69. Control indexing is inconsistent across `page find`, `page actions`, and the visible command surface

- Severity: `P2`
- Surface: result reuse / API ergonomics / operator trust
- What happened:
  - `page find` returns match indices.
  - `page actions` returns control inventories with their own indices.
  - But the visible high-level command surface does not make it clear how those indices are meant to be reused for direct follow-up actions.
  - That leaves index-bearing outputs feeling informative but operationally incomplete.
- Why this matters:
  - Indexed outputs should make follow-up actions easier, not create ambiguity about which index belongs to which command family.
  - In repeated-control UIs like carousels or toolbars, index reuse is often the only reliable way to target the intended instance.
  - If index semantics are not clearly reusable, users are pushed back down to manual interpretation or low-level execute payloads.
- Evidence from source:
  - `PageFindMatch` includes `index`, and `PageResearchControl` is also index-based, but the visible CLI help does not expose a matching first-class action primitive that clearly consumes those indices.
- Current assessment:
  - This is a real API/CLI contract sharp edge and worth tracking separately from the missing click-control primitive itself.

### 70. `page find` does not expose enough control-state detail for production UI QA

- Severity: `P2`
- Surface: inspection fidelity / UI QA workflow
- What happened:
  - The reported workflow needed to answer questions like:
    - is this control enabled
    - is it visible
    - is it in viewport
    - what role does it have
  - But `page find` currently returns a lighter match shape centered on kind/text/snippet/index rather than the richer control-state details already present elsewhere in page research.
- Why this matters:
  - UI QA often depends on operational state, not just semantic match text.
  - A match result that says "control found" is incomplete if it cannot also answer whether that control is presently actionable.
  - This is especially important for carousel arrows, hidden controls, disabled pagination, sticky overlays, and responsive layouts.
- Evidence from source:
  - `PageFindMatch` is currently a slim shape.
  - `PageResearchControl` already carries richer fields such as role, disabled state, and viewport status in the runtime data model.
- Current assessment:
  - This is a real inspection-surface mismatch: the data exists, but `page find` does not expose enough of it for strong UI QA workflows.

### 71. `page actions` lacks filtering primitives needed for focused QA work

- Severity: `P2`
- Surface: high-level CLI usability / page research targeting
- What happened:
  - The reported workflow needed a way to narrow controls by text/role/scope/all-controls style filters.
  - The visible `page actions` CLI surface is summary-oriented and does not appear to offer focused filtering such as:
    - text filter
    - role filter
    - scope filter
    - "show all controls"
  - As a result, task-specific controls can be buried or omitted by the summarization/ranking layer.
- Why this matters:
  - Browser QA and debugging often require interrogating a narrow subset of controls rather than receiving a global "most relevant" summary.
  - Without filtering, `page actions` is less useful precisely when the page has many semantically interesting but operationally irrelevant controls.
- Evidence from source:
  - The visible CLI page subcommands do not expose filtering options for `page actions`.
- Current assessment:
  - This is a real product gap in the high-level page-inspection workflow.

### 72. The CLI does not expose first-class screenshot, viewport, or assertion workflows for browser QA

- Severity: `P2`
- Surface: CLI product completeness / frontend verification ergonomics
- What happened:
  - The reported workflow wanted obvious primitives for:
    - screenshots
    - viewport sizing
    - small assertions
    - DOM/query checks shaped like test output
  - The visible CLI help does not surface first-class page subcommands for screenshot capture, viewport control, or assertion-style frontend QA recipes.
- Why this matters:
  - These are baseline browser-verification primitives for responsive layout checks, scroll validation, modal checks, auth-state checks, and before/after comparison work.
  - Their absence reinforces the sense that Aegis is still optimized more for manual exploratory control than for repeatable browser QA.
  - This also compounds existing verification issues because operators must build these capabilities manually around lower-level surfaces.
- Distinction from existing issues:
  - Existing entries cover broad verification productization gaps.
  - This issue is specifically about missing first-class browser-QA CLI primitives.
- Current assessment:
  - This is a real CLI capability gap and not just a documentation polish issue.

### 73. The page-oriented CLI output is useful, but it is not shaped enough like stable test output for automation

- Severity: `P2`
- Surface: CLI automation readiness / schema stability / machine-consumption UX
- What happened:
  - The reported workflow found the JSON helpful, but not shaped enough like test output for clean automation.
  - The visible page workflows also do not emphasize stable, assertion-friendly schemas for all page subcommands in a way that naturally supports production QA scripting.
- Why this matters:
  - Browser verification becomes much easier to automate when outputs are clearly designed for machine comparison:
    - stable schemas
    - predictable fields
    - test-friendly summaries
    - explicit pass/fail-oriented state
  - If the outputs are primarily human-inspection oriented, teams are pushed toward ad hoc parsers and brittle wrappers.
- Distinction from existing issues:
  - This is narrower than the general "manual probe" complaint.
  - The issue here is specifically that the output contract is not yet shaped like a first-class test artifact surface.
- Current assessment:
  - This is a real automation/DX gap worth tracking separately from missing page-level assertion commands.

### 74. The HTTP control API is too hidden relative to how essential it is for real automation

- Severity: `P2`
- Surface: product discoverability / automation entrypoint clarity
- What happened:
  - The pasted report had to manually discover the HTTP control plane by:
    - locating the `aegis` binary
    - inspecting CLI help
    - inferring that `aegis serve` exposed a local API
    - probing `/`, `/manifest`, `/execute`, `/page`, and `/navigate`
  - The CLI presentation remained command-oriented even though the final serious automation path depended directly on the HTTP API.
- Why this matters:
  - For repeatable automation and CI-style use, the HTTP API is not a side path; it is the core programmable surface.
  - If that surface is not obvious, users waste time rediscovering the real control plane instead of using it confidently.
  - This also makes Aegis feel more manual than it really is.
- Distinction from existing issues:
  - This is more specific than the general product discoverability complaint.
  - The issue here is that the programmable API surface is under-signaled despite being essential.
- Current assessment:
  - This is a real DX/product-positioning gap around the primary automation interface.

### 75. `/execute` value serialization is inconsistent enough that primitive returns are not trustworthy

- Severity: `P1`
- Surface: `/execute` contract correctness / automation reliability
- What happened:
  - The pasted report found that returning a primitive string from evaluated JS did not behave reliably enough to use directly.
  - Wrapping the same value in an object produced a more usable result shape.
  - That forced the automation to adopt a workaround pattern of object-wrapping simple values.
- Why this matters:
  - `/execute` is a foundational primitive.
  - If strings, numbers, booleans, `null`, arrays, and objects do not round-trip consistently, test code becomes defensive and brittle immediately.
  - A browser control substrate should not make basic return-value handling surprising.
- Distinction from existing issues:
  - Existing entries cover batch semantics and error handling.
  - This one is specifically about the value contract of JS evaluation results.
- Current assessment:
  - This is a real correctness/DX issue in the `/execute` response contract.

### 76. Async page evaluation does not clearly support awaited promise results and can collapse into `{}` silently

- Severity: `P1`
- Surface: `/execute` async semantics / runtime evaluation correctness
- What happened:
  - The pasted report attempted an async IIFE with awaited delays and a returned object.
  - Aegis returned `{}` instead of the resolved payload.
  - The operator had to split one logical async interaction into multiple synchronous execute calls with sleeps outside Aegis.
- Why this matters:
  - Async page evaluation is a normal need for real browser automation, especially around:
    - delayed pointer sequences
    - framework state settling
    - timed DOM checks
    - animation or transition-aware logic
  - Returning `{}` instead of a clear resolved value or explicit unsupported-error message makes the failure mode ambiguous and time-consuming to debug.
- Distinction from existing issues:
  - This is related to missing wait/assert ergonomics, but it is a lower-level runtime contract bug.
  - The problem here is not just "there is no helper," but that async eval behavior appears misleading.
- Current assessment:
  - This looks like a real `/execute` runtime contract bug and should be treated as high priority.

### 77. Native console/network/error inspection is not exposed clearly enough as a first-class debugging surface

- Severity: `P2`
- Surface: debugging observability / browser-failure diagnosis
- What happened:
  - The pasted report notes that if the app had failed, there was no obvious Aegis-native path for quickly reading:
    - browser console logs
    - failed network requests
    - JS errors
    - request/response-level debugging information
  - The automation succeeded, so this did not block the run, but it would have made failure triage much slower.
- Why this matters:
  - Browser verification is much more trustworthy when page failures can be correlated with console and network truth without leaving the Aegis workflow.
  - Missing or hidden debug surfaces increase the chance that operators misdiagnose browser issues as app issues or vice versa.
- Evidence from source:
  - The runtime contains DevTools-network plumbing, but the visible CLI/API workflow does not make these debugging surfaces feel first-class.
- Current assessment:
  - This is a real observability/DX gap between what the runtime may know internally and what the user can easily access.

### 78. Aegis still lacks an obvious blessed helper-library or test-runner pattern for repeatable E2E work

- Severity: `P2`
- Surface: E2E authoring ergonomics / ecosystem maturity
- What happened:
  - The pasted report ended up building a small custom framework around:
    - request helpers
    - execute wrappers
    - polling
    - click helpers
    - drag helpers
  - That means the first serious test required creating a mini client/runtime wrapper before the actual product assertions even began.
- Why this matters:
  - A browser automation product becomes much more useful when there is one obvious blessed path for repeatable test authorship.
  - Without that, every user reinvents:
    - transport wrappers
    - wait semantics
    - selector helpers
    - result normalization
  - This slows adoption and fragments best practices.
- Distinction from existing issues:
  - Existing entries cover missing CLI assertions and missing scenario suites.
  - This issue is specifically about the absence of a clear helper-library/test-runner pattern for using the API directly.
- Current assessment:
  - This is a meaningful product-maturity gap in the Aegis testing story.

- Verify install and launcher behavior end to end, especially whether the bundled CLI becomes the obvious canonical entrypoint for users and agents.
- Keep probing credential auto-store and auto-replay on more modern login flows, because the product direction depends heavily on that path feeling automatic and trustworthy.
- Keep checking semantic/page-action stability on reactive apps, since the current research-layer races already proved the manifest is overstating reliability.
