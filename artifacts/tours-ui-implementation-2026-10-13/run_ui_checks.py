#!/usr/bin/env python3
"""Reproduce release tour checks and adjacent pane/mouse/AI checks.

Usage: python run_ui_checks.py [vaayu-binary] [vy-binary]
Install tests/requirements.txt in the active Python environment first.
"""
from concurrent.futures import ThreadPoolExecutor, as_completed
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

OUT = Path(__file__).resolve().parent
ROOT = OUT.parent.parent
TOURS = ("pty_tours", "pty_tour_focus", "pty_tour_scroll", "pty_tour_highlight",
         "pty_tour_visuals", "pty_tour_markdown", "pty_tour_ux")
NEIGHBORS = ("pty_regression", "pty_window_navigation", "pty_sidebar_navigation",
             "pty_markdown_mouse", "pty_ai_and_tours")


def check(binary, suite):
    log_path = OUT / f"{binary.name}-{suite}.log"
    args = [sys.executable, str(ROOT / "tests" / f"{suite}.py"), str(binary)]
    if binary.name == "vaayu" and suite in ("pty_tour_ux", "pty_tour_markdown"):
        args.append(str(OUT / "captures" / suite))
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix="vaayu-tour-validation-") as tmp:
        env = os.environ.copy()
        env.pop("NO_COLOR", None)
        for key in ("CONFIG", "DATA", "STATE", "CACHE"):
            env[f"XDG_{key}_HOME"] = str(Path(tmp) / key.lower())
        with log_path.open("w") as log:
            result = subprocess.run(args, env=env, cwd=tmp, stdout=log,
                                    stderr=subprocess.STDOUT, timeout=600)
    return {"binary": binary.name, "suite": suite, "exit_code": result.returncode,
            "seconds": round(time.monotonic() - started, 2), "log": log_path.name}


def main():
    binaries = [Path(p).resolve() for p in sys.argv[1:]] or [
        ROOT / "target/release/vaayu", ROOT / "target/release/vy"]
    jobs = [(binary, suite) for binary in binaries for suite in TOURS]
    jobs += [(binaries[0], suite) for suite in NEIGHBORS]
    results = []
    with ThreadPoolExecutor(max_workers=3) as pool:
        pending = [pool.submit(check, *job) for job in jobs]
        for future in as_completed(pending):
            result = future.result()
            results.append(result)
            print(json.dumps(result), flush=True)
    report = {"binary_sha256": {str(b.relative_to(ROOT)) if b.is_relative_to(ROOT) else str(b):
                               hashlib.sha256(b.read_bytes()).hexdigest() for b in binaries},
              "results": sorted(results, key=lambda r: (r["binary"], r["suite"]))}
    (OUT / "ui-results.json").write_text(json.dumps(report, indent=2) + "\n")
    return int(any(r["exit_code"] for r in results))


if __name__ == "__main__":
    sys.exit(main())
