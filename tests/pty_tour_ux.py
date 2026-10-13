#!/usr/bin/env python3
"""Tour reading, recovery, scratch preservation, catalog and mouse regressions.

Usage: python tests/pty_tour_ux.py BINARY [artifact-directory]
"""
import json
from pathlib import Path
import sys
import tempfile

import pty_tour_visuals as ui


def run(binary, cols, rows, artifacts=None):
    with tempfile.TemporaryDirectory(prefix="vaayu-tour-ux-") as tmp:
        root = Path(tmp)
        source = ui.write_fixture(root)
        path = root / ".tours/visual.tour"
        tour = json.loads(path.read_text())
        tour["title"] = "A long tour title about tracing input all the way to a screen update"
        tour["steps"][0]["description"] = "# Read the lesson\n\nLesson first paragraph.\n\n" + "\n".join(f"- Reading row {i:02}" for i in range(40))
        path.write_text(json.dumps(tour))
        (root / ".tours/broken.tour").write_text(json.dumps({"title": "Broken", "steps": [
            {"file": "missing.rs", "line": 1, "description": "Missing source lesson"},
            {"file": "demo.txt", "line": 2, "description": "Valid next step"}]}))
        (root / ".tours/empty.tour").write_text('{"title":"Empty","steps":[]}')
        (root / ".tours/invalid.tour").write_text('{"title":')
        session = ui.Session(binary, root, cols, rows)
        name = f"{cols}x{rows}"

        def capture(label):
            session.screenshot(artifacts, f"{name}-{label}")

        def pinned(step):
            assert f"step {step}/3" in session.screen.display[0], session.text()
            assert "Help" in session.screen.display[-2], session.text()

        try:
            session.key(":tour visual\r")
            pinned(1)
            session.key("K")
            session.wait("Read the lesson")
            session.wait("Lesson first paragraph")
            assert "Lesson" in session.screen.display[0], session.text()
            if rows <= 16:
                assert "read_input();" not in session.text(), session.text()
                assert len(session.screen.display[1:-3]) >= 4
            capture("reading")
            # The visible Next control invokes the same action as ]t.
            session.key("K")
            session.key(f"\x1b[<0;12;{rows-1}M\x1b[<0;12;{rows-1}m")
            pinned(2)
            session.key("[tK")
            if cols >= 120:
                footer = session.screen.display[-2]
                x = footer.index("Diff") + 1
                session.key(f"\x1b[<0;{x};{rows-1}M\x1b[<0;{x};{rows-1}m", .2)
                session.wait("Tour Diff")
                session.key("\x0f")
            session.key("G")
            pinned(1)
            session.wait("validates targets.")
            capture("scrolled")
            session.key(":tourmarkdown off\r")
            session.key("ggiPRESERVE_THIS_NOTE\r\x1b")
            for keys in (",vf", "\x0f", ",vd", "\x0f", ",vi", ",vi"):
                session.key(keys, .2)
                session.wait("PRESERVE_THIS_NOTE")
                if cols >= 80:
                    assert "[+]" in session.text(), session.text()
            session.key("]h")
            session.key("K")
            session.wait("PRESERVE_THIS_NOTE")
            session.key(":tourmarkdown on\r")
            session.key("g")  # Rendering retains the previous rendered viewport.
            session.wait("PRESERVE_THIS_NOTE")
            session.resize(40, 12)
            session.wait("PRESERVE_THIS_NOTE")
            session.resize(cols, rows)
            session.wait("PRESERVE_THIS_NOTE")
            capture("scratch-preserved")
            session.key("]t")
            session.wait("Plain step")
            pinned(2)
            session.key(":toursteps\r")
            session.wait("[2] Plain step")
            assert "[2]" in session.screen.display[session.screen.cursor.y], session.text()
            assert "�" not in session.screen.display[2], session.text()
            capture("steps")
            session.key("q")
            session.key("]q")
            session.key(":tours\r", .2)
            session.wait("Resume step 2")
            session.wait("Invalid tour")
            session.wait("Empty tour")
            session.wait("3 steps")
            session.wait("1 missing source")
            capture("catalog")
            session.key("fvisual\r")
            session.key("K")
            pinned(2)
            session.key(":tours\r", .2)
            session.key("fvisual\rr")
            pinned(1)
            session.key(":tourhelp\r")
            session.wait("Tour help")
            session.key("jjjjj")
            session.wait("Source view")
            capture("help")
            session.key("q")
            session.key(":tour broken\r")
            session.wait("Source file not found")
            session.wait("Retry")
            assert "Source" not in session.screen.display[-2], session.text()
            assert not (root / "missing.rs").exists()
            if "Raw scratch" in session.screen.display[0] or "Lesson" in session.screen.display[0]:
                session.key("K")
            session.key(":tourview source\r")
            session.wait("Source unavailable")
            session.key("iSHOULD_NOT_EDIT\x1b")
            assert (root / "demo.txt").read_text() == source
            session.key(":tourcheck\r", .2)
            session.wait("missing.rs")
            session.wait("Tour check")
            capture("missing-source-check")
            session.key("q")
            (root / "missing.rs").write_text("RECOVERED_SOURCE\n")
            session.key(":tourretry\r")
            session.wait("RECOVERED_SOURCE")
            assert "Source file not found" not in session.screen.display[0]
            capture("recovered")
            session.key(":touredit\r")
            assert "step 1/2" not in session.screen.display[0], session.text()
            session.wait("broken.tour")
            session.key(":tour visual\r")
            session.key("]t]t]t")
            session.wait("Tour complete")
            assert "Finish" in session.text() or "End" in session.screen.display[-2]
            capture("complete")
            session.key(":tourexample\r")
            session.wait("example.tour")
            assert "step 3/3" not in session.screen.display[0], session.text()
            for item in (root / ".tours").glob("*.tour"):
                item.unlink()
            session.key(":tours\r", .2)
            session.wait("No tours yet")
            session.wait("minimal example")
            capture("empty")
            session.key("j\r")
            assert (root / ".tours/example.tour").is_file()
            json.loads((root / ".tours/example.tour").read_text())
            session.wait("example.tour")
            assert (root / "demo.txt").read_text() == source
        finally:
            session.close()
    print(f"Tour UX PTY passed: {cols}x{rows}", flush=True)


if __name__ == "__main__":
    binary = str(Path(sys.argv[1]).resolve())
    artifacts = Path(sys.argv[2]).resolve() if len(sys.argv) > 2 else None
    for cols, rows in ((40, 12), (80, 24), (120, 30), (180, 50)):
        run(binary, cols, rows, artifacts)
