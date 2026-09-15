#!/usr/bin/env python3
"""Key-to-first-response latency benchmark for terminal editors.

Measures wall-clock time from writing a key to a pty to the first byte of
the editor's response arriving -- a responsiveness proxy, not completed-frame
latency. This is distinct from "time until the screen goes fully quiet" (which conflates rendering
completion with genuine idle time and would understate responsiveness for
an editor that redraws in more than one write). Runs a scripted, realistic
editing session (movement, word motion, insert-mode typing, undo, search)
against each editor in turn and reports p50/p90/p99 per operation class and
overall.

Usage:
    python3 bench/latency.py --cols 100 --rows 40 --out bench/results.json \
        vaayu:/path/to/vy \
        "nvim (bare)":"nvim -u NONE --cmd 'set noswapfile'" \
        "nvim (user config)":nvim

Each editor arg is `label:command`. The command is split with shlex and the
target file path is appended. Run from the repo root; picks a real source
file from the repo as the edit target unless --file is given.
"""
import argparse
import json
import os
import pty
import select
import shlex
import statistics
import struct
import sys
import termios
import time
import fcntl


def spawn(argv, cols, rows, cwd=None):
    pid, fd = pty.fork()
    if pid == 0:
        if cwd:
            os.chdir(cwd)
        os.environ["TERM"] = "xterm-256color"
        try:
            os.execvp(argv[0], argv)
        except Exception as e:
            sys.stderr.write(f"exec failed: {e}\n")
        os._exit(1)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
    return pid, fd


def drain(fd, timeout=0.05):
    out = b""
    while True:
        r, _, _ = select.select([fd], [], [], timeout)
        if not r:
            break
        try:
            d = os.read(fd, 65536)
        except OSError:
            break
        if not d:
            break
        out += d
    return out


def wait_for_response(fd, max_wait=2.0):
    start = time.time()
    r, _, _ = select.select([fd], [], [], max_wait)
    if not r:
        return None
    t = time.time() - start
    drain(fd, 0.08)  # let the rest of this frame's burst clear, uncounted
    return t


def send_key(fd, key, max_wait=2.0):
    os.write(fd, key.encode() if isinstance(key, str) else key)
    return wait_for_response(fd, max_wait)


def kill(pid):
    try:
        os.kill(pid, 9)
        os.waitpid(pid, 0)
    except Exception:
        pass


def percentile(data, p):
    if not data:
        return None
    data = sorted(data)
    k = (len(data) - 1) * p
    f = int(k)
    c = min(f + 1, len(data) - 1)
    if f == c:
        return data[f]
    return data[f] + (data[c] - data[f]) * (k - f)


def run_session(argv, filepath, cols, rows, cwd=None, warmup=1.2):
    pid, fd = spawn(argv + [filepath], cols, rows, cwd)
    time.sleep(warmup)
    drain(fd, 0.5)  # discard startup frame(s)

    samples = []  # (label, seconds)
    timeouts = 0

    def record(label, lat):
        nonlocal timeouts
        if lat is None:
            timeouts += 1
        else:
            samples.append((label, lat))

    for _ in range(100):
        record("move_down", send_key(fd, "j"))
    for _ in range(50):
        record("word_fwd", send_key(fd, "w"))

    record("enter_insert", send_key(fd, "o"))
    for ch in "the quick brown fox jumps over the lazy dog 1234567890":
        record("insert_char", send_key(fd, ch))
    record("leave_insert", send_key(fd, "\x1b"))

    for _ in range(10):
        record("undo", send_key(fd, "u"))
    for _ in range(10):
        record("redo", send_key(fd, "\x12"))

    record("search_open", send_key(fd, "/"))
    for ch in "fox":
        record("search_type", send_key(fd, ch))
    record("search_submit", send_key(fd, "\r"))
    for _ in range(5):
        record("search_next", send_key(fd, "n"))

    kill(pid)
    return samples, timeouts


def summarize(samples):
    by_label = {}
    for label, lat in samples:
        by_label.setdefault(label, []).append(lat)
    all_lat = [lat for _, lat in samples]
    out = {
        "n": len(all_lat),
        "p50_ms": round(percentile(all_lat, 0.50) * 1000, 3) if all_lat else None,
        "p90_ms": round(percentile(all_lat, 0.90) * 1000, 3) if all_lat else None,
        "p99_ms": round(percentile(all_lat, 0.99) * 1000, 3) if all_lat else None,
        "max_ms": round(max(all_lat) * 1000, 3) if all_lat else None,
        "by_label_p50_ms": {
            k: round(percentile(v, 0.50) * 1000, 3) for k, v in sorted(by_label.items())
        },
    }
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("editors", nargs="+", help="label:command entries")
    ap.add_argument("--file", default=None, help="target file to edit (defaults to a real repo source file)")
    ap.add_argument("--cols", type=int, default=100)
    ap.add_argument("--rows", type=int, default=40)
    ap.add_argument("--cwd", default=None)
    ap.add_argument("--out", default=None, help="write JSON results here")
    args = ap.parse_args()

    cwd = args.cwd or os.getcwd()
    filepath = args.file or os.path.join(cwd, "src", "normal.rs")
    if not os.path.exists(filepath):
        sys.exit(f"target file not found: {filepath}")

    # Work on a throwaway copy so repeated runs (and undo/redo/insert bursts)
    # never touch a real tracked file.
    import shutil, tempfile

    tmpdir = tempfile.mkdtemp(prefix="vaayu-bench-")
    bench_file = os.path.join(tmpdir, os.path.basename(filepath))
    shutil.copy(filepath, bench_file)

    results = {}
    for entry in args.editors:
        label, cmd = entry.split(":", 1)
        argv = shlex.split(cmd)
        print(f"--- {label} ({cmd}) ---", file=sys.stderr)
        samples, timeouts = run_session(argv, bench_file, args.cols, args.rows, cwd=cwd)
        summary = summarize(samples)
        summary["timeouts"] = timeouts
        results[label] = summary
        print(json.dumps(summary, indent=2), file=sys.stderr)
        shutil.copy(filepath, bench_file)  # reset between editors

    shutil.rmtree(tmpdir, ignore_errors=True)

    print(json.dumps(results, indent=2))
    if args.out:
        with open(args.out, "w") as f:
            json.dump(results, f, indent=2)


if __name__ == "__main__":
    main()
