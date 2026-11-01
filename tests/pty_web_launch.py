#!/usr/bin/env python3
"""`:web` from the terminal editor: starts one detached session per project, opens
the browser on it, reuses it afterwards and survives the terminal going away."""
import codecs, fcntl, http.client, json, os, pathlib, pty, re, select, signal, stat, struct, sys, tempfile, termios, time
import pyte

binary = str(pathlib.Path(sys.argv[1]).resolve())
failures = []


def check(name, ok, detail=""):
    print(f"  {'ok  ' if ok else 'FAIL'} {name} {'' if ok else detail}")
    if not ok:
        failures.append(name)


with tempfile.TemporaryDirectory(prefix="vaayu-web-launch-") as directory:
    tmp = pathlib.Path(directory)
    project = tmp / "proj"
    project.mkdir()
    (project / "main.txt").write_text("from the terminal\n")
    (project / "second.txt").write_text("second file\n")
    (tmp / "cfg/vaayu").mkdir(parents=True)
    (tmp / "cfg/vaayu/config.toml").write_text("jk_escape=false\nprogress=false\nclipboard_unnamedplus=false\n")
    opened = tmp / "opened.log"
    fake_browser = tmp / "browser.sh"
    fake_browser.write_text(f'#!/bin/sh\necho "$1" >> {opened}\n')
    fake_browser.chmod(fake_browser.stat().st_mode | stat.S_IXUSR)

    cols, rows = 150, 24
    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(project)
        os.environ.update(TERM="xterm-256color", XDG_CONFIG_HOME=str(tmp / "cfg"), XDG_DATA_HOME=str(tmp / "data"),
                          HOME=str(tmp), BROWSER=str(fake_browser))
        os.environ.pop("DISPLAY", None)
        os.environ.pop("WAYLAND_DISPLAY", None)
        os.execv(binary, [binary, "main.txt"])
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
    screen = pyte.Screen(cols, rows)
    stream = pyte.Stream(screen)
    decoder = codecs.getincrementaldecoder("utf-8")("replace")

    def drain(seconds=0.2):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            ready, _, _ = select.select([fd], [], [], 0.02)
            if ready:
                try:
                    stream.feed(decoder.decode(os.read(fd, 65536)))
                except OSError:
                    return

    def wait_for(needle, timeout=8):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            drain(0.05)
            if needle in "\n".join(screen.display):
                return True
        return False

    def wait_opened(count, timeout=8):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            lines = opened.read_text().splitlines() if opened.exists() else []
            if len(lines) >= count:
                return lines
            time.sleep(0.05)
        return opened.read_text().splitlines() if opened.exists() else []

    def ping(port, token):
        connection = http.client.HTTPConnection("127.0.0.1", port, timeout=3)
        connection.request("GET", f"/api/ping?token={token}")
        status = connection.getresponse().status
        connection.close()
        return status

    drain(0.4)
    state = project / ".vaayu/web.json"
    try:
        os.write(fd, b":web\r")
        check(":web reports where the session is", wait_for("Web editor: http://127.0.0.1"), screen.display[-1])
        check("the terminal editor stays usable", "main.txt" in "\n".join(screen.display))
        lines = wait_opened(1)
        check(":web opens the browser", len(lines) == 1, lines)
        session = json.loads(state.read_text())
        port, token = session["port"], session["token"]
        check("the browser gets the private link", lines and lines[0] == f"http://127.0.0.1:{port}/?token={token}", lines)
        check("the session answers", ping(port, token) == 200)
        first_url = lines[0]

        os.write(fd, b":web second.txt\r")
        lines = wait_opened(2)
        check("a second :web reuses the session", len(lines) == 2 and lines[1] == first_url, lines)
        check("the session keeps its address", json.loads(state.read_text())["port"] == port)

        os.write(fd, b"iedit\x1b")
        drain(0.2)
        os.write(fd, b":web\r")
        check(":web says what stayed unsaved here", wait_for("1 unsaved here"), screen.display[-1])
        os.write(fd, b"u:q!\r")
        drain(0.5)
        os.waitpid(pid, 0)
        os.close(fd)
        time.sleep(0.5)
        check("the session outlives the terminal editor", ping(port, token) == 200)
    finally:
        # Stops the session through its own signal path.
        if state.exists():
            port = json.loads(state.read_text())["port"]
            for line in os.popen(f"ss -ltnpH 'sport = :{port}'"):
                found = re.search(r"pid=(\d+)", line)
                if found:
                    os.kill(int(found[1]), signal.SIGTERM)
        time.sleep(0.5)
    check("SIGTERM ends the detached session", not state.exists())

if failures:
    print(f"{len(failures)} failed:", *failures, sep="\n  - ")
    sys.exit(1)
print("web launch checks passed")
