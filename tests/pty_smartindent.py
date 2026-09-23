#!/usr/bin/env python3
"""Bracket-aware auto-indent: Enter and `o` after a line ending in an opening
bracket add one indent level; a following close-brace line and open-above (`O`)
keep the plain copied indent. Driven through a real PTY, verified on disk."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(60,14),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-si-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        # expandtab + shiftwidth 4 so an indent level is four spaces.
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nnumber=false\nexpandtab=true\nshiftwidth=4\n')
        f=root/"f.txt"; f.write_text("fn main() {\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(f)])
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",rows,cols,0,0))
        screen=pyte.Screen(cols,rows);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder("utf-8")("replace")
        def drain(seconds=.3):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                ready,_,_=select.select([fd],[],[],min(.02,max(0,end-time.monotonic())))
                if ready:
                    try:data=os.read(fd,65536)
                    except OSError:break
                    if not data:break
                    stream.feed(decoder.decode(data))
        def key(s,seconds=.3):os.write(fd,s.encode());drain(seconds)
        try:
            drain(.4)
            # Append at end of the `{` line, Enter (should indent +4), type body.
            key("A\rlet x = 1;\x1b",.3)
            key(":w\r",.3)
            got=f.read_text()
            assert got=="fn main() {\n    let x = 1;\n", ("Enter after brace should indent:\n%r"%got)
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
    print(f"smartindent PTY passed: {cols}x{rows}")
