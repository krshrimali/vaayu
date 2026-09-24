#!/usr/bin/env python3
"""End-to-end checks for the AI sidebar + code tours:
  - :ai spawns the (stand-in) claude sidebar and the prompt is *deferred* until
    the CLI has started, then lands intact (not garbled by racing startup);
  - :q on the AI bar closes just that pane (even with an unsaved buffer), not
    the whole editor;
  - Ctrl-W navigates out of the terminal pane, and `jk` leaves Terminal mode;
  - :tournew -> :w prompts for a name -> the prompt is sent to claude;
  - :tours lists a .tour and Enter starts it, showing the step panel.
The stand-in "claude" prints a banner then `cat`s stdin, so pasted input is
echoed back to the screen where we can assert on it.
"""
import codecs, fcntl, os, pathlib, pty, select, struct, sys, termios, time
import pyte

binary = str(pathlib.Path(sys.argv[1]).resolve())


def run(cols, rows):
    import tempfile
    with tempfile.TemporaryDirectory(prefix="vaayu-aitour-") as tmp:
        root = pathlib.Path(tmp)
        (root / "config/vaayu").mkdir(parents=True)
        # Stand in for `claude`: print a banner (so output_revision bumps and the
        # deferred send fires ~800ms later), then cat stdin back so we see it.
        (root / "config/vaayu/config.toml").write_text(
            'jk_escape=true\nclipboard_unnamedplus=false\nnumber=false\n\n'
            '[agent_commands]\n'
            'claude = ["sh", "-c", "printf CLAUDE_READY; cat"]\n'
        )
        (root / ".tours").mkdir()
        (root / "main.rs").write_text("fn main() {}\nlet x = 1;\nlet y = 2;\n")
        (root / ".tours/intro.tour").write_text(
            '{"title":"Intro","steps":['
            '{"file":"main.rs","line":1,"description":"TOURDESC_ONE the entry point"},'
            '{"file":"main.rs","line":3,"description":"TOURDESC_TWO a binding"}]}')
        pid, fd = pty.fork()
        if pid == 0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color", XDG_CONFIG_HOME=str(root / "config"))
            os.execv(binary, [binary, "main.rs"])
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        screen = pyte.Screen(cols, rows)
        stream = pyte.Stream(screen)
        dec = codecs.getincrementaldecoder("utf-8")("replace")

        def drain(seconds=0.25):
            end = time.monotonic() + seconds
            while time.monotonic() < end:
                r, _, _ = select.select([fd], [], [], min(0.02, max(0, end - time.monotonic())))
                if r:
                    try:
                        data = os.read(fd, 65536)
                    except OSError:
                        break
                    if not data:
                        break
                    stream.feed(dec.decode(data))

        def key(s, seconds=0.25):
            os.write(fd, s.encode())
            drain(seconds)

        def text():
            return "\n".join(screen.display)

        def wait_for(pred, timeout=6):
            end = time.monotonic() + timeout
            while time.monotonic() < end:
                drain(0.1)
                if pred():
                    return True
            return False

        try:
            drain(1.0)
            # Modify the buffer so the :q-on-sidebar test also proves the
            # unsaved-changes guard does not block closing a split.
            key("ix\x1b")  # insert 'x', back to Normal; buffer now modified

            # --- :ai deferred send ---
            key(":ai explain this code\r", 0.4)
            assert wait_for(lambda: "CLAUDE_READY" in text()), \
                ("the claude sidebar should have started\n" + text())
            # The prompt must actually arrive (deferred until the CLI was ready).
            assert wait_for(lambda: "explain this code" in text()), \
                ("the AI prompt should be delivered after the CLI started\n" + text())

            # --- :q on the AI bar closes just the pane, not the editor ---
            key("\x1b", 0.2)          # leave Terminal mode -> Normal on the sidebar
            key(":q\r", 0.4)          # close the sidebar pane
            assert wait_for(lambda: "CLAUDE_READY" not in text()), \
                (":q on the AI bar should close the sidebar pane\n" + text())
            # ...and the editor is still alive with our buffer.
            assert wait_for(lambda: "fn main()" in text()), \
                (":q must not quit the whole editor\n" + text())

            # --- :tours list + Enter starts the tour, showing the panel ---
            key(":tours\r", 0.4)
            assert wait_for(lambda: "Intro" in text()), \
                (":tours should list the tour\n" + text())
            key("\r", 0.4)            # start the selected tour
            assert wait_for(lambda: "TOURDESC_ONE" in text() and "step 1/2" in text()), \
                ("the tour panel should show step 1's description\n" + text())
            key(":tournext\r", 0.3)
            assert wait_for(lambda: "TOURDESC_TWO" in text() and "step 2/2" in text()), \
                (":tournext should advance the tour\n" + text())
            key(":tourend\r", 0.3)
            assert wait_for(lambda: "TOURDESC_TWO" not in text()), \
                (":tourend should dismiss the panel\n" + text())

            # --- :tournew -> :w prompts for a name -> sends to claude ---
            key(":tournew\r", 0.3)
            key("iWalk me through main\x1b", 0.3)   # write the prompt
            key(":w\r", 0.3)                         # should prefill :toursave, not write a file
            assert wait_for(lambda: text().rstrip().splitlines()[-1].startswith(":toursave")), \
                (":w on the tour prompt should ask for a name via :toursave\n" + text())
            key("mytour\r", 0.4)                     # complete the name -> :toursave mytour
            assert wait_for(lambda: "Walk me through main" in text()), \
                ("the tour prompt should be sent to the claude sidebar\n" + text())

            key(":qa!\r", 0.3)
            end = time.monotonic() + 3
            while time.monotonic() < end:
                done, status = os.waitpid(pid, os.WNOHANG)
                if done:
                    pid = None
                    break
                drain(0.05)
        finally:
            if pid is not None:
                try:
                    os.kill(pid, 9)
                    os.waitpid(pid, 0)
                except OSError:
                    pass
    print(f"AI + tours PTY passed: {cols}x{rows}")


for cols, rows in [(120, 30), (180, 50)]:
    run(cols, rows)
