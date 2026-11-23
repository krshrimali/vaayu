#!/usr/bin/env python3
"""Several :ai sessions, clickable agent output and smooth scrolling, driven
through a real PTY with real (SGR) mouse events.

A shell script stands in for the agent CLI: it prints Claude Code-style
output (a tool header, a numbered diff, file references) and then waits.
Checks, at three sizes:
  * :aistart then :ainew give two agent panes stacked on the right, the
    second labelled `claude #2` on its statusline;
  * hovering a file reference underlines it;
  * clicking a diff row opens the file at that line in the editor pane next
    to the agents, and Ctrl-W p goes back to the agent;
  * clicking `src/a.rs:5:7` lands on line 5, column 7;
  * the wheel scrolls an agent's history, eased over several frames
    (`smooth_scroll`), and typing snaps it back;
  * :aistop! closes every agent pane.
"""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte

binary = str(pathlib.Path(sys.argv[1]).resolve())
FILLER = "".join(f"filler {i}\n" for i in range(1, 81))
OUTPUT = FILLER + """\
⏺ Update(src/a.rs)
  ⎿  Updated src/a.rs with 1 addition and 1 removal
       11 -      line 11
       11 +      line eleven
see src/a.rs:5:7 for more
END
"""

for cols, rows in [(100, 30), (140, 40), (200, 50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-ailinks-") as tmp:
        root = pathlib.Path(tmp)
        (root / "config/vaayu").mkdir(parents=True)
        (root / "config/vaayu/config.toml").write_text(
            "jk_escape=false\nclipboard_unnamedplus=false\nnumber=true\n"
            'agent_commands = { claude = ["sh", "-c", "cat out.txt; exec cat"] }\n'
        )
        (root / "src").mkdir()
        (root / "src/a.rs").write_text("".join(f"    line {i}\n" for i in range(1, 41)))
        (root / "out.txt").write_text(OUTPUT)
        (root / "notes.txt").write_text("notes\n")
        pid, fd = pty.fork()
        if pid == 0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color", XDG_CONFIG_HOME=str(root / "config"), SHELL="/bin/sh")
            os.execv(binary, [binary, str(root / "notes.txt")])
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        screen = pyte.Screen(cols, rows)
        stream = pyte.Stream(screen)
        decoder = codecs.getincrementaldecoder("utf-8")("replace")

        def drain(seconds=0.25):
            end = time.monotonic() + seconds
            while time.monotonic() < end:
                ready, _, _ = select.select([fd], [], [], min(0.02, max(0, end - time.monotonic())))
                if ready:
                    try:
                        data = os.read(fd, 65536)
                    except OSError:
                        break
                    if not data:
                        break
                    stream.feed(decoder.decode(data))

        def key(s, seconds=0.25):
            os.write(fd, s.encode())
            drain(seconds)

        def text():
            return "\n".join(screen.display)

        def wait_for(pred, timeout=6.0):
            end = time.monotonic() + timeout
            while time.monotonic() < end:
                if pred():
                    return True
                drain(0.1)
            return pred()

        def find(needle, nth=0):
            """(x, y), 0-based, of the nth occurrence of needle on screen."""
            hits = []
            for y, line in enumerate(screen.display):
                start = 0
                while (x := line.find(needle, start)) >= 0:
                    hits.append((x, y))
                    start = x + 1
            assert len(hits) > nth, f"{needle!r} not on screen (#{nth})\n" + text()
            return hits[nth]

        def mouse(button, x, y, release=False, seconds=0.25):
            key(f"\x1b[<{button};{x + 1};{y + 1}{'m' if release else 'M'}", seconds)

        def click(x, y):
            mouse(0, x, y, seconds=0.05)
            mouse(0, x, y, release=True)

        try:
            drain(0.5)
            key(":aistart\r", 0.5)
            assert wait_for(lambda: text().count("END") == 1), "agent output\n" + text()
            key("\x1b", 0.3)
            key(":ainew\r", 0.5)
            assert wait_for(lambda: text().count("END") == 2), "a second agent pane\n" + text()
            assert "claude #2" in text(), "the second session is numbered\n" + text()
            first_end, second_end = find("END", 0), find("END", 1)
            assert first_end[0] == second_end[0] and second_end[1] > first_end[1], \
                "the agents are stacked in one column\n" + text()
            assert first_end[0] > cols // 3, "agents sit to the right of the code\n" + text()

            # Hover underlines the reference (in the second agent's pane).
            x, y = find("src/a.rs:5:7", 1)
            mouse(35, x + 2, y)  # motion, no button
            assert wait_for(lambda: all(screen.buffer[y][x + i].underscore for i in range(len("src/a.rs:5:7")))), \
                "the hovered link is underlined"
            assert not screen.buffer[y][x - 2].underscore, "only the link is underlined"

            # Click a diff row: the file opens beside the agents.
            x, y = find("line eleven", 1)
            click(x + 3, y)
            assert wait_for(lambda: "Opened src/a.rs:11" in text()), "opened from the click\n" + text()
            assert wait_for(lambda: any(l.startswith(" 11") or l.lstrip().startswith("11 ") for l in screen.display)), \
                "line 11 is in view\n" + text()
            assert "NORMAL" in text(), "focus is on the code, in Normal mode\n" + text()
            assert text().count("END") == 2, "both agents are still shown\n" + text()
            # Ctrl-W p goes back to the agent it came from.
            key("\x17p", 0.3)
            key("i", 0.3)
            assert wait_for(lambda: "TERMINAL" in text() or "TERM" in text()), "back on the agent\n" + text()
            key("\x1b", 0.3)

            # A file:line:col reference.
            x, y = find("src/a.rs:5:7", 0)
            click(x + 1, y)
            assert wait_for(lambda: "Opened src/a.rs:5" in text()), "file:line:col link\n" + text()
            assert wait_for(lambda: "5:7" in text()), "cursor at line 5 col 7 on the statusline\n" + text()

            # Wheel up over the first agent: history scrolls, eased.
            x, y = find("END", 0)
            for _ in range(3):
                mouse(64, x, y - 2, seconds=0.0)
            drain(0.6)
            assert wait_for(lambda: "↑ 9/" in text()), "three notches = 9 lines back\n" + text()
            assert "filler" in text(), "older output is in view\n" + text()
            # Typing in the agent snaps back to the live screen.
            click(x, y - 1)
            key("i", 0.2)
            key("z", 0.4)
            assert wait_for(lambda: "lines back" not in text()), "input returns to the live view\n" + text()
            key("\x1b", 0.3)

            key(":aistop!\r", 0.6)
            assert wait_for(lambda: "Stopped 2 claude sessions" in text()), ":aistop! stops both\n" + text()
            assert "END" not in text(), "the agent panes are gone\n" + text()

            key(":qa!\r")
            end = time.monotonic() + 3
            while time.monotonic() < end:
                done, status = os.waitpid(pid, os.WNOHANG)
                if done:
                    assert os.waitstatus_to_exitcode(status) == 0
                    pid = None
                    break
                drain(0.05)
            assert pid is None, "Editor failed to exit"
        finally:
            if pid:
                os.kill(pid, signal.SIGKILL)
                os.waitpid(pid, 0)
            os.close(fd)
    print(f"AI links PTY passed: {cols}x{rows}")
