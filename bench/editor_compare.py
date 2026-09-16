#!/usr/bin/env python3
"""Local PTY comparison across Vaayu, Neovim and Helix.

Reports key-to-first-byte and key-to-quiet-terminal timings. The latter waits
for a small quiet window and is a useful completed-output proxy, although it
still cannot measure physical terminal painting. Editor-specific picker keys
are supported. Results are evidence for these workloads, not a universal rank.
"""
import argparse
import json
import os
import pathlib
import pty
import select
import shlex
import signal
import statistics
import struct
import tempfile
import termios
import time
import fcntl


def percentile(values, p):
    values = sorted(values)
    if not values:
        return None
    at = (len(values) - 1) * p
    lo = int(at)
    hi = min(lo + 1, len(values) - 1)
    return values[lo] + (values[hi] - values[lo]) * (at - lo)


def spawn(command, path, cwd, cols, rows, env):
    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(cwd)
        os.environ.update(env)
        os.execvp(command[0], command + [path])
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
    return pid, fd


def drain(fd, timeout):
    data = bytearray()
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        ready, _, _ = select.select([fd], [], [], min(0.01, deadline - time.monotonic()))
        if not ready:
            continue
        try:
            part = os.read(fd, 65536)
        except OSError:
            break
        if not part:
            break
        data.extend(part)
    return data


def transact(fd, keys, quiet=0.012, timeout=3.0):
    drain(fd, 0.005)
    started = time.perf_counter()
    os.write(fd, keys)
    first = None
    total = 0
    last = started
    while time.perf_counter() - started < timeout:
        ready, _, _ = select.select([fd], [], [], quiet)
        now = time.perf_counter()
        if ready:
            try:
                part = os.read(fd, 65536)
            except OSError:
                break
            if not part:
                break
            if first is None:
                first = now - started
            total += len(part)
            last = now
        elif first is not None and now - last >= quiet:
            return first, now - started, total
    return first, None, total


def settle(fd, minimum=0.15, quiet=0.025, timeout=5.0):
    """Collect delayed async output, waiting at least `minimum` seconds."""
    started = time.perf_counter()
    first = None
    total = 0
    last = started
    while time.perf_counter() - started < timeout:
        ready, _, _ = select.select([fd], [], [], min(quiet, timeout))
        now = time.perf_counter()
        if ready:
            try:
                part = os.read(fd, 65536)
            except OSError:
                break
            if not part:
                break
            if first is None:
                first = now - started
            total += len(part)
            last = now
        elif now - started >= minimum and now - last >= quiet:
            return first, now - started, total
    return first, None, total


def summary(samples):
    out = {}
    for label in sorted({label for label, *_ in samples}):
        group = [item for item in samples if item[0] == label]
        first = [x[1] * 1000 for x in group if x[1] is not None]
        complete = [x[2] * 1000 for x in group if x[2] is not None]
        out[label] = {
            "n": len(group),
            "first_p50_ms": round(statistics.median(first), 3) if first else None,
            "first_p95_ms": round(percentile(first, 0.95), 3) if first else None,
            "quiet_p50_ms": round(statistics.median(complete), 3) if complete else None,
            "quiet_p95_ms": round(percentile(complete, 0.95), 3) if complete else None,
            "bytes_median": round(statistics.median(x[3] for x in group)),
            "timeouts": sum(x[2] is None for x in group),
        }
    return out


