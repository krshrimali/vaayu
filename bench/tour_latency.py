#!/usr/bin/env python3
"""Compare key-to-first-output latency before/after optional tour visuals.

Runs isolated 50k-line fixtures, alternating binaries across three rounds.
This measures the first PTY response, not completed-frame or display latency.
Run after builds/tests finish to avoid contaminating the measurements.

Usage: python bench/tour_latency.py BEFORE AFTER OUTPUT.json
"""
import importlib.util
import hashlib
import json
import os
from pathlib import Path
import select
import statistics
import sys
import tempfile
import time

repo = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("tour_ui", repo / "tests/pty_tour_visuals.py")
ui = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ui)


def percentile(values, p):
    ordered = sorted(values)
    k = (len(ordered) - 1) * p
    lo = int(k)
    return ordered[lo] + (ordered[min(lo + 1, len(ordered) - 1)] - ordered[lo]) * (k - lo)


def session_samples(binary, active_tour):
    with tempfile.TemporaryDirectory(prefix="vaayu-tour-bench-") as tmp:
        root = Path(tmp)
        ui.write_fixture(root, large=True)
        session = ui.Session(binary, root, 100, 30)
        measurements = {"movement": [], "typing": []}
        if active_tour:
            measurements["step_ready"] = []
        try:
            if active_tour:
                session.key(":tour visual\r", .25)
            else:
                session.key("j", .1)
            session.drain(.1)

            def sample(category, key):
                started = time.perf_counter_ns()
                os.write(session.fd, key.encode())
                ready, _, _ = select.select([session.fd], [], [], 2)
                assert ready, (binary, active_tour, category, "input timed out")
                measurements[category].append((time.perf_counter_ns() - started) / 1e6)
                session.drain(.003)

            for _ in range(18):
                for key in "jjjjjkkkkk":
                    sample("movement", key)
            session.key("A", .05)
            for key in "abcdefghijklmnopqrstuvwxyz_0123456789":
                sample("typing", key)
            session.key("\x1b", .25)
            session.key("u", .25)
            if active_tour:
                for _ in range(12):
                    for key, label in (("]t", "Plain step"), ("[t", "Follow the edit.")):
                        started = time.perf_counter_ns()
                        os.write(session.fd, key.encode())
                        while label not in session.text():
                            assert time.perf_counter_ns() - started < 2e9, (key, session.text())
                            session.drain(.001)
                        measurements["step_ready"].append((time.perf_counter_ns() - started) / 1e6)
                        session.drain(.003)
        finally:
            session.close()
        return measurements


def run(before, after, output):
    samples = {}
    for round_index in range(3):
        order = [("before", before), ("after", after)]
        if round_index % 2:
            order.reverse()
        for label, binary in order:
            for active in (False, True):
                key = label + ("_tour" if active else "_plain")
                values = session_samples(binary, active)
                aggregate = samples.setdefault(key, {category: [] for category in values})
                for category in values:
                    aggregate[category].extend(values[category])
                print(f"Round {round_index + 1}: {key}", flush=True)
    result = {
        "metric": "key-to-first-PTY-output, milliseconds; not completed-frame latency",
        "step_ready_metric": "key sequence to expected step text in the parsed PTY screen, milliseconds",
        "fixture": "50,005 lines; text buffer; 100x30 terminal; isolated configuration/state",
        "rounds": 3,
        "binaries": {"before": before, "after": after},
        "sha256": {"before": hashlib.sha256(Path(before).read_bytes()).hexdigest(),
                   "after": hashlib.sha256(Path(after).read_bytes()).hexdigest()},
        "results": {label: {category: {"count": len(values), "p50_ms": statistics.median(values),
                                     "p95_ms": percentile(values, .95), "p99_ms": percentile(values, .99)}
                             for category, values in categories.items()}
                    for label, categories in samples.items()}
    }
    Path(output).write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result["results"], indent=2))


if __name__ == "__main__":
    run(str(Path(sys.argv[1]).resolve()), str(Path(sys.argv[2]).resolve()), sys.argv[3])
