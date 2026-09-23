#!/usr/bin/env python3
"""Line-ending (fileformat) handling: a CRLF file loads as [dos], edits keep
\\n-only internally, `:w` restores CRLF on disk, and `:set ff=unix` + `:w`
converts to LF. Driven through a real PTY; the parent reads the file bytes
back off disk to verify the exact endings."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(60,14),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-ff-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        f=root/"dos.txt"; f.write_bytes(b"one\r\ntwo\r\n")
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
        def text():return "\n".join(screen.display)
        try:
            drain(.4)
            assert "one" in text() and "two" in text(), ("initial content\n"+text())
            assert "[dos]" in text(), ("status bar should mark [dos]\n"+text())
            # No raw carriage returns should be visible in the buffer text.
            assert "^M" not in text(), ("CR must not leak into the buffer\n"+text())
            key("ggIX\x1b",.2)         # insert X at the very start
            key(":w\r",.3)             # write — CRLF must be preserved
            assert f.read_bytes()==b"Xone\r\ntwo\r\n", \
                (":w should preserve CRLF, got %r"%f.read_bytes())
            key(":set ff=unix\r",.3)   # convert to unix
            key(":w\r",.3)
            assert f.read_bytes()==b"Xone\ntwo\n", \
                (":set ff=unix + :w should convert to LF, got %r"%f.read_bytes())
            key(":q!\r")
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
    print(f"fileformat PTY passed: {cols}x{rows}")
