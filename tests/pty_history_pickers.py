#!/usr/bin/env python3
""":jumps, :chistory/:history and :shistory expose the jumplist, command
history and search history as navigable Results lists -- selecting an
entry jumps to the location or reruns the command/search -- driven
through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-histpick-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        file=root/"f.txt"
        file.write_text("alpha\nbravo\ncharlie\ndelta\necho\n")
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
            drain(.4)
            # --- :jumps -------------------------------------------------
            # Set a mark at line 1, move to line 5, then jump back to the
            # mark: `a pushes a jump recording line 5 (echo) as the origin
            # before landing on line 1. Absolute tmp paths can get clipped
            # in the 40-col list, so check the result count, not the path.
            key("ma",.2)
            key("G",.2)
            key("`a",.2)
            key(":jumps\r",.3)
            assert "· 1 results ·" in text(), ("jumps list should have one entry\n"+text())
            key("\r",.2)  # open the (only) entry -> back to line 5 (echo)
            key("x",.2)   # delete the char under cursor to prove we landed there
            assert "cho" in text() and "echo" not in text(), \
                (":jumps entry did not navigate back to line 5\n"+text())
            key("u",.2)  # undo the deletion
            assert "echo" in text(), ("undo after jumps-nav check failed\n"+text())
            # --- :chistory / :history ------------------------------------
            key(":set number\r",.2)
            key(":set nonumber\r",.2)
            assert "1 alpha" not in text(), ("gutter should be hidden\n"+text())
            key(":chistory\r",.3)
            assert ":chistory" in text() and ":set nonumber" in text() and ":set number" in text(), \
                ("chistory list missing expected entries\n"+text())
            key("jj",.2)  # cursor: 0=:chistory 1=:set nonumber 2=:set number
            key("\r",.3)  # rerun ":set number"
            assert "1 alpha" in text(), (":chistory entry did not rerun :set number\n"+text())
            key(":set nonumber\r",.2)
            # --- :shistory -------------------------------------------------
            key("/alpha\r",.2)
            key("/bravo\r",.2)
            key(":shistory\r",.3)
            assert "/bravo" in text() and "/alpha" in text(), \
                ("shistory list missing expected entries\n"+text())
            key("j",.2)  # cursor: 0=/bravo 1=/alpha
            key("\r",.3)  # rerun "/alpha" search
            key("x",.2)
            assert "lpha" in text() and "alpha" not in text(), \
                (":shistory entry did not rerun the search and land on the match\n"+text())
            key("u",.2)
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
    print(f"History-pickers PTY passed: {cols}x{rows}")
