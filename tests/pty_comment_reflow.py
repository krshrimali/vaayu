#!/usr/bin/env python3
"""Comment-aware `gq`: reflowing a long `//` comment keeps `//` on each wrapped
line (the leader isn't turned into a word). End-to-end via a PTY + on-disk."""
import fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(80,14),(120,24)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-cr-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\ntextwidth=20\n')
        f=root/"a.txt"; f.write_text("// alpha beta gamma delta epsilon zeta eta theta iota\n")
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
            os.write(fd,b"gqq"); drain(.5)   # reflow the comment line
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
        assert len(lines)>1, f"comment should wrap: {out!r}"
        assert all(l.startswith("// ") for l in lines), f"every line keeps //: {out!r}"
        assert all(len(l)<=22 for l in lines), f"within textwidth: {out!r}"
        # No line has a doubled leader (leader consumed as a word would show //).
        assert all(l.count("//")==1 for l in lines), f"single leader per line: {out!r}"
    print(f"comment reflow PTY passed: {cols}x{rows}")
