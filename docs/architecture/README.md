# Aegis Architecture Plate

The diagram is an engineering view of `../../aegis-architecture.json`, with source checks for process boundaries and trace ownership. Open `aegis-architecture.svg` for editable vectors, `aegis-architecture.png` for a 5120-pixel-wide image, or `aegis-architecture.pdf` for a scalable, single-page document.

The five-module command path makes Rust routing, runtime execution, transport, native hosting, and renderer execution readable in one pass. The outer boundary deliberately includes both Rust and the native shared library. CEF renderer IPC crosses the separate process boundary. Selected ownership paths and supporting state are shown below the command path; this is a logical architecture plate, not a complete call graph or a literal thread schedule.

## Rebuild

From the repository root, with Python 3 and `rsvg-convert` installed:

```sh
python3 scripts/render_architecture.py
python3 scripts/render_architecture.py --verify
```

The source JSON hash is embedded in the SVG. Verification detects source drift and missing graph nodes. The layout and concise labels are curated; review them against changed architecture content before rebuilding. The exporter does not change the input JSON or runtime source.

## Verification

The SVG and PDF were visually inspected. The PDF text-bound check found no overlapping or out-of-bounds words, and the PNG is nonblank at 5120 by 3680 pixels. All 16 source graph nodes have visible SVG elements.

Run the declared deterministic fixture first, then the unmocked host scenario:

```sh
fozzy doctor --deep --scenario tests/aegis_architecture.fozzy.json --runs 5 --seed 424242 --json
fozzy test --det --strict-verify tests/aegis_architecture.fozzy.json --json
fozzy run docs/architecture/verify-host.fozzy.json --det --strict-verify --seed 424242 --proc-backend host --fs-backend host --http-backend host --record .fozzy/aegis/architecture-plate-host.trace.fozzy --json
fozzy trace verify .fozzy/aegis/architecture-plate-host.trace.fozzy --strict --json
fozzy replay .fozzy/aegis/architecture-plate-host.trace.fozzy --json
fozzy ci .fozzy/aegis/architecture-plate-host.trace.fozzy --json
```

The installed CLI's `test` command requires `--strict-verify` instead of the removed `--strict` flag. Its strict doctor/test preflight requires `proc_when` declarations even with host flags; the fixture checks the deterministic execution contract. The separate host `run` deliberately omits `proc_when` so it executes the Python verifier against the real exported files. These checks validate diagram artifacts, not browser runtime behavior.

## Design Philosophy

**Execution as structure.** Position communicates causality: control enters at the upper left, follows the aligned request path, and reaches the page at the right. Supporting detail sits beneath its owning module. Process and language boundaries are distinct concepts and receive distinct visual treatments.

**Color as ownership.** Blue belongs to Rust, green to native CEF, and plum to the renderer and page. White and cool neutral gray supply the field. Accent color encodes the system rather than decorating it. No gradients or decorative illustrations compete with the relationships.

**Typography as hierarchy.** Large, restrained sans-serif headings name the system and its modules. Compact monospace labels identify source files, commands, protocols, and API surfaces. Spacing separates levels of detail so the plate remains useful both as a whole and at inspection scale.

**Lines as contracts.** Solid arrows show requests or explicitly labeled ownership paths. The dashed return path summarizes results, DOM changes, and events flowing back into Rust. In-process FFI and cross-process CEF messages are visibly distinguished. Persistence sits outside the live process field.

**Evidence before symmetry.** Every graph node in the source JSON is represented. Related concepts may share a module, but distinct trace formats remain distinct. The diagram favors accurate relationships even when a more symmetric composition would be simpler.

## First-Principles Review

- **Reality:** The persistent Rust server dynamically loads the CEF host. The renderer injects the page runtime and exchanges process messages with the browser process.
- **Interpretation:** A control path plus explicit ownership regions explains the architecture more clearly than a directory tree.
- **Contradictions:** The JSON's Fozzy-to-Trace-File graph edge combines two different trace concepts. Its detailed trace model and the actual recorder show that Aegis writes JSON, while Fozzy records scenario traces. These have separate destinations here.
- **What The Lamp Reveals:** The important architectural units are execution and state ownership, not merely implementation languages.
- **Better Abstraction:** One persistent control surface spanning an in-process native bridge and an out-of-process semantic page runtime.
- **What Is Proven:** The diagram's 16 input graph nodes are represented; the SVG carries a matching source hash. `src/api/server.rs`, `src/host/mod.rs`, `native/include/aegis_host_abi.h`, `native/aegis_messages.h`, `native/aegis_app.cc`, and `src/trace/recorder.rs` support the boundary distinctions.
- **What Is Still Unknown:** This static plate does not prove correctness on arbitrary websites or enumerate every Chromium subprocess.
- **Next Experiments:** Re-render and inspect after architecture changes; extend with a sequence diagram when a particular operation needs thread-level detail.
- **Honest Thesis:** This is a source-backed logical system map with selected runtime paths, suitable for engineering communication and further graphics work.

## Coverage Notes

The input's `http_api` and `context_manager` share the first request module. `native_host_library` contains `cef_host` in the native module. `cef_browser` is below that host; the IPC request arrow from the module summarizes the host's use of its main frame. All remaining graph nodes are individually labeled. The native host's DevTools observer is shown adjacent to its browser ownership path.

The API's local JSON/SSE responses are separate from internal CEF messages. The bottom state band is canonical Aegis persistence, including the `AEGIS_HOME` override. The two trace columns distinguish Aegis JSON reconstruction from Fozzy scenario record/verify/replay/CI. Fozzy verification is outside the production request path.
