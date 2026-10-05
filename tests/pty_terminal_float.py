#!/usr/bin/env python3
"""Open the Ctrl-\\ floating terminal and exercise real shell input/output."""
import os, pathlib, re, tempfile, time
import pexpect

binary = str(pathlib.Path(__import__("sys").argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix="vaayu-terminal-float-") as tmp:
    base = pathlib.Path(tmp)
    root = base / "project"
    root.mkdir()
    (root / "sample.txt").write_text("editor buffer stays here\n")
    config = base / "config" / "vaayu"
    config.mkdir(parents=True)
    (config / "config.toml").write_text("jk_escape=false\nnumber=false\n")
    env = os.environ | {
        "TERM": "xterm-256color",
        "XDG_CONFIG_HOME": str(base / "config"),
        "XDG_DATA_HOME": str(base / "data"),
        "SHELL": "/bin/sh",
    }
    child = pexpect.spawn(binary, [str(root / "sample.txt")], cwd=str(root), env=env,
                          dimensions=(30, 100), encoding="utf-8", timeout=5)
    try:
        time.sleep(.2)
        started = time.monotonic()
        child.send("\x1c")  # Ctrl-\\
        child.expect_exact("Terminal: /bin/sh")
        open_ms = (time.monotonic() - started) * 1000
        assert "\x1b[5;11H" in child.before, "the terminal float should be centered at 100x30"
        assert "╭" in child.before, "the terminal float's top-left border should be visible"
        time.sleep(.1)
        screen_tail = child.read_nonblocking(size=10000, timeout=.2)
        assert "│" in screen_tail, "the terminal float's side borders should be visible"
        time.sleep(.3)  # wait for the newly spawned shell to finish initializing
        child.send("printf 'FLOATING_TERMINAL_OK\\n'\r")
        marker = "FLOATING_TERMINAL_OK"
        ansi = r"(?:\x1b\[[0-?]*[ -/]*[@-~])*"
        child.expect(re.compile(ansi.join(re.escape(c) for c in marker)))
        child.send("\x1b")
        time.sleep(.2)
        child.send(":term\r")
        started = time.monotonic()
        child.expect_exact("Terminal: /bin/sh")
        command_ms = (time.monotonic() - started) * 1000
        child.send("\x1b")
        time.sleep(.1)
        child.send("\x17c")  # Ctrl-W c closes the split terminal
        print(f"Ctrl-\\ float rendered in {open_ms:.0f} ms; :term rendered in {command_ms:.0f} ms; shell input/output passed")

        if pathlib.Path("/bin/fish").exists():
            fish_env = env | {"SHELL": "/bin/fish"}
            fish = pexpect.spawn(binary, [str(root / "sample.txt")], cwd=str(root), env=fish_env,
                                 dimensions=(30, 100), encoding="utf-8", timeout=5)
            try:
                time.sleep(.2)
                fish.send("\x1c")
                fish.expect_exact("Terminal: /bin/fish")
                time.sleep(.15)
                fish_started = time.monotonic()
                fish.send("printf 'FISH_QUERY_HANDSHAKE_OK\\n'\r")
                marker = "FISH_QUERY_HANDSHAKE_OK"
                ansi = r"(?:\x1b\[[0-?]*[ -/]*[@-~])*"
                fish.expect(re.compile(ansi.join(re.escape(c) for c in marker)))
                fish_ready_ms = (time.monotonic() - fish_started) * 1000
                fish.send("\x1c")
                time.sleep(.1)
                fish.send("\x1c")
                fish.expect_exact("Terminal: /bin/fish")
                fish.send("\x1b")
            finally:
                fish.terminate(force=True)
            print(f"Fish terminal accepted input in {fish_ready_ms:.0f} ms; capability handshake and hide/reopen passed")
    finally:
        child.terminate(force=True)
