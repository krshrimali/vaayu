#!/usr/bin/env python3
"""GitHub workspace: `,Gp` lists PRs (Enter opens an overview), `:ghthreads`
lists review threads that jump to the commented line, `:ghdiff` maps hunk
lines to local files, `:ghchecks` puts failures first and Enter opens the
job's failed-step log, `,Gi` lists issues (Enter shows one), `:ghcheckout`
checks a PR out and reloads buffers (refused with unsaved edits), and a
logged-out or missing `gh` degrades to one clear message -- all against a
fake `gh` on PATH (hermetic) through a real PTY at several sizes."""
import codecs, fcntl, os, pathlib, pty, select, shutil, signal, struct, subprocess, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())

FAKE_GH=r'''#!/usr/bin/env python3
import json, os, subprocess, sys
a = sys.argv[1:]
with open("gh.log", "a") as f:
    f.write(" ".join(a) + "\n")
if os.path.exists("noauth"):
    sys.stderr.write("To get started with GitHub CLI, please run:  gh auth login\n")
    sys.exit(4)
def out(v):
    print(json.dumps(v))
if a[:2] == ["pr", "list"]:
    out([{"number": 5, "title": "Fix parser", "author": {"login": "ann"},
          "headRefName": "pr-5", "isDraft": False, "state": "OPEN", "reviewDecision": "APPROVED"},
         {"number": 4, "title": "Docs", "author": {"login": "bob"},
          "headRefName": "docs", "isDraft": True, "state": "OPEN", "reviewDecision": ""}])
elif a[:2] == ["pr", "view"] and a[2] == "--json":
    out({"number": 5})
elif a[:2] == ["pr", "view"]:
    out({"number": int(a[2]), "title": "Fix parser", "state": "OPEN", "isDraft": False,
         "author": {"login": "ann"}, "headRefName": "pr-5", "baseRefName": "main",
         "url": "https://github.com/o/r/pull/5", "additions": 1, "deletions": 1,
         "changedFiles": 1, "reviewDecision": "APPROVED", "body": "Fixes the parser body text",
         "statusCheckRollup": [{"conclusion": "SUCCESS"}, {"conclusion": "FAILURE"}]})
elif a[:2] == ["pr", "diff"]:
    print("diff --git a/a.py b/a.py\n--- a/a.py\n+++ b/a.py\n@@ -1,3 +1,3 @@\n one\n-two\n+TWO\n three")
elif a[:1] == ["api"]:
    print(json.dumps([{"id": 1, "path": "a.py", "line": 3, "user": {"login": "ann"},
                       "body": "Rename this?"}]))
    print(json.dumps([{"id": 2, "in_reply_to_id": 1, "path": "a.py", "line": 3,
                       "user": {"login": "cy"}, "body": "Done"}]))
elif a[:2] == ["pr", "checks"]:
    out([{"name": "lint", "bucket": "pass", "workflow": "CI", "description": "",
          "link": "https://github.com/o/r/actions/runs/1/job/11"},
         {"name": "test", "bucket": "fail", "workflow": "CI", "description": "",
          "link": "https://github.com/o/r/actions/runs/1/job/22"}])
    sys.exit(1)  # gh exits non-zero when a check failed, JSON still printed
elif a[:2] == ["run", "view"]:
    print("test\tRun tests\t2026-01-02T03:04:05.0000000Z running 1 test")
    print("test\tRun tests\t2026-01-02T03:04:06.0000000Z error: boom in parser")
elif a[:2] == ["issue", "list"]:
    out([{"number": 3, "title": "Crash on start", "author": {"login": "dee"},
          "labels": [{"name": "bug"}]}])
elif a[:2] == ["issue", "view"]:
    print("title:\tCrash on start\n--\nissue body text here")
elif a[:2] == ["pr", "checkout"]:
    subprocess.run(["git", "checkout", "-q", "pr-5"], check=True)
else:
    sys.stderr.write("unknown command\n")
    sys.exit(1)
'''

def git(root,*args):
    subprocess.run(["git","-C",str(root),*args],check=True,capture_output=True)

