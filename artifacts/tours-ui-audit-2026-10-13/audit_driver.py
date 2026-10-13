#!/usr/bin/env python3
"""Capture exploratory tour UI evidence using the application's real PTY.

Run with the dependencies in tests/requirements.txt:
    python artifacts/tours-ui-audit-2026-10-13/audit_driver.py target/release/vaayu
Screenshots reproduce terminal character/color cells, not an OS terminal window.
"""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "tests"))
import pty_tour_visuals as ui

BINARY = str(Path(sys.argv[1]).resolve())
OUT = Path(__file__).resolve().parent
EVENTS = []
OBSERVATIONS = []

DESCRIPTION = """# Follow one keypress

Understand how **input**, the `Buffer`, and rendering work together.

1. Decode the key and choose the current mode's handler.
2. Apply the edit at the cursor.
3. Request a redraw of the affected rows.

## Why this matters

The input handler delegates document changes to the buffer. The renderer
then reads the new state, instead of applying the edit itself.

```rust
let key = read_key()?;
editor.handle_key(key);
renderer.draw(&editor)?;
```

## Try it yourself

Move to the next highlight and follow the call into the renderer.
"""


def fixture(root):
    ui.write_fixture(root)
    path = root / ".tours/visual.tour"
    tour = json.loads(path.read_text())
    tour["title"] = "How a keypress becomes a screen update"
    tour["steps"][0]["description"] = DESCRIPTION
    tour["steps"][1]["description"] = "The renderer updates only the affected rows."
    tour["steps"][2] = {
        "file": "other.txt", "line": 2,
        "description": "You are ready to trace another edit.",
    }
    path.write_text(json.dumps(tour))
    (root / ".tours/empty.tour").write_text('{"title":"Empty tour","steps":[]}')
    (root / ".tours/malformed.tour").write_text('{"title": "Invalid JSON",')
    (root / ".tours/broken.tour").write_text(json.dumps({
        "title": "Broken targets", "steps": [
            {"file": "demo.txt", "line": 2, "description": "Known good step"},
            {"file": "missing.rs", "line": 1, "description": "MISSING_FILE_STEP"},
            {"file": "demo.txt", "line": 999, "pattern": "removed_function();",
             "description": "UNRESOLVED_PATTERN_STEP"},
        ],
    }))


def record(session, scenario, name, keys=None):
    if keys is not None:
        session.key(keys, .16)
    session.screenshot(OUT / "captures", f"{scenario}-{name}")
    EVENTS.append({
        "scenario": scenario, "capture": name, "keys": repr(keys),
        "size": [session.cols, session.rows],
        "cursor": [session.screen.cursor.x, session.screen.cursor.y],
    })


def observe(name, value):
    OBSERVATIONS.append({"observation": name, "value": value})


for cols, rows in ((40, 12), (80, 24), (120, 30), (180, 50)):
    scenario = f"{cols}x{rows}"
    with tempfile.TemporaryDirectory(prefix="vaayu-tours-explore-") as temp:
        root = Path(temp)
        fixture(root)
        original_source = (root / "demo.txt").read_bytes()
        original_tour = (root / ".tours/visual.tour").read_bytes()
        session = ui.Session(BINARY, root, cols, rows)
        try:
            record(session, scenario, "list", ":tours\r")
            record(session, scenario, "default-raw", ":tour visual\r")
            record(session, scenario, "explanation-focused", "K")
            record(session, scenario, "rendered", ",vm")
            record(session, scenario, "scrolled-end", "G")
            observe(f"{scenario}: step progress visible at explanation end",
                    "step 1/3" in session.text())
            record(session, scenario, "flow", ",vf")
            record(session, scenario, "source-return", "\x0f")
            record(session, scenario, "diff", ",vd")
            record(session, scenario, "diff-source-return", "\x0f")
            record(session, scenario, "steps", ":toursteps\r")
            # Dismiss the results view and explicitly return to source.
            session.key("\x1b")
            session.key(":tourview source\r")
            session.key("]q")
            record(session, scenario, "restart", ":tour visual\r")
            record(session, scenario, "last-step", "]t]t")
            record(session, scenario, "past-end", "]t")
            observe(f"{scenario}: final next message", session.screen.display[-1].strip())
            session.key("]q")
            record(session, scenario, "resume", ":tour\r")
            if (cols, rows) == (120, 30):
                record(session, scenario, "mouse-explanation", "\x1b[<0;12;23M\x1b[<0;12;23m")
                session.resize(40, 12)
                record(session, scenario, "resize-small")
                session.resize(120, 30)
                record(session, scenario, "resize-restored")
            observe(f"{scenario}: source and tour unchanged",
                    (root / "demo.txt").read_bytes() == original_source
                    and (root / ".tours/visual.tour").read_bytes() == original_tour)
        finally:
            session.close()