def run(spec, path, cwd, cols, rows):
    env = {
        "TERM": "xterm-256color",
        "XDG_CONFIG_HOME": spec["config"],
        "HELIX_RUNTIME": spec.get("runtime", ""),
    }
    if "cache" in spec:
        env["XDG_CACHE_HOME"] = spec["cache"]
        env["XDG_STATE_HOME"] = spec["state"]
    started = time.perf_counter()
    pid, fd = spawn(spec["command"], path, cwd, cols, rows, env)
    ready, _, _ = select.select([fd], [], [], 10)
    startup = time.perf_counter() - started if ready else None
    drain(fd, 0.5)
    try:
        status = pathlib.Path(f"/proc/{pid}/status").read_text()
        rss_kib = int(next(line.split()[1] for line in status.splitlines() if line.startswith("VmRSS:")))
    except (OSError, StopIteration, ValueError):
        rss_kib = None
    samples = []

    def record(label, keys, count=1):
        for _ in range(count):
            first, complete, size = transact(fd, keys)
            samples.append((label, first, complete, size))

    record("motion_down", b"j", 80)
    record("page_scroll", b"\x04", 20)
    record("insert_enter", b"i")
    for char in b"quick brown fox 0123456789":
        record("insert_char", bytes([char]))
    record("insert_leave", b"\x1b")
    record("buffer_search_submit", b"/repeat_target\r")
    record("buffer_search_next", b"n", 10)
    if spec.get("picker"):
        record("picker_open", spec["picker"])
        first, complete, size = settle(fd, 0.12)
        samples.append(("picker_inventory", first, complete, size))
        for char in b"render":
            record("picker_filter", bytes([char]))
        record("picker_close", b"\x1b")
    if spec.get("grep"):
        record("grep_open", spec["grep"])
        for char in b"request_timeout":
            record("grep_filter", bytes([char]))
        first, complete, size = settle(fd, 0.25)
        samples.append(("grep_results", first, complete, size))
    try:
        os.kill(pid, signal.SIGKILL)
        os.waitpid(pid, 0)
    except ProcessLookupError:
        pass
    os.close(fd)
    return {
        "version": spec["version"],
        "startup_first_ms": round(startup * 1000, 3) if startup else None,
        "idle_rss_kib": rss_kib,
        "operations": summary(samples),
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--vaayu", default="target/release/vaayu")
    parser.add_argument("--nvim", default="nvim")
    parser.add_argument("--helix", default="hx")
    parser.add_argument("--helix-runtime", default="/usr/lib/helix/runtime")
    parser.add_argument("--out", default="bench/editor-comparison.json")
    parser.add_argument("--cols", type=int, default=100)
    parser.add_argument("--rows", type=int, default=40)
    parser.add_argument(
        "--only",
        action="append",
        choices=["vaayu", "nvim_bare", "nvim_user", "helix"],
        help="run only the named editor (repeatable)",
    )
    args = parser.parse_args()
    root = pathlib.Path.cwd()
    with tempfile.TemporaryDirectory(prefix="vaayu-compare-") as tmp:
        tmp = pathlib.Path(tmp)
        source = tmp / "large.rs"
        source.write_text("\n".join(
            f"fn line_{i}() {{ /* {'repeat_target' if i % 500 == 0 else f'unique_{i}'} */ }}"
            for i in range(50000)
        ) + "\n")
        vaayu_cfg = tmp / "vaayu-config"
        (vaayu_cfg / "vaayu").mkdir(parents=True)
        (vaayu_cfg / "vaayu/config.toml").write_text(
            "jk_escape=false\nclipboard_unnamedplus=false\n[lsp.rust]\nenabled=false\n"
        )
        empty_cfg = tmp / "empty-config"
        empty_cfg.mkdir()
        (empty_cfg / "helix").mkdir()
        (empty_cfg / "helix/languages.toml").write_text(
            "[[language]]\nname='rust'\nlanguage-servers=[]\n"
        )
        specs = {
            "vaayu": {
                "command": [str(pathlib.Path(args.vaayu).resolve())],
                "config": str(vaayu_cfg),
                "picker": b"\x10",
                "grep": b",/",
                "version": "workspace release",
            },
            "nvim_bare": {
                "command": [args.nvim, "-u", "NONE", "--cmd", "set noswapfile shadafile=NONE"],
                "config": str(empty_cfg),
                "picker": None,
                "grep": None,
                "version": "0.12.5 bare; no comparable interactive project picker",
            },
            "nvim_user": {
                "command": [args.nvim],
                "config": str(pathlib.Path.home() / ".config"),
                "picker": b"\x10",
                "grep": b",/",
                "version": "0.12.5 with ~/.config/nvim (Snacks picker)",
            },
            "helix": {
                "command": [str(pathlib.Path(args.helix).resolve())],
                "config": str(empty_cfg),
                "runtime": str(pathlib.Path(args.helix_runtime).resolve()),
                "picker": b" f",
                "grep": b" /",
                "version": "25.07.1 official x86_64 release; Rust LSP disabled",
            },
        }
        results = {
            "method": "local PTY, first byte and 12 ms quiet-window completion proxy",
            "file": "50,000 generated Rust lines",
            "terminal": [args.cols, args.rows],
            "editors": {},
        }
        for name, spec in specs.items():
            if args.only and name not in args.only:
                continue
            if name == "helix":
                cache = tmp / "helix-cache"
                state = tmp / "helix-state"
                cache.mkdir()
                state.mkdir()
                spec["cache"] = str(cache)
                spec["state"] = str(state)
            results["editors"][name] = run(spec, str(source), str(root), args.cols, args.rows)
            print(name, json.dumps(results["editors"][name], indent=2))
        pathlib.Path(args.out).write_text(json.dumps(results, indent=2) + "\n")


if __name__ == "__main__":
    main()
