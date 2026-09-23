#!/usr/bin/env python3
"""`gq` reflow: `gq}` re-wraps a long paragraph to textwidth without touching
the paragraph after it. Driven through a real PTY, verified on disk."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(60,14),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-reflow-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nnumber=false\ntextwidth=20\n')
        f=root/"prose.txt"
        f.write_text("alpha beta gamma delta epsilon zeta eta theta iota kappa\n\nsecond paragraph unchanged\n")
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
            key("gg0",.2)          # top of the file, first column
            key("gq}",.3)          # reflow the first paragraph
            key(":w\r",.3)
            got=f.read_text()
            # The second paragraph must be intact.
            assert "second paragraph unchanged" in got, ("2nd paragraph changed:\n%r"%got)
            # The first paragraph is now several lines, each within textwidth.
            first=got.split("\n\n")[0]
            assert "\n" in first, ("first paragraph should have wrapped:\n%r"%got)
            for line in first.splitlines():
                assert len(line)<=20, ("line exceeds textwidth: %r"%line)
            # No words were lost.
            assert first.split()==["alpha","beta","gamma","delta","epsilon","zeta","eta","theta","iota","kappa"], \
                ("words changed:\n%r"%first)
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
    print(f"reflow PTY passed: {cols}x{rows}")
