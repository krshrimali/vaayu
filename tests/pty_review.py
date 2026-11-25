#!/usr/bin/env python3
"""Review comments, end to end through a real PTY.

Covers: the composer split (`,rc`, `:w`, `:wq`, `:q`, `:q!`, empty drafts),
inline comment text and colored gutter marks, the tinted range while
writing, live anchors through unsaved edits and undo, "code changed" marks,
`]r`/`[r`, the comments list (filters, resolve, reply, confirmed delete,
move + `,rm`), several reviews per folder, two editors on one review merging
live, agent runs (replies, findings, stderr, finding → comment, timeout,
cancel, status line), crash recovery of an unsaved comment, the version-1
`comments.json` import, and a large review staying responsive.

Usage: python3 tests/pty_review.py target/release/vaayu
"""
import codecs, fcntl, json, os, pathlib, pty, random, select, signal, struct, sys, tempfile, termios, time
import pyte

BINARY = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "target/release/vaayu").resolve())
ESC = "\x1b"


class Session:
    def __init__(self, root, proj, args, cols, rows):
        self.root, self.proj, self.cols, self.rows = root, proj, cols, rows
        pid, fd = pty.fork()
        if pid == 0:
            os.chdir(proj)
            os.environ.update(TERM="xterm-256color", XDG_CONFIG_HOME=str(root / "config"),
                              HOME=str(root), XDG_DATA_HOME=str(root / "data"))
            os.environ.pop("NO_COLOR", None)
            os.execv(BINARY, [BINARY, *map(str, args)])
        self.pid, self.fd = pid, fd
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        self.screen = pyte.Screen(cols, rows)
        self.stream = pyte.Stream(self.screen)
        self.dec = codecs.getincrementaldecoder("utf-8")("replace")
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
                self.stream.feed(self.dec.decode(data))

    def key(self, s, delay=0.25):
        # Esc is sent on its own so it is never read as Alt+key.
        for i, part in enumerate(s.split(ESC)):
            if i:
                os.write(self.fd, ESC.encode())
                self.drain(0.1)
            if part:
                os.write(self.fd, part.encode())
        self.drain(delay)

    def text(self):
        return "\n".join(self.screen.display)

    def line(self, y):
        return self.screen.display[y]

    def message(self):
        return self.screen.display[-1].strip()

    def wait_for(self, pred, what, seconds=4):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            self.drain(0.05)
            if pred(self.text()):
                return
        raise AssertionError(f"{what}\n{self.text()}")

    def row_of(self, needle):
        for y, l in enumerate(self.screen.display):
            if needle in l:
                return y
        raise AssertionError(f"{needle!r} not on screen\n{self.text()}")

    def alive(self):
        try:
            return os.waitpid(self.pid, os.WNOHANG)[0] == 0
        except ChildProcessError:
            return False

    def close(self):
        try:
            os.kill(self.pid, signal.SIGKILL)
            os.waitpid(self.pid, 0)
        except (ProcessLookupError, ChildProcessError):
            pass


def project(files, config=""):
    root = pathlib.Path(tempfile.mkdtemp(prefix="vaayu-review-"))
    (root / "config/vaayu").mkdir(parents=True)
    (root / "config/vaayu/config.toml").write_text("winbar=false\n"+
        'jk_escape=false\nnumber=true\ncolorscheme="tokyonight"\nreview_author="Tester"\n' + config)
    proj = root / "proj"
    for name, body in files.items():
        p = proj / name
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(body)
    return root, proj


def stored(proj):
    files = sorted((proj / ".vaayu/reviews").glob("*.json"))
    return [json.loads(f.read_text()) for f in files]


def comment(s, keys_before, text):
    s.key(keys_before + ",rc", 0.35)
    s.key(text + ESC)
    s.key(":wq\r", 0.3)


SRC = "".join(f"fn f{i}() {{ let x = {i}; }}\n" for i in range(1, 25))


