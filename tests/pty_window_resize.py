#!/usr/bin/env python3
"""Ctrl-W >/< resize a vertical split (the │ separator moves); Ctrl-W = restores
an even split. Driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(80,14),(120,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-wresize-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        f=root/"f.txt"; f.write_text("left pane content\n")
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
        def sep_col():
            # column of the vertical separator on the top row
            row=screen.display[0]
            return row.find("│")
        try:
            drain(.4)
            key("\x17v",.4)          # Ctrl-W v: vertical split
            s0=sep_col()
            assert s0>0, ("expected a vertical separator\n"+"\n".join(screen.display))
            key("\x17>",.3); key("\x17>",.3); key("\x17>",.3)  # grow active pane
            s1=sep_col()
            assert s1!=s0, (f"Ctrl-W > should move the separator ({s0}->{s1})")
            key("\x17=",.3)          # equalize
            s2=sep_col()
            assert s2==s0, (f"Ctrl-W = should restore the even split ({s2} vs {s0})")
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
    print(f"window-resize PTY passed: {cols}x{rows}")
