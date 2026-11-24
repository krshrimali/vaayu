#!/usr/bin/env python3
"""The review screen for an agent's worktree, end to end through a real PTY.

Covers: `:worktrees` listing a worktree with its `code-reviews/<slug>.json`
record, Enter switching to it and opening `:reviewstart` against the
record's base, the table of contents (status, path, +/- counts), unchanged
lines drawn dim (SGR 2) and the line under the mouse pointer lit,
`:reviewtoc bottom` moving the contents, a click on a file row opening it,
and a comment copied by `:reviewcopy`.

Usage: python3 tests/pty_review_screen.py target/release/vaayu
"""
import codecs, fcntl, json, os, pathlib, pty, select, shutil, signal, struct, subprocess, sys, tempfile, termios, time
import pyte

BINARY = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "target/release/vaayu").resolve())
ESC = "\x1b"
COLS, ROWS = 120, 30
WORDS = ["one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten"]


class Session:
    def __init__(self, home, cwd):
        pid, fd = pty.fork()
        if pid == 0:
            os.chdir(cwd)
            for var in ("NO_COLOR", "DISPLAY", "WAYLAND_DISPLAY", "SSH_TTY", "SSH_CONNECTION", "SSH_CLIENT", "TMUX"):
                os.environ.pop(var, None)
            os.environ.update(TERM="xterm-256color", HOME=str(home), XDG_CONFIG_HOME=str(home / "config"),
                              XDG_DATA_HOME=str(home / "data"), XDG_STATE_HOME=str(home / "state"))
            os.execv(BINARY, [BINARY, "a.txt"])
        self.pid, self.fd = pid, fd
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
        self.screen = pyte.Screen(COLS, ROWS)
        self.stream = pyte.Stream(self.screen)
        self.dec = codecs.getincrementaldecoder("utf-8")("replace")
        # Raw output since the last `mark()`: pyte ignores SGR 2, so dimming
        # is checked on the bytes the editor wrote.
        self.raw = ""
        self.drain(0.8)

    def drain(self, seconds=0.25):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            r, _, _ = select.select([self.fd], [], [], min(0.02, max(0, end - time.monotonic())))
            if r:
                try:
                    data = os.read(self.fd, 65536)
                except OSError:
                    return
                if not data:
                    return
                text = self.dec.decode(data)
                self.raw += text
                self.stream.feed(text)

    def key(self, s, delay=0.3):
        # Esc is sent on its own so it is never read as Alt+key.
        for i, part in enumerate(s.split(ESC)):
            if i:
                os.write(self.fd, ESC.encode())
                self.drain(0.1)
            if part:
                os.write(self.fd, part.encode())
        self.drain(delay)

    def mouse(self, code, x, y, release=False):
        """Helper function to send one SGR (1006) mouse report at 0-based cell (x, y)."""
        os.write(self.fd, f"\x1b[<{code};{x + 1};{y + 1}{'m' if release else 'M'}".encode())
        self.drain(0.3)

    def mark(self):
        self.raw = ""

    def text(self):
        return "\n".join(self.screen.display)

    def row_of(self, needle):
        for y, line in enumerate(self.screen.display):
            if needle in line:
                return y
        raise AssertionError(f"{needle!r} not on screen\n{self.text()}")

    def wait_for(self, needle, seconds=4):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            self.drain(0.05)
            if needle in self.text():
                return
        raise AssertionError(f"{needle!r} never appeared\n{self.text()}")

    def close(self):
        try:
            os.kill(self.pid, signal.SIGKILL)
            os.waitpid(self.pid, 0)
        except (ProcessLookupError, ChildProcessError):
            pass


def drawn_dim(raw, word):
    """Whether `word`'s glyph run was last drawn dim: SGR 2 since the last SGR 0 / 22 before it."""
    at = raw.rfind(word)
    assert at >= 0, f"{word!r} was not redrawn"
    before = raw[:at]
    start = max(before.rfind("\x1b[0m"), before.rfind("\x1b[22m"), 0)
    return "\x1b[2m" in before[start:]