def composer_and_marks(cols, rows):
    root, proj = project({"src/lib.rs": SRC})
    s = Session(root, proj, ["src/lib.rs"], cols, rows)
    try:
        # A new comment opens ready to type in a split under the code, with
        # the commented lines tinted.
        s.key("3GVj,rc", 0.4)
        assert "INSERT [New comment · src/lib.rs:3–4]" in s.text(), s.text()
        assert "BUFFER src/lib.rs" in s.text(), "the code stays visible above"
        bg = lambda y: s.screen.buffer[y][8].bg
        assert bg(2) == bg(3) != bg(4), (bg(2), bg(3), bg(4))
        # Empty drafts are not saved; :q drops them and returns to the code.
        s.key(ESC + ":w\r")
        assert "Empty comment not saved" in s.message(), s.message()
        s.key(":q\r")
        assert s.alive() and "NORMAL src/lib.rs" in s.text(), s.text()
        assert not (proj / ".vaayu/reviews").exists() or all(not r["comments"] for r in stored(proj))
        # Typed text survives :q; :q! discards it.
        s.key("3G,rc", 0.35)
        s.key("draft" + ESC + ":q\r")
        assert "Unsaved comment" in s.message(), s.message()
        s.key(":q!\r")
        assert "NORMAL src/lib.rs" in s.text()
        # :wq saves and closes only the composer.
        s.key("3GVj,rc", 0.35)
        s.key("Needs a doc comment" + ESC + ":wq\r", 0.35)
        assert s.alive(), ":wq in a composer must not quit"
        y = s.row_of("fn f3()")
        assert s.line(y).startswith("●"), s.line(y)
        assert s.line(y + 1).startswith("│"), s.line(y + 1)
        assert "#1 Needs a doc comment" in s.line(y), s.line(y)
        glyph, number = s.screen.buffer[y][0], s.screen.buffer[y][4]
        assert glyph.fg != number.fg, "the comment mark has its own color"
        assert "● 1" in s.line(rows - 2), "status line counts open comments"
        data = stored(proj)[0]["comments"][0]
        assert (data["start"], data["end"], data["author"]) == (2, 3, "Tester"), data
        # Live anchors: an unsaved insert above moves the mark; undo restores.
        s.key("ggOnew line" + ESC)
        assert s.line(s.row_of("fn f3()")).startswith("●"), s.text()
        s.key("u")
        assert s.line(s.row_of("fn f3()")).startswith("●"), s.text()
        # Changing commented code flags it in the warning color.
        s.key("3Gccfn f3() { changed }" + ESC)
        y = s.row_of("changed }")
        assert s.line(y).startswith("◌"), s.line(y)
        assert s.screen.buffer[y][0].fg != glyph.fg
        # ,rm re-anchors it to the new code.
        s.key(",rm")
        assert "now points at line 3" in s.message(), s.message()
        assert s.line(s.row_of("changed }")).startswith("●")
        s.key(":w\r")
        # ,rv shows the thread; ,rr replies; ,rx resolves.
        s.key("3G,rv")
        assert "Comment #1" in s.text() and "Tester" in s.text(), s.text()
        s.key("q")
        s.key("3G,rr", 0.35)
        assert "Reply to #1" in s.text(), s.text()
        s.key("Done now" + ESC + ":w\r", 0.35)
        assert "Reply saved" in s.message(), s.message()
        assert "↩1" in s.line(s.row_of("fn f3()"))
        s.key("3G,rx")
        assert "Comment resolved" in s.message()
        assert s.line(s.row_of("fn f3()")).startswith("○"), s.line(s.row_of("fn f3()"))
        assert stored(proj)[0]["comments"][0]["status"] == "resolved"
    finally:
        s.close()