with tempfile.TemporaryDirectory(prefix="vaayu-tours-errors-") as temp:
    root = Path(temp)
    fixture(root)
    session = ui.Session(BINARY, root, 120, 30)
    try:
        record(session, "errors", "empty-tour", ":tour empty\r")
        record(session, "errors", "malformed-tour", ":tour malformed\r")
        record(session, "errors", "missing-named-tour", ":tour nonexistent\r")
        session.key(":tour broken\r")
        session.key(":tourcheck\r")
        session.wait("Tour check —")
        record(session, "errors", "validation-before-opening-missing-file")
        session.key("\x1b")
        record(session, "errors", "missing-file", "]t")
        observe("missing source opened as apparently normal step", "step 2/3" in session.text())
        observe("opening missing source creates a file on disk", (root / "missing.rs").exists())
        record(session, "errors", "missing-anchor", "]t")
        record(session, "errors", "missing-anchor-after-move", "j")
        record(session, "errors", "validation", ":tourcheck\r")
        observe("validation still identifies missing source after it is opened",
                "missing.rs" in session.text())
        session.key("\x1b")
        session.key("]q")
        session.key(":tour visual\r")
        session.key("K")
        session.key(":tourmarkdown off\r")
        record(session, "editing", "scratch-note", "ggiLOCAL_NOTE_MUST_SURVIVE\r\x1b")
        session.key(",vf")
        record(session, "editing", "scratch-after-view-switch")
        session.key("gg")
        observe("scratch note survives Flow switch", "LOCAL_NOTE_MUST_SURVIVE" in session.text())
        record(session, "editing", "scratch-after-source-return", "\x0f")
        for action_name, action in (("highlight", "]h"), ("indicators", ",vi"),
                                    ("markdown", ":tourmarkdown on\r")):
            session.key("]q")
            session.key(":tour visual\r")
            session.key("K")
            session.key(":tourmarkdown off\r")
            session.key("ggiLOCAL_NOTE_MUST_SURVIVE\r\x1b")
            assert "LOCAL_NOTE_MUST_SURVIVE" in session.text(), session.text()
            record(session, "editing", f"scratch-after-{action_name}", action)
            session.key("gg" if action_name != "markdown" else "g")
            observe(f"scratch note survives {action_name}", "LOCAL_NOTE_MUST_SURVIVE" in session.text())
        session.key("]q")
        for path in (root / ".tours").glob("*.tour"):
            path.unlink()
        record(session, "errors", "no-tours", ":tours\r")
    finally:
        session.close()

metadata = {
    "commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip(),
    "binary_sha256": hashlib.sha256(Path(BINARY).read_bytes()).hexdigest(),
    "events": EVENTS,
    "observations": OBSERVATIONS,
}
(OUT / "exploratory-results.json").write_text(json.dumps(metadata, indent=2) + "\n")
print(json.dumps(OBSERVATIONS, indent=2))
print(f"Captured {len(EVENTS)} terminal frames in {OUT / 'captures'}")
