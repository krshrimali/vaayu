#!/usr/bin/env python3
"""`gqq` is dot-repeatable: reflow one line, move to another long line, `.`
reflows it too. Verified end-to-end via a PTY + on-disk contents."""
import fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(80,14),(120,24)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-gqd-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\ntextwidth=9\n')
        f=root/"a.txt"; f.write_text("aaaa bbbb cccc dddd eeee\nffff gggg hhhh iiii jjjj\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(f)])
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",rows,cols,0,0))
        def drain(seconds=.4):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                r,_,_=select.select([fd],[],[],.05)
                if r:
                    try:
                        if not os.read(fd,65536):break
                    except OSError:break
        try:
            drain(.5)
            os.write(fd,b"gqq"); drain(.4)   # reflow line 1
            os.write(fd,b"G"); drain(.3)      # jump to the still-long last line
            os.write(fd,b"."); drain(.4)      # repeat the reflow there
            os.write(fd,b":w\r"); drain(.4)
            os.write(fd,b":q!\r"); drain(.3)
            end=time.monotonic()+3
            while time.monotonic()<end:
                done,status=os.waitpid(pid,os.WNOHANG)
                if done:
                    assert os.waitstatus_to_exitcode(status)==0;pid=None;break
                time.sleep(.05)
            assert pid is None,"Editor failed to exit"
        finally:
            if pid:
                os.kill(pid,signal.SIGKILL);os.waitpid(pid,0)
            os.close(fd)
        out=f.read_text()
        lines=[l for l in out.splitlines() if l.strip()]
        assert all(len(l)<=11 for l in lines), f"both lines reflowed: {out!r}"
        # Both paragraphs' words survive.
        for w in ("aaaa","eeee","ffff","jjjj"):
            assert w in out, f"{w} preserved: {out!r}"
        assert len(lines)>=5, f"both long lines wrapped into several: {out!r}"
    print(f"gq dot-repeat PTY passed: {cols}x{rows}")
