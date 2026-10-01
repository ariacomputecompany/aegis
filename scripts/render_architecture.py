#!/usr/bin/env python3
"""Render the source-backed Aegis architecture plate as SVG, PNG, and PDF."""

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import xml.etree.ElementTree as ET


ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "aegis-architecture.json"
OUTPUT = ROOT / "docs" / "architecture"
W, H = 2560, 1840
NS = "http://www.w3.org/2000/svg"
ET.register_namespace("", NS)
C = {
    "paper": "#FAFBFC", "white": "#FFFFFF", "ink": "#182128",
    "muted": "#55616C", "line": "#CED6DD", "subtle": "#E7ECF0",
    "rust": "#255BC5", "rust_tint": "#F0F5FF",
    "native": "#147568", "native_tint": "#EEF7F3",
    "page": "#9A3A69", "page_tint": "#FCF1F6",
    "storage": "#825D21", "storage_tint": "#F8F5EC",
}


class Plate:
    def __init__(self, architecture):
        self.architecture = architecture
        self.root = ET.Element(f"{{{NS}}}svg", {
            "width": str(W), "height": str(H), "viewBox": f"0 0 {W} {H}",
            "role": "img", "aria-labelledby": "title description",
        })
        self.add("title", id="title").text = "Aegis: persistent browser architecture"
        self.add("desc", id="description").text = (
            "Source-backed architecture diagram. Agents and the aegis CLI send local HTTP "
            "requests to a persistent Rust server. The context manager serializes work "
            "through AegisRuntime and CefBridge. A C ABI connects to the native CEF host "
            "in the same process. CEF IPC connects the browser to its renderer, where "
            "window.__aegis inspects and acts on the real page DOM. Aegis JSON state, "
            "Aegis runtime traces, and Fozzy scenario traces have separate ownership."
        )
        self.add("metadata", id="source-provenance").text = json.dumps({
            "source": "../../aegis-architecture.json",
            "sha256": hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
            "source_date": architecture["schema"]["generated_from"]["date"],
            "graph_node_ids": [n["id"] for n in architecture["graph_model"]["nodes"]],
            "clarifications": {
                "process_boundary": "C ABI is in-process; CefProcessMessage crosses into renderer.",
                "trace_formats": "Fozzy records scenario traces; Aegis TraceRecorder writes JSON.",
            },
        }, sort_keys=True)
        defs = self.add("defs")
        for name in ("ink", "muted", "rust", "native", "page", "storage"):
            marker = self.add("marker", parent=defs, id=f"arrow-{name}",
                              viewBox="0 0 10 10", refX="9", refY="5",
                              markerWidth="7", markerHeight="7", orient="auto-start-reverse")
            self.add("path", parent=marker, d="M 1 1 L 9 5 L 1 9 Z", fill=C[name])
        self.nodes = set()

    def add(self, tag, parent=None, **attrs):
        return ET.SubElement(self.root if parent is None else parent, f"{{{NS}}}{tag}",
                             {k.replace("_", "-"): str(v) for k, v in attrs.items()})

    def rect(self, x, y, w, h, fill, stroke="none", radius=0, **attrs):
        return self.add("rect", x=x, y=y, width=w, height=h, rx=radius,
                        fill=C.get(fill, fill), stroke=C.get(stroke, stroke), **attrs)

    def text(self, x, y, value, size=20, color="ink", weight=400, mono=False,
             anchor="start", **attrs):
        font = "Menlo, Consolas, monospace" if mono else "Helvetica Neue, Helvetica, Arial, sans-serif"
        node = self.add("text", x=x, y=y, fill=C.get(color, color), font_size=size,
                        font_family=font, font_weight=weight, text_anchor=anchor,
                        letter_spacing="0", **attrs)
        node.text = str(value)
        return node

    def line(self, x1, y1, x2, y2, color="line", width=1.5, **attrs):
        return self.add("line", x1=x1, y1=y1, x2=x2, y2=y2,
                        stroke=C[color], stroke_width=width, **attrs)

    def path(self, points, color="ink", dashed=False, arrow=True, **attrs):
        values = {"fill": "none", "stroke": C[color], "stroke_width": 2,
                  "stroke_linejoin": "round", "stroke_linecap": "round"}
        if dashed:
            values["stroke_dasharray"] = "7 7"
        if arrow:
            values["marker_end"] = f"url(#arrow-{color})"
        values.update(attrs)
        return self.add("path", d="M " + " L ".join(f"{x} {y}" for x, y in points), **values)

    def node(self, node_id, x, y, w, h, color, heading, lines, source=None,
             eyebrow=None, title_size=28):
        self.nodes.add(node_id)
        self.rect(x, y, w, h, "white", "line", 5, id=node_id, data_node=node_id)
        self.rect(x, y, 4, h, color)
        if eyebrow:
            label = self.text(x + 24, y + 29, eyebrow, 14, color, 600, True)
            if node_id == "cef_host":
                label.set("id", "native_host_library")
        self.text(x + 24, y + 66, heading, title_size, "ink", 600)
        for i, content in enumerate(lines):
            self.text(x + 24, y + 102 + i * 27, content, 18, "muted")
        if source:
            self.line(x + 24, y + h - 39, x + w - 24, y + h - 39, "subtle")
            self.text(x + 24, y + h - 16, source, 13, "muted", mono=True)

    def label(self, x, y, value, color="muted", size=15, anchor="middle"):
        self.text(x, y, value, size, color, 500, anchor=anchor)

    def detail(self, x, y, heading, lines, color="muted", mono=False):
        self.text(x, y, heading, 17, color, 600)
        for i, content in enumerate(lines):
            self.text(x, y + 31 + i * 27, content, 17, "muted", mono=mono)

    def render(self):
        self.rect(0, 0, W, H, "paper")
        self.rect(96, 56, 88, 5, "rust")
        self.text(96, 158, "AEGIS", 84, weight=700)
        self.text(424, 113, "Persistent browser", 33, weight=500)
        self.text(424, 157, "architecture", 33, "muted")
        self.text(2464, 83, "ENGINEERING / SYSTEM MAP", 15, "muted", 500, True, "end")
        self.text(2464, 119, "Rust 2024  /  C++20 + CEF  /  JavaScript", 17, "ink", anchor="end")
        date = self.architecture["schema"]["generated_from"]["date"]
        self.text(2464, 153, f"Source snapshot  {date}   |   Protocol v1", 15, "muted", mono=True, anchor="end")
        self.line(96, 199, 2464, 199)
        self.text(96, 235, "REAL BROWSER  /  SEMANTIC CONTROL  /  STRUCTURED STATE", 15, "muted", mono=True)
        for x, color, name in [(1556, "rust", "Rust control"), (1764, "native", "Native CEF"), (1972, "page", "Page runtime")]:
            self.rect(x, 223, 10, 10, color)
            self.text(x + 20, 234, name, 15, "muted")
        self.path([(2192, 228), (2248, 228)], "ink")
        self.text(2260, 234, "Request", 14, "muted")
        self.path([(2360, 228), (2400, 228)], "page", dashed=True)
        self.text(2412, 234, "Reply", 14, "muted")

        self.node("user_or_agent", 96, 274, 300, 108, "ink", "User / agent", [],
                  eyebrow="CONTROL CLIENT", title_size=25)
        self.node("aegis_cli", 500, 274, 416, 108, "rust", "aegis CLI", [],
                  eyebrow="navigate  /  search  /  page", title_size=25)
        self.path([(396, 329), (500, 329)])
        self.label(448, 313, "runs", size=14)
        self.node("fozzy", 1788, 274, 676, 108, "ink", "Fozzy determinism engine", [],
                  eyebrow="SCENARIOS + HOST-BACKED SMOKE", title_size=25)
        self.path([(1788, 329), (916, 329)], "muted")
        self.rect(1195, 307, 305, 33, "paper")
        self.label(1348, 329, "scenario execution", size=17)

        self.rect(72, 422, 1928, 962, "white", "line", 6)
        self.rect(2032, 422, 448, 962, "white", "line", 6, stroke_dasharray="8 7")
        self.rect(73, 423, 1926, 59, "rust_tint")
        self.rect(1536, 423, 463, 59, "native_tint")
        self.rect(2033, 423, 446, 59, "page_tint")
        self.text(550, 459, "PERSISTENT SERVE / BROWSER PROCESS", 17, "rust", 600, True)
        self.text(1308, 459, "Rust", 17, "rust", anchor="end")
        self.text(1560, 459, "Native CEF", 17, "native", 600)
        self.text(2056, 459, "RENDERER PROCESS", 17, "page", 600, True)
        self.line(1524, 492, 1524, 1358, "line", 1, stroke_dasharray="3 6")

        self.path([(245, 382), (245, 562)], "rust", data_edge="user_or_agent:http_api")
        self.label(130, 524, "HTTP", "rust", anchor="start")
        self.path([(708, 382), (708, 402), (432, 402), (432, 562)], "rust",
                  data_edge="aegis_cli:http_api")
        self.label(461, 527, "HTTP", "rust", anchor="start")
        self.text(900, 406, "One default runtime; named contexts are managed independently.", 17, "muted")

        self.path([(2378, 562), (2378, 520), (790, 520), (790, 562)], "page", dashed=True,
                  data_edge="renderer:aegis_runtime")
        self.rect(1040, 503, 857, 32, "white")
        self.text(1060, 525, "RESULTS + DOM CHANGES + EVENTS  /  collected and sequenced in Rust", 14, "page", mono=True)

        # Five aligned modules carry the command path; subgraphs below preserve ownership.
        self.node("http_api", 96, 562, 416, 232, "rust", "Local HTTP API", [
            "Axum / 127.0.0.1:7878", "Default + /contexts/{id} routes",
        ], "src/api/server.rs", "REQUEST ROUTING")
        self.nodes.add("context_manager")
        self.text(120, 717, "Context manager", 20, "rust", 600, id="context_manager")
        self.text(120, 743, "ApiCommand via mpsc", 16, "muted", mono=True)

        self.node("aegis_runtime", 584, 562, 416, 232, "rust", "AegisRuntime", [
            "AegisClient + runtime executor", "Batch commands; sequence events",
            "Apply DOM snapshots + mutations", "wait_for / media_state polling",
        ], "src/runtime/executor.rs", "RUNTIME ORCHESTRATION")
        self.node("cef_bridge", 1072, 562, 416, 232, "rust", "CefBridge", [
            "LoadedHost / libloading", "Typed requests + framed JSON",
            "HostFunctionTable calls", "Native buffers freed by free_buffer",
        ], "src/transport/{bridge,protocol}.rs", "RUST / NATIVE TRANSPORT")
        self.node("cef_host", 1560, 562, 416, 232, "native", "AegisCefHost", [
            "Owns CEF lifecycle + message pump", "Routes operations to CEF / renderer",
            "Cookies, storage, network overrides", "Browser state + download events",
        ], "native/src/aegis_cef_host.cpp", "aegis_host / DYNAMIC LIBRARY")
        self.nodes.add("native_host_library")
        self.node("renderer", 2048, 562, 416, 232, "page", "Renderer bridge", [
            "AegisApp / main-frame operations", "Aegis.Request / Aegis.Response",
            "Runtime injection on V8 context", "Aegis.Lifecycle: context_ready",
        ], "native/aegis_app.cc", "CEF RENDERER / V8")
        for x, label, color, edge in [
            (512, "mpsc", "rust", "context_manager:aegis_runtime"),
            (1000, "typed", "rust", "aegis_runtime:cef_bridge"),
            (1488, "C ABI", "native", "cef_bridge:native_host_library"),
            (1976, "IPC", "page", "cef_browser:renderer"),
        ]:
            self.path([(x, 673), (x + 72, 673)], color, data_edge=edge)
            self.label(x + 36, 657, label, color, size=13)

        # External clients consume JSON and SSE; these are not CEF IPC streams.
        self.text(120, 846, "CONTROL SURFACE", 15, "rust", 600, True)
        self.detail(120, 885, "Act", ["POST /execute", "POST /navigate  /search"], "ink", True)
        self.detail(120, 995, "Inspect", ["GET /page/*  /dom", "GET /runtime  /downloads"], "ink", True)
        self.detail(120, 1105, "Observe", ["GET /events", "GET /events/live  (SSE)"], "ink", True)
        self.detail(120, 1215, "Manage", ["/contexts  /session  /trace/enable", "/healthz  /readyz  /doctor"], "ink", True)
        self.text(120, 1355, "JSON responses + filtered event streams", 16, "muted")

        self.text(608, 846, "RUNTIME-OWNED STATE", 15, "rust", 600, True)
        self.detail(608, 885, "DOM model", ["DomTree + snapshot / diff", "Semantic matchers + action targets"], "ink")
        self.detail(608, 995, "Event stream", ["Navigation, DOM, network, download", "WebSocket + logs; retention gaps"], "ink")
        self.detail(608, 1105, "Scheduler", ["Batch IDs + event sequence numbers", "Synthetic event timestamps"], "ink")
        self.detail(608, 1215, "Research + session state", ["Text, headings, links, forms, actions", "Session validation; upload staging"], "ink")
        self.text(608, 1355, "Headless or headful; bootstrap is local", 16, "muted")

        self.text(1096, 846, "IN-PROCESS BOUNDARY", 15, "rust", 600, True)
        self.text(1096, 885, "AEGS frame", 23, "ink", 600)
        self.text(1096, 914, "16-byte header + JSON envelope", 17, "muted")
        fields = [(1096, 82, "AEGS", "4 B"), (1178, 72, "v1", "2 B"),
                  (1250, 72, "kind", "2 B"), (1322, 142, "body length", "8 B")]
        for x, width, name, size in fields:
            self.rect(x, 936, width, 44, "rust_tint", "line")
            self.text(x + width / 2, 963, name, 13, "rust", 500, True, "middle")
            self.text(x + width / 2, 1007, size, 14, "muted", mono=True, anchor="middle")
        self.text(1096, 1045, "Little-endian fields; version + length checks", 16, "muted")
        self.detail(1096, 1105, "Host function table", [
            "ensure_runtime / eval_js / send_batch", "snapshot_dom / session / host state",
            "navigate / activate_browser / events", "pump / request_cancel / free_buffer",
        ], "ink")
        self.text(1096, 1260, "The shared library lives in the host process.", 16, "muted")
        self.text(1096, 1290, "CEF messages cross the renderer boundary.", 16, "muted")
        self.text(1096, 1355, "native/include/aegis_host_abi.h", 13, "muted", mono=True)

        self.path([(1768, 794), (1768, 880)], "native", data_edge="cef_host:cef_browser")
        self.label(1790, 848, "owns / manages", "native", anchor="start")
        self.node("cef_browser", 1560, 880, 416, 178, "native", "CEF browser", [
            "Browser instances + request contexts", "Navigation, cookies, downloads",
        ], "native/aegis_client.cc", "REAL BROWSER ENGINE", 26)
        self.path([(1768, 1058), (1768, 1134)], "native", data_edge="cef_host:devtools_network")
        self.label(1790, 1100, "DevTools observer", "native", anchor="start")
        self.node("devtools_network", 1560, 1134, 416, 146, "native", "Network instrumentation", [
            "Network + WebSocket capture",
        ], eyebrow="Network.enable", title_size=23)
        self.text(1584, 1317, "PLATFORM + LIFECYCLE", 14, "native", 600, True)
        self.text(1584, 1348, "macOS / Linux helpers; owner-thread pump", 16, "muted")

        self.path([(2256, 794), (2256, 880)], "page", data_edge="renderer:page_js_runtime")
        self.label(2278, 848, "inject runtime script", "page", anchor="start")
        self.node("page_js_runtime", 2048, 880, 416, 178, "page", "window.__aegis", [
            "Semantic match / exec / snapshot", "Research, wait state, event queues",
        ], "assets/js/aegis_runtime.js", "INJECTED JAVASCRIPT", 26)
        self.path([(2256, 1058), (2256, 1134)], "page", data_edge="page_js_runtime:page_dom")
        self.label(2278, 1100, "inspect + act", "page", anchor="start")
        self.node("page_dom", 2048, 1134, 416, 146, "page", "Real page DOM", [
            "Elements, forms, media, mutations",
        ], eyebrow="BROWSER DOCUMENT", title_size=26)
        self.text(2072, 1317, "SEMANTIC TARGETS", 14, "page", 600, True)
        self.text(2072, 1348, "Role, name, label, text; document-local IDs", 16, "muted")

        # Persistence is a full-width band, separate from the live process topology.
        self.rect(72, 1430, 2408, 252, "#F0F2F4")
        self.path([(96, 743), (54, 743), (54, 1480), (96, 1480)], "storage",
                  data_edge="http_api:aegis_state")
        self.path([(1000, 744), (1028, 744), (1028, 1410), (1232, 1410), (1232, 1430)],
                  "rust", data_edge="aegis_runtime:trace_file")
        self.text(1047, 1382, "optional runtime recording", 15, "rust")
        self.nodes.add("aegis_state")
        self.text(96, 1470, "CANONICAL AEGIS STATE", 14, "storage", 600, True)
        self.text(96, 1509, "~/.aegis", 26, weight=600, mono=True, id="aegis_state")
        self.text(96, 1543, "Settings / profiles / secrets", 19, "ink", 500)
        self.text(96, 1576, "Session JSON, credentials, uploads, downloads, logs", 17, "muted")
        self.text(96, 1608, "$AEGIS_HOME overrides the root; atomic JSON writes", 16, "muted")
        self.text(96, 1640, "Chromium profile internals are runtime substrate.", 16, "muted")
        self.line(812, 1460, 812, 1652)

        self.nodes.add("trace_file")
        self.text(852, 1470, "AEGIS RUNTIME TRACE", 14, "rust", 600, True)
        self.text(852, 1509, "TraceRecorder", 26, weight=600, id="trace_file")
        self.text(852, 1543, "Browser config + initial session + batches", 19, "ink", 500)
        self.text(852, 1576, "JSON requests, responses, snapshots + sequenced events", 17, "muted")
        self.text(852, 1608, "aegis trace inspect", 17, "rust", mono=True)
        self.text(852, 1640, "Reconstructs snapshot + event state without CEF.", 16, "muted")
        self.line(1656, 1460, 1656, 1652)

        self.text(1696, 1470, "FOZZY SCENARIO TRACE", 14, "muted", 600, True,
                  id="fozzy_trace", data_edge="fozzy:fozzy_trace")
        self.text(1696, 1509, "Verification pipeline", 26, weight=600)
        self.text(1696, 1543, "doctor / deterministic tests / host smoke", 19, "ink", 500)
        self.text(1696, 1576, "fozzy run --record <trace.fozzy>", 17, "muted", mono=True)
        self.text(1696, 1608, "trace verify   >   replay   >   ci", 17, "muted", mono=True)
        self.text(1696, 1640, "Separate from Aegis's JSON runtime trace format.", 16, "muted")

        self.line(96, 1740, 2464, 1740)
        self.text(96, 1774, "COMMAND PATH", 14, "rust", 600, True)
        self.text(280, 1774, "Route  >  serialize  >  execute  >  bridge  >  render  >  inspect / act", 18, "ink")
        self.text(96, 1810, "Source: aegis-architecture.json  |  boundary checks: server.rs, host ABI, aegis_app.cc, trace/recorder.rs", 14, "muted", mono=True)
        self.text(2464, 1774, "Logical ownership + selected runtime paths", 15, "muted", anchor="end")
        self.text(2464, 1810, "AEGIS  /  ARCHITECTURE  /  01", 14, "muted", mono=True, anchor="end")
        expected = {n["id"] for n in self.architecture["graph_model"]["nodes"]}
        if self.nodes != expected:
            raise ValueError(f"Graph coverage mismatch: {self.nodes ^ expected}")
        return ET.tostring(self.root, encoding="unicode", xml_declaration=True)