def list_and_navigation(cols, rows):
    root, proj = project({"src/a.rs": SRC, "src/b.rs": "one\ntwo\nthree\n"})
    s = Session(root, proj, ["src/a.rs"], cols, rows)
    try:
        comment(s, "5G", "five")
        comment(s, "10G", "ten")
        s.key(":e src/b.rs\r")
        comment(s, "2G", "bee")
        s.key("gg]r")
        assert s.line(rows - 1).startswith("#3"), s.message()
        s.key("]r")
        assert "wrapped" in s.message() and "src/a.rs" in s.text(), s.message()
        s.key("[r")
        assert "#3" in s.message(), s.message()
        s.key(":comments\r", 0.35)
        assert "Review:" in s.line(0) and "3 open" in s.line(0), s.line(0)
        order = [l for l in s.screen.display if "#" in l and "src/" in l]
        assert "src/a.rs:5" in order[0] and "src/b.rs:2" in order[2], order
        # Resolving saves at once and leaves the open view.
        s.key("R", 0.3)
        assert "2 open" in s.line(0) and "five" not in s.text(), s.text()
        assert [c["status"] for c in stored(proj)[0]["comments"]].count("resolved") == 1
        s.key("o")
        assert "needs attention" in s.line(0)
        s.key("o")
        assert "showing resolved" in s.line(0) and "five" in s.text()
        s.key("o")
        assert "showing all" in s.line(0)
        # Delete asks first.
        s.key("G" + "d")
        assert "y yes" in s.text(), s.text()
        s.key("n")
        assert len(stored(proj)[0]["comments"]) == 3
        s.key("d")
        s.key("y", 0.3)
        assert len(stored(proj)[0]["comments"]) == 2
        # m then ,rm moves a comment.
        s.key("gg")
        y = s.row_of("ten")
        s.key("j" * (y - 2) + "m", 0.35)
        assert "Moving #2" in s.message(), s.message()
        s.key("15G,rm")
        moved = [c for c in stored(proj)[0]["comments"] if c["seq"] == 2][0]
        assert moved["start"] == 14, moved
        # r replies from the list.
        s.key(":comments\r")
        s.key("r", 0.35)
        assert "Reply to" in s.text()
        s.key("from the list" + ESC + ":wq\r")
        assert any(c["replies"] for c in stored(proj)[0]["comments"])
    finally:
        s.close()


def several_reviews_and_two_editors(cols, rows):
    root, proj = project({"src/a.rs": SRC})
    a = Session(root, proj, ["src/a.rs"], cols, rows)
    b = Session(root, proj, ["src/a.rs"], cols, rows)
    try:
        comment(a, "4G", "from A")
        b.drain(1.2)
        b.key("j")
        assert "#1 from A" in b.text(), "B shows A's comment without any command\n" + b.text()
        comment(b, "6G", "from B")
        a.drain(1.2)
        a.key("j")
        assert "#2 from B" in a.text(), a.text()
        data = stored(proj)
        assert len(data) == 1, "both editors joined one review"
        assert sorted(c["seq"] for c in data[0]["comments"]) == [1, 2]
        # A second review in the same folder has its own comments.
        a.key(":reviewnew Second pass\r")
        assert 'Started review "Second pass"' in a.message(), a.message()
        assert "#1 from A" not in a.text()
        comment(a, "8G", "second review")
        a.key(":reviews\r", 0.35)
        assert "Second pass" in a.text() and "▶" in a.line(a.row_of("Second pass")), a.text()
        y = a.row_of("2 open")
        a.key("gg" + "j" * (y - 2) + "\r", 0.4)
        assert "2 open" in a.line(0) and "from B" in a.text(), a.text()
        a.key("q:reviewswitch second\r")
        assert 'Now in review "Second pass"' in a.message(), a.message()
        assert len(stored(proj)) == 2
    finally:
        a.close()
        b.close()


AGENT = """#!/usr/bin/env python3
import json, sys, time
p = json.load(sys.stdin)
if 'slow' in sys.argv: time.sleep(30)
for c in p["comments"]:
    print(json.dumps({"comment": c["number"], "reply": "Handled " + c["text"], "resolve": c["number"] == 1}))
print("src/a.rs:7: f7 shadows x")
print("plain output line")
print("agent warning", file=sys.stderr)
"""


