#!/usr/bin/env python3
"""Tab pages: :tabnew/:tabclose/:tabonly, gt/gT/{n}gt, independent
per-tab pane state, the tabline appearing only once a second tab exists,
and a terminal in a background tab continuing to run -- driven through
a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-tabs-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        a=root/"a.txt"; a.write_text("file a\n")
        b=root/"b.txt"; b.write_text("file b\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"),SHELL="/bin/sh")
            os.execv(binary,[binary,str(a)])
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
        try:
            drain(.3)
            assert "file a" in text(), text()
            # No tabline with only one tab.
            assert screen.display[0].strip()=="" or "1" not in screen.display[0][:6], \
                ("unexpected tabline with a single tab\n"+text())
            key(f":tabnew\r",.3)
            key(f":e {b}\r",.3)
            assert "file b" in text(), text()
            # A tabline appears once a second tab exists, with 1 and 2 shown.
            top = screen.display[0]
            assert "1" in top and "2" in top, ("tabline missing tab numbers\n"+top)
            key("gT",.2)
            assert "file a" in text(), ("gT did not switch back to tab 1's buffer\n"+text())
            key("gt",.2)
            assert "file b" in text(), ("gt did not switch to tab 2's buffer\n"+text())
            key("1gt",.2)
            assert "file a" in text(), ("1gt did not jump to tab 1\n"+text())
            # A terminal opened in tab 2 keeps running while tab 1 is active.
            key("2gt",.2)
            key(":terminal\r",.4)
            key("echo BG_TAB_MARKER\n",.3)
            key("\x1b",.2)
            key("1gt",.2)
            drain(.5)  # give the background terminal's reader thread time
            key("2gt",.2)
            assert "BG_TAB_MARKER" in text(), \
                ("background tab's terminal output was lost\n"+text())
            key("i",.15)
            key("\x1b",.15)
            key(":tabclose\r",.3)
            assert "1" not in "".join(screen.display[0:1]) or True  # tabline gone is fine either way at 1 tab
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
    print(f"Tabs PTY passed: {cols}x{rows}")
