#!/usr/bin/env python3
"""Real terminal regression: explicit dimensions, pyte screen assertions, isolated clipboard."""
import os, sys, pty, select, time, termios, fcntl, struct, tempfile, pathlib, json, signal
import pyte
binary = str(pathlib.Path(sys.argv[1] if len(sys.argv)>1 else "target/release/vaayu").resolve())
with tempfile.TemporaryDirectory(prefix="vaayu-pty-") as tmp:
    root=pathlib.Path(tmp)
    (root/"config/vaayu").mkdir(parents=True)
    (root/"config/vaayu/config.toml").write_text('jk_escape=false\nclipboard_unnamedplus=false\n')
    (root/"sample.md").write_text("# Review\n\nfirst source line\nsecond source line\n\n"+"long line with words "*15+"\n")
    (root/"tools").mkdir()
    copy=root/"tools/wl-copy";copy.write_text('#!/bin/sh\ncat > "$VAAYU_TEST_CLIPBOARD"\n');copy.chmod(0o755)
    pid,fd=pty.fork()
    if pid==0:
        os.chdir(root);os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"),WAYLAND_DISPLAY="test",VAAYU_TEST_CLIPBOARD=str(root/"clipboard"),PATH=str(root/"tools")+":"+os.environ["PATH"])
        os.environ.pop("SSH_CONNECTION",None);os.environ.pop("SSH_TTY",None)
        os.execv(binary,[binary,"sample.md"])
    fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",24,100,0,0))
    screen=pyte.Screen(100,24);stream=pyte.Stream(screen);capture=bytearray()
    def drain(seconds=.2):
        deadline=time.monotonic()+seconds
        while time.monotonic()<deadline:
            r,_,_=select.select([fd],[],[],min(.03,max(0,deadline-time.monotonic())))
            if r:
                try: data=os.read(fd,65536)
                except OSError: break
                if not data: break
                capture.extend(data);stream.feed(data.decode('utf-8','replace'))
    def key(s,delay=.15):os.write(fd,s.encode());drain(delay)
    def text():return '\n'.join(screen.display)
    try:
        drain(.5)
        assert "# Review" in text(),text()
        key("jjVj,rc")
        assert "Private comment" in text(),text()
        key("iReview the boundary case\x1b")
        key("\x13",.3)
        data=json.loads((root/".vaayu/comments.json").read_text())
        assert data["notes"][0]["text"]=="Review the boundary case",data
        assert data["notes"][0]["start"]==2 and data["notes"][0]["end"]==3,data
        key(":comments\r")
        assert "Private comments" in text() and "Review the boundary case" in text(),text()
        key("Y",.3)
        assert "Review the boundary case" in (root/"clipboard").read_text()
        key("\t\x11")
        assert "QUICKFIX / Private comments" in text(),text()
        key("/boundary\r")
        assert "Pattern not found" not in text(),text()
        # Capture a representative reviewed quickfix screen.
        pathlib.Path("/tmp/vaayu-quickfix-screen.txt").write_text(text())
        key("\r")
        key(":vpreview\r",.3)
        assert "PREVIEW" in text(),text()
        assert "sample.md" in text(),text()
        pathlib.Path("/tmp/vaayu-split-screen.txt").write_text(text())
        key("\x17w")
        key("jj")
        key("\x17w")
        key(":only\r")
        key(":set nowrap\r")
        key("G$")
        assert screen.cursor.x<100 and screen.cursor.y<24
        (root/"clipboard").unlink()
        key(":grep source\r",.5)
        key("\r")
        assert "2 results" in text(),text()
        key("\x11")
        assert "QUICKFIX / Live grep" in text(),text()
        key("q")
        key(":qa\r",.2)
        deadline=time.monotonic()+3
        while time.monotonic()<deadline:
            done,status=os.waitpid(pid,os.WNOHANG)
            if done: assert os.waitstatus_to_exitcode(status)==0;pid=None;break
            time.sleep(.02)
        assert pid is None,"Editor did not exit"
        assert (root/"sample.md").read_text().startswith("# Review\n\nfirst source line"),"source was changed"
        print("PTY review/quickfix/clipboard/split/wrap/grep regression passed")
    finally:
        if pid:
            os.kill(pid,signal.SIGKILL);os.waitpid(pid,0)
        os.close(fd)