for cols,rows in [(60,16),(100,24),(160,45)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-gh-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-gh-cfg-") as cfg, \
         tempfile.TemporaryDirectory(prefix="vaayu-gh-bin-") as fakebin:
        root=pathlib.Path(proj)
        cfg=pathlib.Path(cfg)
        (cfg/"vaayu").mkdir(parents=True)
        (cfg/"vaayu/config.toml").write_text('jk_escape=false\nclipboard_unnamedplus=false\nnumber=true\n')
        bindir=pathlib.Path(fakebin)
        # PATH is only this dir, so the real `gh` (if any) can't leak in and
        # removing the fake one simulates a machine without gh.
        for tool in ["git","rg","sh","python3"]:
            found=shutil.which(tool)
            if found:
                (bindir/tool).symlink_to(found)
        fake=bindir/"gh"
        fake.write_text(FAKE_GH)
        fake.chmod(0o755)
        git(root,"init","-q","-b","main")
        git(root,"config","user.email","t@example.com"); git(root,"config","user.name","t")
        (root/".gitignore").write_text("gh.log\nnoauth\n")
        (root/"a.py").write_text("one\ntwo\nthree\n")
        git(root,"add","."); git(root,"commit","-qm","init")
        git(root,"checkout","-qb","pr-5")
        (root/"a.py").write_text("one\nTWO\nchanged on pr branch\n")
        git(root,"commit","-qam","pr"); git(root,"checkout","-q","main")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(cfg),PATH=str(bindir))
            os.execv(binary,[binary,str(root/"a.py")])
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",rows,cols,0,0))
        screen=pyte.Screen(cols,rows);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder("utf-8")("replace")
        def drain(seconds=.2):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                ready,_,_=select.select([fd],[],[],min(.02,max(0,end-time.monotonic())))
                if ready:
                    try:data=os.read(fd,65536)
                    except OSError:break
                    if not data:break
                    stream.feed(decoder.decode(data))
        def key(s,seconds=.15):os.write(fd,s.encode());drain(seconds)
        def text():
            return "\n".join(screen.display)
        def wait_for(pred, timeout=4.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():
                    return True
                drain(.05)
            return False
        def row_with(s):
            return next((r for r in screen.display if s in r), None)
        def close():
            for _ in range(3):
                if "a.py" in screen.display[rows-2] and not row_with("GitHub") and not row_with("CI log"):
                    return
                key("\x1b",.2)
        def ex(cmd):
            close()
            key(":"+cmd+"\r",.2)
        try:
            drain(.4)
            # PR list → Enter → overview.
            key(",Gp",.1)
            assert wait_for(lambda: row_with("#5  Fix parser")), ("PR list should show #5\n"+text())
            assert row_with("#4  Docs"), ("and #4\n"+text())
            if cols>=100:
                assert "[approved]" in row_with("#5  Fix parser"), text()
                assert "[draft]" in row_with("#4  Docs"), text()
            key("\r",.1)
            assert wait_for(lambda: row_with("#5 Fix parser") and row_with("▸ Diff")), \
                ("Enter should open the PR overview\n"+text())
            assert row_with("checks: 1 passed, 1 failed, 0 pending"), ("rollup summary\n"+text())
            # An action row in the overview: "▸ Review threads".
            close()

            # Review threads jump to the commented line.
            ex("ghthreads 5")
            assert wait_for(lambda: row_with("ann: Rename this?")), ("threads list\n"+text())
            assert row_with("↳ cy: Done"), ("reply nested under its root\n"+text())
            key("\r",.3)
            assert wait_for(lambda: screen.display[rows-2].rstrip().endswith(" 3:1")), \
                ("Enter should jump to a.py line 3\n"+text())

            # Diff: hunk lines are locations in the local file.
            ex("ghdiff 5")
            assert wait_for(lambda: row_with("+TWO")), ("PR diff\n"+text())
            assert "a.py:2:1" in row_with("+TWO"), ("+TWO maps to a.py line 2\n"+text())

            # Checks: failure first; Enter opens its failed-step log.
            ex("ghchecks 5")
            assert wait_for(lambda: row_with("✗ CI / test")), ("checks list\n"+text())
            fail_i=next(i for i,r in enumerate(screen.display) if "✗ CI / test" in r)
            pass_i=next(i for i,r in enumerate(screen.display) if "✓ CI / lint" in r)
            assert fail_i<pass_i, ("failing checks are listed first\n"+text())
            key("\r",.1)
            assert wait_for(lambda: row_with("error: boom in parser")), ("job log\n"+text())
            assert row_with("── Run tests ──"), ("log grouped by step\n"+text())
            assert not row_with("2026-01-02T"), ("timestamps stripped\n"+text())
            log=(root/"gh.log").read_text().splitlines()
            assert "run view --job 22 --log-failed" in log, log

            # Issues.
            close()
            key(",Gi",.1)
            assert wait_for(lambda: row_with("#3  Crash on start")), ("issue list\n"+text())
            key("\r",.1)
            assert wait_for(lambda: row_with("issue body text here")), ("issue view\n"+text())

            # Checkout: refused while dirty, then checks out and reloads.
            close()
            key("ggix\x1b",.2)
            ex("ghcheckout 5")
            assert wait_for(lambda: "Save all buffers" in screen.display[rows-1]), \
                ("dirty buffers block a checkout\n"+text())
            key("u",.2)
            ex("ghcheckout 5")
            assert wait_for(lambda: row_with("changed on pr branch")), \
                ("checkout should reload the buffer\n"+text())
            assert "Checked out PR #5" in screen.display[rows-1], text()

            # Logged out.
            (root/"noauth").write_text("")
            ex("ghprs")
            assert wait_for(lambda: "isn't logged in" in screen.display[rows-1]), \
                ("auth failure should be one clear message\n"+text())
            (root/"noauth").unlink()

            # No gh at all.
            fake.unlink()
            key(",Gi",.1)
            assert wait_for(lambda: "isn't installed" in screen.display[rows-1]), \
                ("missing gh should be one clear message\n"+text())
            print(f"ok {cols}x{rows}")
        finally:
            try:os.kill(pid,signal.SIGKILL)
            except ProcessLookupError:pass
            os.waitpid(pid,0)
print("pty_github: all passed")
