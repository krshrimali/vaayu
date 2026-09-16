#!/usr/bin/env python3
""",gw: live grep for the word under the cursor (Normal) or the selected
text (Visual) -- driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-grepword-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        file=root/"f.txt"
        file.write_text("needle in a haystack\nanother needle here\nno match on this line\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(file)])
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",rows,cols,0,0))
        screen=pyte.Screen(cols,rows);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder("utf-8")("replace")
        def drain(seconds=.25):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                ready,_,_=select.select([fd],[],[],min(.02,max(0,end-time.monotonic())))
                if ready:
                    try:data=os.read(fd,65536)
                    except OSError:break
                    if not data:break
                    stream.feed(decoder.decode(data))
        def key(s,seconds=.2):os.write(fd,s.encode());drain(seconds)
        def text():
            return "\n".join(screen.display)
        try:
            drain(.3)
            # Cursor starts on "needle" (word 0); ,gw greps for it live.
            key(",gw",.5)
            assert "needle" in text() and "haystack" in text(), \
                ("grep-word did not find the match on the cursor's own line\n"+text())
            assert "another needle here" in text(), \
                ("grep-word missed the second match\n"+text())
            assert "no match on this line" not in text(), \
                ("grep-word matched a line that doesn't contain the word\n"+text())
            # Live grep opens straight into query-editing; the first Esc
            # only leaves that sub-mode (back to browsing results), a
            # second is needed to leave the results list entirely.
            key("\x1b",.2)
            key("\x1b",.2)
            # Visual selection: select "another" on line 2 and grep for it.
            key("j0",.2)
            key("vllllll",.2)  # select "another" (7 chars from col0)
            key(",gw",.5)
            assert "another needle here" in text(), \
                ("grep-selection did not find its own line\n"+text())
            assert "haystack" not in text(), \
                ("grep-selection matched a line without the selected text\n"+text())
            key("\x1b",.2)
            key("\x1b",.2)
            key(":qa\r")
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
    print(f"Grep word/selection PTY passed: {cols}x{rows}")