def git(cwd, *args):
    subprocess.run(["git", "-c", "commit.gpgsign=false", *args], cwd=cwd, check=True, capture_output=True)


def fixture():
    home = pathlib.Path(tempfile.mkdtemp(prefix="vaayu-review-screen-"))
    (home / "config/vaayu").mkdir(parents=True)
    (home / "config/vaayu/config.toml").write_text(
        'jk_escape=false\nnumber=true\ncolorscheme="tokyonight"\nreview_author="Tester"\n')
    main = home / "repo"
    main.mkdir()
    git(main, "init", "-q")
    git(main, "config", "user.name", "Vaayu test")
    git(main, "config", "user.email", "vaayu-test@example.invalid")
    (main / "a.txt").write_text("\n".join(WORDS) + "\n")
    git(main, "add", ".")
    git(main, "commit", "-qm", "base")
    base = subprocess.run(["git", "rev-parse", "HEAD"], cwd=main, check=True, capture_output=True,
                          text=True).stdout.strip()
    with open(main / ".git/info/exclude", "a") as f:
        f.write("/code-reviews/\n")
    git(main, "worktree", "add", "-q", "-b", "fix", "code-reviews/fix", base)
    wt = main / "code-reviews/fix"
    (wt / "a.txt").write_text("\n".join("FIVE!" if w == "five" else w for w in WORDS) + "\n")
    (main / "code-reviews/fix.json").write_text(json.dumps({
        "version": 1, "slug": "fix", "title": "Make five loud", "worktree": "code-reviews/fix",
        "description": "The user wanted five to stand out.", "branch": "fix", "base": base,
        "status": "ready-for-review", "updated_at": "2026-11-24T12:00:00Z",
    }))
    return home, main, wt


def main():
    home, repo, wt = fixture()
    s = Session(home, repo)
    try:
        s.key(":worktrees\r", 0.6)
        s.wait_for("Make five loud")
        assert "+1 −1 · 1 file" in s.text(), s.text()
        assert "The user wanted five to stand out." in s.text(), "the detail pane shows the description"

        # Enter switches to the worktree and opens its review screen.
        s.mark()
        s.key("\r", 0.8)
        s.wait_for("REVIEW")
        assert "Make five loud" in s.text(), s.text()
        toc_row = s.row_of("a.txt  ")
        assert "+1 −1" in s.screen.display[toc_row], s.screen.display[toc_row]
        assert "FIVE!" in s.text(), "the changed file is open"
        assert drawn_dim(s.raw, "three"), "an unchanged line is dimmed"
        assert not drawn_dim(s.raw, "FIVE!"), "the changed line is not"
        assert (wt / ".vaayu/reviews").is_dir(), "the review lives in the worktree"

        # Pointer over `three`: that line is redrawn lit.
        y = s.row_of("three")
        x = s.screen.display[y].index("three")
        s.mark()
        s.mouse(35, x, y)
        assert not drawn_dim(s.raw, "three"), repr(s.raw[-200:])

        # The contents move to the bottom edge.
        s.key(":reviewtoc bottom\r", 0.5)
        assert s.row_of("REVIEW") > ROWS // 2, s.text()
        assert "Review contents on the bottom" in s.text()

        # A click on the file row opens it; then comment and copy.
        y = s.row_of("a.txt  ")
        s.mouse(0, 4, y)
        s.mouse(0, 4, y, release=True)
        s.key(ESC)
        s.key("5G,rc", 0.4)
        s.key("Why so loud?" + ESC)
        s.key(":wq\r", 0.4)
        s.key(":reviewcopy\r", 0.4)
        s.wait_for("Copied 1 comment")
        print("pty_review_screen: ok")
    finally:
        s.close()
        subprocess.run(["git", "worktree", "remove", "--force", "code-reviews/fix"], cwd=repo, capture_output=True)
        shutil.rmtree(home, ignore_errors=True)


if __name__ == "__main__":
    main()
