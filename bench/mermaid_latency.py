#!/usr/bin/env python3
"""Measure cold and cached Mermaid PTY output; never physical terminal paint.

Uses one three-node flowchart in fresh processes. Cold timings include the
150 ms worker debounce. Forced Kitty mode measures PNG upload and placement
bytes against a synthetic PTY, not terminal capability negotiation or pixels.
"""
import argparse
import codecs
import fcntl
import json
import os
import pathlib
import pty
import select
import signal
import statistics
import struct
import tempfile
import termios
import time

import pyte


SOURCE = "# Benchmark\n\n```mermaid\nflowchart LR\nA[BenchStart] --> B[BenchProcess] --> C[BenchEnd]\n```\n"


def run(binary, mode):
    with tempfile.TemporaryDirectory(prefix="vaayu-mermaid-bench-") as tmp:
        root = pathlib.Path(tmp)
        config = root / "config/vaayu"
        config.mkdir(parents=True)
        (config / "config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
            f'watch=false\nprogress=false\nmermaid_preview="{mode}"\n'
        )
        source = root / "diagram.md"
        source.write_text(SOURCE)
        pid, fd = pty.fork()
        if pid == 0:
            os.chdir(root)
            os.environ.update(
                TERM="xterm-256color", XDG_CONFIG_HOME=str(root / "config"),
                XDG_DATA_HOME=str(root / "data"), XDG_STATE_HOME=str(root / "state"),
            )
            os.environ.pop("NO_COLOR", None)
            os.environ.pop("VAAYU_PROFILE", None)
            os.execv(binary, [binary, str(source)])
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 100, 800, 640))
        screen = pyte.Screen(100, 40)
        stream = pyte.Stream(screen)
        decoder = codecs.getincrementaldecoder("utf-8")("replace")

        def read(timeout):
            ready, _, _ = select.select([fd], [], [], timeout)
            if not ready:
                return b""
            data = os.read(fd, 65536)
            # pyte ignores Kitty APC payloads, as in the regression harness.
            stream.feed(decoder.decode(data))
            return data

        def drain(seconds):
            end = time.perf_counter() + seconds
            while time.perf_counter() < end:
                read(min(0.01, max(0, end - time.perf_counter())))

        def measure():
            capture = bytearray()
            first = None
            start = time.perf_counter()
            os.write(fd, b",mp")
            while time.perf_counter() - start < 8:
                data = read(0.005)
                now = time.perf_counter()
                if data:
                    first = first if first is not None else now - start
                    capture.extend(data)
                text = "\n".join(screen.display)
                if mode == "kitty":
                    done = b"a=p,U=1" in capture and b"\xf4\x8e\xbb\xae" in capture
                else:
                    done = ("BenchStart" in text and "BenchEnd" in text
                            and "flowchart LR" not in text and "rendering" not in text)
                if done:
                    return {"first_output_ms": round(first * 1000, 3),
                            "diagram_output_ms": round((now - start) * 1000, 3),
                            "bytes_through_diagram_output": len(capture),
                            "png_uploads": capture.count(b"a=t,")}
            raise RuntimeError(f"{mode} render timed out:\n{text}")

        try:
            drain(0.5)
            cold = measure()
            drain(0.05)
            os.write(fd, b"q")
            drain(0.05)
            cached = measure()
            if mode == "kitty":
                assert cold["png_uploads"] == 1 and cached["png_uploads"] == 1
            return {"cold": cold, "cached_reopen": cached}
        finally:
            os.kill(pid, signal.SIGKILL)
            os.waitpid(pid, 0)
            os.close(fd)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--vaayu", default="target/release/vaayu")
    parser.add_argument("--attempts", type=int, default=5)
    parser.add_argument("--out", required=True)
    args = parser.parse_args()
    if args.attempts < 1:
        parser.error("--attempts must be positive")
    binary = str(pathlib.Path(args.vaayu).resolve())
    result = {"method": __doc__.strip(), "terminal": [100, 40],
              "cell_pixels": [8, 16], "source": SOURCE, "modes": {}}
    for mode in ("unicode", "kitty"):
        samples = [run(binary, mode) for _ in range(args.attempts)]
        result["modes"][mode] = {
            "samples": samples,
            "medians_ms": {phase: {metric: round(statistics.median(
                sample[phase][metric] for sample in samples), 3)
                for metric in ("first_output_ms", "diagram_output_ms")}
                for phase in ("cold", "cached_reopen")},
        }
    pathlib.Path(args.out).write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