def verify(architecture):
    svg = OUTPUT / "aegis-architecture.svg"
    root = ET.parse(svg).getroot()
    meta = json.loads(root.find(f"{{{NS}}}metadata").text)
    if meta["sha256"] != hashlib.sha256(SOURCE.read_bytes()).hexdigest():
        raise ValueError("Architecture source changed; regenerate the diagram.")
    ids = [element.get("id") for element in root.iter() if element.get("id")]
    if len(ids) != len(set(ids)):
        raise ValueError("Duplicate SVG IDs")
    for node in architecture["graph_model"]["nodes"]:
        if node["id"] not in ids:
            raise ValueError(f"Missing architecture node: {node['id']}")
    for extension in ("svg", "png", "pdf"):
        path = OUTPUT / f"aegis-architecture.{extension}"
        if path.stat().st_size < 10000:
            raise ValueError(f"Missing or unexpectedly small output: {path}")
    if (OUTPUT / "aegis-architecture.png").read_bytes()[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("Invalid PNG")
    if not (OUTPUT / "aegis-architecture.pdf").read_bytes().startswith(b"%PDF-"):
        raise ValueError("Invalid PDF")
    print(json.dumps({"ok": True, "graph_nodes": len(architecture["graph_model"]["nodes"]),
                      "source_hash_matches": True, "formats": ["svg", "png", "pdf"]}, sort_keys=True))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify", action="store_true", help="Check existing artifacts without regenerating.")
    args = parser.parse_args()
    architecture = json.loads(SOURCE.read_text())
    if not args.verify:
        renderer = shutil.which("rsvg-convert")
        if not renderer:
            parser.error("rsvg-convert is required for PNG/PDF export (SVG construction is local).")
        OUTPUT.mkdir(parents=True, exist_ok=True)
        svg = OUTPUT / "aegis-architecture.svg"
        svg.write_text(Plate(architecture).render())
        for extension, options in [("png", ["--width", "5120"]), ("pdf", [])]:
            subprocess.run([renderer, *options, "--format", extension, "--output",
                            str(OUTPUT / f"aegis-architecture.{extension}"), str(svg)], check=True)
    verify(architecture)


if __name__ == "__main__":
    main()
