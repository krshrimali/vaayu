#!/usr/bin/env python3
"""Argument text objects: `daa` on the first arg drops it and its comma;
`cia` changes just the argument under the cursor. Driven through a real PTY,
verified on disk."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(60,14),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-arg-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\nautopairs=false\n')
        f=root/"f.txt"; f.write_text("call(alpha, beta, gamma)\n")
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
            # Cursor on 'beta' (the middle argument): change it with cia.
            key("fb",.2)                # jump to first 'b' -> start of beta
            key("ciaNEW\x1b",.3)        # change inner argument
            key(":w\r",.3)
            assert f.read_text()=="call(alpha, NEW, gamma)\n", \
                ("cia should change just the argument, got %r"%f.read_text())
            # Now daa on the first argument removes it and its comma.
            key("0f(l",.2)              # to '(' then one right -> first char of alpha
            key("daa",.3)
            key(":w\r",.3)
            assert f.read_text()=="call(NEW, gamma)\n", \
                ("daa on first arg should drop it + comma, got %r"%f.read_text())
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
    print(f"argobject PTY passed: {cols}x{rows}")
