#!/usr/bin/env python3

import argparse
import json
import os
import signal
import socket
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path
from statistics import median


def http_get_json(url: str, timeout: float) -> dict:
    with urllib.request.urlopen(url, timeout=timeout) as response:
        return json.loads(response.read().decode())


def http_post_json(url: str, payload: dict, timeout: float) -> dict:
    data = json.dumps(payload).encode()
    request = urllib.request.Request(
        url,
        data=data,
        headers={"content-type": "application/json"},
    )
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return json.loads(response.read().decode())


def http_ready_json(base_url: str, timeout: float) -> dict:
    with urllib.request.urlopen(f"{base_url}/readyz", timeout=timeout) as response:
        return json.loads(response.read().decode())


def wait_for_command_ready(base_url: str, timeout_s: float) -> tuple[float, dict, int]:
    started = time.time()
    attempts = 0
    while time.time() - started < timeout_s:
        attempts += 1
        try:
            diagnostics = http_ready_json(base_url, timeout=1.0)
            return time.time() - started, diagnostics, attempts
        except urllib.error.HTTPError as error:
            if error.code != 503:
                raise
            time.sleep(0.05)
        except Exception:
            time.sleep(0.05)
    raise TimeoutError("runtime did not become command-ready in time")


def build_release_binary(root: Path) -> Path:
    subprocess.run(
        ["cargo", "build", "--release", "--bin", "aegis"],
        cwd=root,
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    workspace_release = root / "target" / "aarch64-apple-darwin" / "release" / "aegis"
    workspace_default_release = root / "target" / "release" / "aegis"
    if workspace_release.exists():
        return workspace_release
    if workspace_default_release.exists():
        return workspace_default_release
    raise FileNotFoundError("release aegis binary was not produced by cargo build --release")


def resolve_host_library(root: Path, configured: str | None) -> str:
    if configured:
        return configured
    installed_host_lib = (
        Path.home()
        / "Applications"
        / "Aegis.app"
        / "Contents"
        / "Frameworks"
        / "libaegis_host.dylib"
    )
    workspace_host_lib = root / "native" / "build-xcode" / "Release" / "libaegis_host.dylib"
    if installed_host_lib.exists():
        return str(installed_host_lib)
    return str(workspace_host_lib)


def watch_ready_banner(log_path: str, started_at: float, result: dict) -> None:
    path = Path(log_path)
    deadline = time.time() + 60.0
    position = 0
    if not log_path:
        return
    try:
        while time.time() < deadline:
            if not path.exists():
                time.sleep(0.05)
                continue
            with path.open("r", encoding="utf-8", errors="replace") as handle:
                handle.seek(position)
                for line in handle:
                    if "Aegis serve ready on http://" in line:
                        result["serve_ready_banner_ms"] = round((time.time() - started_at) * 1000, 1)
                        result["serve_ready_banner_line"] = line.strip()
                        return
                position = handle.tell()
            time.sleep(0.05)
    except Exception:
        return


def ensure_port_free(host: str, port: int) -> None:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.settimeout(0.25)
        if sock.connect_ex((host, port)) != 0:
            return

    subprocess.run(
        ["zsh", "-lc", f"lsof -ti tcp:{port} | xargs -r kill -9"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    time.sleep(0.2)


def main() -> int:
    parser = argparse.ArgumentParser(description="Measure Aegis cold-start and first-command latency.")
    parser.add_argument("--addr", default="127.0.0.1:7915")
    parser.add_argument("--mode", choices=("headless", "headful"), default="headless")
    parser.add_argument("--start-url")
    parser.add_argument("--host-lib")
    parser.add_argument("--profile", default="measure-startup")
    parser.add_argument("--timeout", type=float, default=45.0)
    parser.add_argument("--debug-log")
    parser.add_argument("--samples", type=int, default=1)
    args = parser.parse_args()

    root = Path(__file__).resolve().parents[1]
    binary = build_release_binary(root)
    host_lib = resolve_host_library(root, args.host_lib)
    base_url = f"http://{args.addr}"
    host, port_text = args.addr.rsplit(":", 1)
    ensure_port_free(host, int(port_text))

    env = os.environ.copy()
    env["AEGIS_WORKSPACE_ROOT"] = str(root)

    command = [
        str(binary),
        "--profile",
        args.profile,
        "--mode",
        args.mode,
        "--host-lib",
        host_lib,
        "serve",
        "--addr",
        args.addr,
    ]
    if args.start_url:
        command[1:1] = ["--start-url", args.start_url]

    def run_sample(sample_index: int) -> dict:
        debug_log = args.debug_log
        if debug_log and args.samples > 1:
            debug_path = Path(args.debug_log)
            debug_log = str(
                debug_path.with_name(
                    f"{debug_path.stem}-{sample_index + 1}{debug_path.suffix}"
                )
            )

        sample_env = env.copy()
        if debug_log:
            sample_env["AEGIS_DEBUG_LOG"] = debug_log
        sanitized_addr = args.addr.replace(":", "-")
        server_log = str(
            root / "tmp" / f"measure-startup-{args.mode}-{sanitized_addr}-{sample_index + 1}.log"
        )
        Path(server_log).parent.mkdir(parents=True, exist_ok=True)
        Path(server_log).write_text("", encoding="utf-8")

        launch_started_at = time.time()
        with open(server_log, "a", encoding="utf-8") as server_stream:
            process = subprocess.Popen(
                command,
                cwd=root,
                env=sample_env,
                stdout=server_stream,
                stderr=subprocess.STDOUT,
                text=True,
                bufsize=1,
            )
        started_at = time.time()
        banner_info: dict[str, object] = {}
        banner_thread = threading.Thread(
            target=watch_ready_banner,
            args=(server_log, started_at, banner_info),
            daemon=True,
        )
        banner_thread.start()

        try:
            runtime_ready_s, readiness_before, runtime_attempts = wait_for_command_ready(
                base_url, args.timeout
            )
            runtime_before = http_get_json(f"{base_url}/runtime", timeout=1.0)

            first_command_started = time.time()
            first_execute = http_post_json(
                f"{base_url}/execute",
                {"commands": [{"type": "eval", "code": "document.title"}]},
                timeout=args.timeout,
            )
            first_command_s = time.time() - first_command_started

            runtime_after = http_get_json(f"{base_url}/runtime", timeout=1.0)

            report = {
                "addr": args.addr,
                "mode": args.mode,
                "start_url": args.start_url,
                "pid": process.pid,
                "process_spawn_ms": round((started_at - launch_started_at) * 1000, 1),
                "runtime_ready_ms": round(runtime_ready_s * 1000, 1),
                "runtime_poll_attempts": runtime_attempts,
                "first_command_ms": round(first_command_s * 1000, 1),
                "readiness_before": readiness_before,
                "runtime_before": runtime_before,
                "first_execute": first_execute,
                "runtime_after": runtime_after,
                "debug_log": debug_log,
                "server_log": server_log,
            }
            report.update(banner_info)
            return report
        finally:
            try:
                process.terminate()
                process.wait(timeout=5)
            except Exception:
                try:
                    os.kill(process.pid, signal.SIGKILL)
                except Exception:
                    pass
            banner_thread.join(timeout=0.2)

    if args.samples == 1:
        print(json.dumps(run_sample(0), indent=2))
        return 0

    samples = [run_sample(i) for i in range(args.samples)]
    summary = {
        "samples": args.samples,
        "mode": args.mode,
        "addr": args.addr,
        "median_process_spawn_ms": round(median(sample["process_spawn_ms"] for sample in samples), 1),
        "median_runtime_ready_ms": round(median(sample["runtime_ready_ms"] for sample in samples), 1),
        "median_first_command_ms": round(median(sample["first_command_ms"] for sample in samples), 1),
        "max_runtime_ready_ms": round(max(sample["runtime_ready_ms"] for sample in samples), 1),
        "max_first_command_ms": round(max(sample["first_command_ms"] for sample in samples), 1),
        "sample_reports": samples,
    }
    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