def agent_runs(cols, rows):
    tmp = pathlib.Path(tempfile.mkdtemp(prefix="vaayu-agent-"))
    agent = tmp / "fakeagent"
    agent.write_text(AGENT)
    agent.chmod(0o755)
    root, proj = project({"src/a.rs": SRC}, f'review_command=["{agent}"]\nreview_timeout_secs=2\n')
    s = Session(root, proj, ["src/a.rs"], cols, rows)
    try:
        s.key(":reviewresults\r")
        assert "No agent review has run" in s.message(), s.message()
        comment(s, "2G", "two")
        comment(s, "4G", "four")
        s.key(",ra")
        s.wait_for(lambda t: "Agent review finished (exit 0)" in t, "run finishes")
        assert "2 replies added" in s.message() and "1 finding" in s.message(), s.message()
        y = s.row_of("fn f2()")
        assert "↩1" in s.line(y) and "✓?" in s.line(y), s.line(y)
        s.key(":reviewresults\r", 0.35)
        for needle in ["── Replies to comments", "[suggests resolving]", "── Findings",
                       "f7 shadows x", "plain output line", "── Errors (stderr)", "agent warning"]:
            assert needle in s.text(), needle + "\n" + s.text()
        y = s.row_of("f7 shadows x")
        s.key("gg" + "j" * (y - 2) + "c", 0.4)
        assert "[New comment · src/a.rs:7]" in s.text() and "f7 shadows x" in s.text(), s.text()
        s.key(":wq\r")
        assert len(stored(proj)[0]["comments"]) == 3
        assert all(not c["status"] == "resolved" for c in stored(proj)[0]["comments"]), "agents never resolve"
        runs = list((proj / ".vaayu/reviews/runs").glob("*/*"))
        assert len(runs) == 1 and (runs[0] / "stderr.txt").read_text().strip() == "agent warning"
        s.key(":reviewruns\r")
        assert "finished (exit 0)" in s.text(), s.text()
        s.key("q")
    finally:
        s.close()
    root, proj = project({"src/a.rs": SRC}, f'review_command=["{agent}", "slow"]\nreview_timeout_secs=2\n')
    s = Session(root, proj, ["src/a.rs"], cols, rows)
    try:
        comment(s, "2G", "two")
        s.key(",ra", 1.2)
        assert "review agent" in s.line(rows - 2), "status line shows the run\n" + s.text()
        s.key(",ra")
        assert "already running" in s.message(), s.message()
        s.wait_for(lambda t: "timed out after 2s" in t, "timeout", 5)
        s.key(",ra")
        s.key(":reviewcancel\r", 0.4)
        s.wait_for(lambda t: "Agent review cancelled" in t, "cancel")
    finally:
        s.close()


def recovery_and_import(cols, rows):
    root, proj = project({"src/a.rs": SRC})
    (proj / ".vaayu").mkdir()
    (proj / ".vaayu/comments.json").write_text(json.dumps({"version": 1, "notes": [
        {"id": 1, "file": "src/a.rs", "start": 1, "end": 1, "whole_file": False,
         "anchor": "fn f2() { let x = 2; }", "text": "old note", "stale": False, "resolved": False}]}))
    s = Session(root, proj, ["src/a.rs"], cols, rows)
    try:
        assert "#1 old note" in s.text(), s.text()
        s.key(":comments\r")
        assert "Imported comments" in s.line(0), s.line(0)
        assert (proj / ".vaayu/comments.json.v1-imported").exists()
        s.key("q")
        s.key("9G,rc", 0.35)
        s.key("never saved", 1.6)
    finally:
        s.close()
    s = Session(root, proj, ["src/a.rs"], cols, rows)
    try:
        s.key(":recover\r", 0.35)
        assert "Recover New comment · src/a.rs:9" in s.text(), s.text()
        s.key("\r", 0.4)
        assert "never saved" in s.text() and "[+]" in s.text(), s.text()
        s.key(":wq\r", 0.35)
        assert any(c["text"] == "never saved" for c in stored(proj)[0]["comments"])
    finally:
        s.close()


FAKE_GH = r"""#!/usr/bin/env python3
import json, os, sys
a = sys.argv[1:]
log = os.environ["FAKE_GH_LOG"]
if a[:2] == ["pr", "view"] and a[2:] == ["--json", "number"]:
    print(json.dumps({"number": 7}))
elif a[:2] == ["pr", "view"] and "title" in a[-1]:
    print(json.dumps({"number": 7, "title": "Speed up f", "headRefOid": os.environ["FAKE_HEAD"], "url": "https://example.test/pr/7"}))
elif a[:2] == ["pr", "diff"]:
    print("diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,5 +1,5 @@\n a\n-b\n+c\n d\n e\n f")
elif a[0] == "api" and "--input" in a:
    open(log, "w").write(json.dumps({"args": a, "body": json.load(sys.stdin)}))
    print(json.dumps({"html_url": "https://example.test/pr/7#review"}))
elif a[0] == "api" and "--paginate" in a:
    print(json.dumps([
        {"id": 1, "path": "src/a.rs", "line": 3, "user": {"login": "ann"}, "body": "Why this?", "created_at": "2026-11-01T00:00:00Z"},
        {"id": 2, "in_reply_to_id": 1, "path": "src/a.rs", "line": 3, "user": {"login": "bob"}, "body": "Speed", "created_at": "2026-11-01T01:00:00Z"},
        {"id": 3, "path": "src/a.rs", "line": None, "original_line": 9, "user": {"login": "ann"}, "body": "Outdated one", "created_at": "2026-11-02T00:00:00Z"}]))
else:
    sys.exit("unexpected gh call: " + " ".join(a))
"""


