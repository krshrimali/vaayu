#!/usr/bin/env python3
"""Exercise tour focus, navigation and clipboard actions in a real PTY."""
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import time

import pexpect

binary = str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix="vaayu-tour-focus-") as tmp:
    base = pathlib.Path(tmp)
    root = base / "project"
    root.mkdir()
    (root / "src").mkdir()
    source = root / "src" / "tour.rs"
    source.write_text("fn tracked_step() {}\n")
    untracked = root / "scratch.rs"
    untracked.write_text("fn untracked_step() {}\n")
    tours = root / ".tours"
    tours.mkdir()
    description = "Tracked explanation\n" + "\n".join(
        f"EXPLANATION_LINE_{i:02}" for i in range(1, 41)
    )
    tour = {
        "title": "Focus tour",
        "steps": [
            {"file": "src/tour.rs", "line": 1, "description": description},
            {"file": "scratch.rs", "line": 1, "description": "Untracked explanation"},
        ],
    }
    (tours / "intro.tour").write_text(json.dumps(tour))
    for args in (
        ["git", "init", "-q"],
        ["git", "config", "user.name", "Vaayu UI test"],
        ["git", "config", "user.email", "vaayu-ui@example.invalid"],
        ["git", "remote", "add", "origin", "git@github.com:acme/widgets.git"],
        ["git", "add", "src/tour.rs", ".tours/intro.tour"],
        ["git", "commit", "-qm", "tour fixture"],
    ):
        subprocess.run(args, cwd=root, check=True)

    config = base / "config" / "vaayu"
    config.mkdir(parents=True)
    (config / "config.toml").write_text("jk_escape=false\nnumber=false\n")
    fake_bin = base / "bin"
    fake_bin.mkdir()
    copy_cmd = fake_bin / "wl-copy"
    copy_cmd.write_text(
        "#!/usr/bin/env python3\n"
        "import os, sys\n"
        "open(os.environ['VAAYU_CLIPBOARD_CAPTURE'], 'wb').write(sys.stdin.buffer.read())\n"
    )
    copy_cmd.chmod(0o755)
    clipboard = base / "clipboard.txt"
    env = os.environ | {
        "TERM": "xterm-256color",
        "XDG_CONFIG_HOME": str(base / "config"),
        "XDG_DATA_HOME": str(base / "data"),
        "WAYLAND_DISPLAY": "vaayu-ui-test",
        "VAAYU_CLIPBOARD_CAPTURE": str(clipboard),
        "PATH": str(fake_bin) + os.pathsep + os.environ.get("PATH", ""),
    }
    child = pexpect.spawn(
        binary,
        [str(source)],
        cwd=str(root),
        env=env,
        dimensions=(24, 110),
        encoding="utf-8",
        timeout=6,
    )

    def leader(sequence):
        child.send(",")
        time.sleep(0.12)
        child.send(sequence)

    def wait_clipboard(predicate, timeout=3):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            if clipboard.exists():
                value = clipboard.read_text()
                if predicate(value):
                    return value
            time.sleep(0.03)
        raise AssertionError("clipboard did not receive the expected tour content")

    try:
        time.sleep(0.25)
        child.send(":tours\r")
        child.expect_exact("Tours — Enter starts · K explanation")
        child.send("K")  # Start the selected tour directly in its explanation.
        child.expect_exact("Tour explanation focused")
        child.send("\x04\x04")  # Ctrl-D scrolls the normal explanation buffer.
        child.expect_exact("23:1")
        child.send("\x15\x15")  # Ctrl-U returns toward the start.
        child.expect_exact("1:1")

        leader("vy")
        child.expect_exact("Copied tour step explanation")
        copied_step = wait_clipboard(lambda value: "GitHub:" in value)
        assert "File: src/tour.rs:1-1" in copied_step
        assert "https://github.com/acme/widgets/blob/" in copied_step
        assert "Tracked explanation" in copied_step

        child.send("]t")
        child.expect_exact("Untracked explanation")
        leader("vy")
        child.expect_exact("Copied tour step explanation")
        copied_untracked = wait_clipboard(lambda value: "Untracked explanation" in value)
        assert "File: scratch.rs:1-1" in copied_untracked
        assert "GitHub:" not in copied_untracked, "untracked source must not get a false permalink"

        leader("vY")
        child.expect_exact("Copied the full tour as JSON")
        copied_tour = wait_clipboard(lambda value: '"steps"' in value)
        assert json.loads(copied_tour)["title"] == "Focus tour"
        assert len(json.loads(copied_tour)["steps"]) == 2

        child.send("[q")  # Restart while explanation buffer is focused.
        child.expect_exact("Tracked explanation")
        child.send("K")
        child.expect_exact("Tour source focused")
        child.send("]q")
        child.expect_exact("Tour ended")
        child.send(":qa!\r")
        child.expect(pexpect.EOF)
    finally:
        if child.isalive():
            child.terminate(force=True)

print("Tour focus PTY passed: Vim navigation, step/full copy, valid link, restart and end")
