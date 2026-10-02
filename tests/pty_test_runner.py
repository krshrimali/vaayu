#!/usr/bin/env python3
"""Test runner: `,Ts` runs the detected suite (pytest here, via a fake
`pytest` on PATH so the test is hermetic) streaming its output live into a
Results pane, then lists failures as a jumpable quickfix list and marks
test declarations ✓/✗ in the gutter; `,Tn` runs the test under the cursor,
`:testlast` repeats it, `,Tx` stops a run (killing the runner) and
`:testclear` drops the marks -- driven through a real PTY at several sizes."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())

FAKE_PYTEST=r'''#!/usr/bin/env python3
import os, sys, time
with open("argv.log", "a") as f:
    f.write(" ".join(sys.argv[1:]) + "\n")
with open("runner.pid", "w") as f:
    f.write(str(os.getpid()))
if os.path.exists("slow"):
    print("collecting slowly...", flush=True)
    time.sleep(30)
args = sys.argv[1:]
only = args[args.index("-k") + 1] if "-k" in args else None
if only in (None, "test_ok"):
    print("tests/test_m.py::test_ok PASSED                [ 50%]", flush=True)
if only is None:
    time.sleep(0.6)  # leave the pane visibly mid-run
if only in (None, "test_bad"):
    print("tests/test_m.py::test_bad FAILED               [100%]")
    print("=================== FAILURES ===================")
    print("___________________ test_bad ___________________")
    print("tests/test_m.py:6: AssertionError")
    print("=========== short test summary info ============")
    print("FAILED tests/test_m.py::test_bad - assert 1 == 2")
    sys.exit(1)
'''

SOURCE = "def test_ok():\n    assert 1 == 1\n\n\ndef test_bad():\n    assert 1 == 2\n"

for cols,rows in [(60,16),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-testrun-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-testrun-cfg-") as cfg, \
         tempfile.TemporaryDirectory(prefix="vaayu-testrun-bin-") as fakebin:
        root=pathlib.Path(proj)
        cfg=pathlib.Path(cfg)
        (cfg/"vaayu").mkdir(parents=True)
        (cfg/"vaayu/config.toml").write_text('jk_escape=false\nclipboard_unnamedplus=false\nnumber=true\n')
        fake=pathlib.Path(fakebin)/"pytest"
        fake.write_text(FAKE_PYTEST)
        fake.chmod(0o755)
        (root/"pyproject.toml").write_text("[project]\nname = 'demo'\n")
        (root/"tests").mkdir()
        test_file=root/"tests/test_m.py"
        test_file.write_text(SOURCE)
        argv_log=root/"argv.log"
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(cfg),
                              PATH=fakebin+os.pathsep+os.environ.get("PATH",""))
            os.execv(binary,[binary,str(test_file)])
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
        def argv_lines():
            return argv_log.read_text().splitlines() if argv_log.exists() else []
        try:
            drain(.4)
            # Suite run: the output pane opens at once and streams.
            key(",Ts",.1)
            assert wait_for(lambda: "running" in text() and "test_ok PASSED" in text()), \
                ("the live pane should show streamed output while running\n"+text())
            assert "test_bad FAILED" not in text(), ("output should arrive as it is printed\n"+text())
            assert wait_for(lambda: "1 failed, 1 passed" in text()), \
                ("a finished run with failures should list them\n"+text())
            assert argv_lines()==["-v"], argv_lines()
            failure=row_with("✗ tests/test_m.py::test_bad")
            assert failure and "tests/test_m.py:6" in failure, \
                ("the failure should be a location entry\n"+text())
            if cols>=100:
                assert "assert 1 == 2" in failure, ("with the summary message\n"+text())
            # Enter jumps to the failing line; the gutter marks both tests.
            key("\r",.3)
            assert wait_for(lambda: screen.display[rows-2].rstrip().endswith(" 6:1")), \
                ("Enter should jump to line 6\n"+text())
            ok_row=row_with("def test_ok")
            bad_row=row_with("def test_bad")
            assert ok_row and ok_row.lstrip().startswith("✓"), ("passing test marked ✓\n"+text())
            assert bad_row and bad_row.lstrip().startswith("✗"), ("failing test marked ✗\n"+text())
            assert not row_with("assert 1 == 1").lstrip().startswith(("✓","✗")), \
                ("only declaration lines get marks\n"+text())

            # Nearest: cursor inside test_ok's body.
            key("gg",.1); key("j",.1)
            key(",Tn",.1)
            assert wait_for(lambda: "1 passed" in text()), ("nearest run should pass\n"+text())
            assert argv_lines()[-1]=="-v tests/test_m.py -k test_ok", argv_lines()
            key("\x1b",.2)
            assert row_with("def test_bad").lstrip().startswith("✗"), \
                ("a nearest run keeps the other marks\n"+text())

            key(":testlast\r",.1)
            assert wait_for(lambda: len(argv_lines())==3), argv_lines()
            assert argv_lines()[-1]=="-v tests/test_m.py -k test_ok", argv_lines()
            drain(.5)
            key("\x1b",.2)

            # Stop a long run: the runner process is killed.
            (root/"slow").write_text("")
            key(",Ts",.1)
            assert wait_for(lambda: "collecting slowly" in text()), ("slow run should start\n"+text())
            runner_pid=int((root/"runner.pid").read_text())
            key("\x1b",.1)
            key(",Tx",.3)
            assert wait_for(lambda: "Test run stopped" in text()), ("stop should confirm\n"+text())
            def gone():
                try:
                    os.kill(runner_pid,0)
                except ProcessLookupError:
                    return True
                # A zombie is gone for our purposes.
                try:
                    with open(f"/proc/{runner_pid}/stat") as f:
                        return f.read().split()[2]=="Z"
                except OSError:
                    return True
            assert wait_for(gone,timeout=3), "the stopped runner should be killed"
            key(",Tx",.2)
            assert "No test run in progress" in text(), ("stopping when idle says so\n"+text())

            key(":testclear\r",.3)
            assert not row_with("def test_bad").lstrip().startswith("✗"), \
                (":testclear should drop the marks\n"+text())
            key(":qa!\r")
            end=time.monotonic()+3
            while time.monotonic()<end:
                done,status=os.waitpid(pid,os.WNOHANG)
                if done:
                    assert os.waitstatus_to_exitcode(status)==0;pid=None;break
                drain(.05)
            assert pid is None,"Editor failed to exit"
        finally:
            if pid:
                os.kill(pid,signal.SIGKILL);os.waitpid(pid,0)
            os.close(fd)
    print(f"Test runner PTY passed: {cols}x{rows}")