def github_publish_and_import(cols, rows):
    import subprocess
    root, proj = project({"src/a.rs": SRC})
    tools = root / "tools"
    tools.mkdir()
    (tools / "gh").write_text(FAKE_GH)
    (tools / "gh").chmod(0o755)
    git = lambda *a: subprocess.run(["git", "-c", "user.name=T", "-c", "user.email=t@t", *a], cwd=proj,
                                    capture_output=True, text=True).stdout.strip()
    git("init", "-q")
    git("add", ".")
    git("commit", "-qm", "base")
    os.environ.update(PATH=f"{tools}:{os.environ['PATH']}", FAKE_GH_LOG=str(root / "posted.json"),
                      FAKE_HEAD=git("rev-parse", "HEAD"))
    s = Session(root, proj, ["src/a.rs"], cols, rows)
    try:
        comment(s, "3G", "inline please")
        comment(s, "20G", "outside the diff")
        s.key(":reviewpublish\r", 0)
        s.wait_for(lambda t: "Publish" in t and "PR #7" in t, "publish plan")
        assert "1 inline, 1 in the summary" in s.text(), s.text()
        assert not (root / "posted.json").exists(), "nothing is posted before Enter"
        s.key("gg\r", 0)
        s.wait_for(lambda t: "Pending review created on PR #7" in t, "posted")
        posted = json.loads((root / "posted.json").read_text())
        assert "pulls/7/reviews" in " ".join(posted["args"])
        body = posted["body"]
        assert "event" not in body, "a pending review is left for the user to submit"
        assert body["comments"] == [{"path": "src/a.rs", "line": 3, "side": "RIGHT", "body": "inline please"}], body
        assert "src/a.rs:20" in body["body"] and "outside the diff" in body["body"], body
        s.key("q:reviewimport\r", 0)
        s.wait_for(lambda t: "Imported 2 thread(s) from PR #7" in t, "import")
        assert "PR #7: Speed up f" in s.line(0), s.line(0)
        assert "Why this?" in s.text() and "↩1" in s.text() and "Outdated one" in s.text(), s.text()
        imported = [r for r in stored(proj) if r["title"].startswith("PR #7")][0]
        authors = sorted(c["author"] for c in imported["comments"])
        assert authors == ["ann", "ann"] and imported["comments"][0]["replies"][0]["author"] == "bob"
    finally:
        s.close()


def large_review_stays_fast():
    random.seed(3)
    files = {f"src/m{f:02}.rs": "".join(f"let v{i} = {i};\n" for i in range(1500)) for f in range(40)}
    root, proj = project(files)
    comments = []
    for n in range(1, 2001):
        f, line = f"src/m{n % 40:02}.rs", random.randrange(1490)
        comments.append({"id": f"{random.getrandbits(62):016x}", "seq": n, "file": f, "start": line,
                         "end": line, "anchor": f"let v{line} = {line};", "text": f"note {n}"})
    (proj / ".vaayu/reviews").mkdir(parents=True)
    (proj / ".vaayu/reviews/rlarge.json").write_text(json.dumps(
        {"version": 2, "id": "rlarge", "title": "Large", "next_seq": 2001, "comments": comments}))
    s = Session(root, proj, ["src/m00.rs"], 120, 40)
    try:
        t = time.monotonic()
        s.key(":comments\r", 0)
        s.wait_for(lambda x: "Review: Large — 2000 open" in x, "large list", 3)
        listed = time.monotonic() - t
        assert listed < 1.5, f":comments took {listed:.2f}s"
        s.key("q")
        t = time.monotonic()
        comment(s, "100G", "one more")
        assert len(stored(proj)[0]["comments"]) == 2001
        assert time.monotonic() - t < 2.5
    finally:
        s.close()


for cols, rows in [(80, 24), (120, 36)]:
    composer_and_marks(cols, rows)
    list_and_navigation(cols, rows)
    several_reviews_and_two_editors(cols, rows)
    agent_runs(cols, rows)
    recovery_and_import(cols, rows)
    github_publish_and_import(cols, rows)
    print(f"review PTY passed: {cols}x{rows}")
large_review_stays_fast()
print("review PTY passed: large review")
