#!/usr/bin/env python3
"""Persistent undo across a real process restart: edit, save, quit; a
fresh process on the same file can still undo past the save -- driven
through two separate real PTYs."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())

def run(binary, root, file, cols, rows, keys_seq):
    (root/"config/vaayu").mkdir(parents=True, exist_ok=True)
    (root/"config/vaayu/config.toml").write_text(
        'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
    )
    pid,fd=pty.fork()
    if pid==0:
        os.chdir(root)
        os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
        os.execv(binary,[binary,str(file)])
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
    try:
        drain(.3)
        for s, secs in keys_seq:
            key(s, secs)
        end=time.monotonic()+3
        while time.monotonic()<end:
            done,status=os.waitpid(pid,os.WNOHANG)
            if done:
                assert os.waitstatus_to_exitcode(status)==0,"editor exited non-zero"
                pid=None;break
            drain(.05)
        assert pid is None,"Editor failed to exit"
    finally:
        if pid:
            os.kill(pid,signal.SIGKILL);os.waitpid(pid,0)
        os.close(fd)

for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-persist-undo-") as tmp:
        root=pathlib.Path(tmp)
        file=root/"f.txt"
        file.write_text("original\n")
        # Process 1: append text and save, leaving undo history persisted.
        run(binary, root, file, cols, rows, [
            ("A", .1), (" appended", .1), ("\x1b", .2), (":w\r", .3), (":qa\r", .3),
        ])
        assert file.read_text()=="original appended\n", file.read_text()

        # Process 2 (a fresh process, simulating a restart): the very
        # first undo must restore pre-save history, not report "nothing
        # to undo" -- proving persistence survived the restart, not just
        # in-memory undo within one process.
        run(binary, root, file, cols, rows, [
            ("u", .2), (":w\r", .3), (":qa\r", .3),
        ])
        assert file.read_text()=="original\n", file.read_text()

        # A file that changed on disk after the save (by another tool,
        # here just rewritten directly) must not have stale undo history
        # replayed against it: undo must be a no-op, not corrupt the file.
        file.write_text("original\n")
        run(binary, root, file, cols, rows, [
            ("A", .1), (" x", .1), ("\x1b", .2), (":w\r", .3), (":qa\r", .3),
        ])
        file.write_text("changed externally\n")
        run(binary, root, file, cols, rows, [
            ("u", .2), (":w\r", .3), (":qa\r", .3),
        ])
        assert file.read_text()=="changed externally\n", file.read_text()
    print(f"Persistent undo PTY passed: {cols}x{rows}")
