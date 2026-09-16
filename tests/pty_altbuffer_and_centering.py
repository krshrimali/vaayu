#!/usr/bin/env python3
"""Ctrl-6/:b# toggle to the alternate (previously edited) buffer, and a
search jump (/, ?, n, N) recenters the match in the viewport instead of
doing a minimal scroll -- driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-altbuf-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        a=root/"a.txt"; a.write_text("file_a_unique_content\n")
        b=root/"b.txt"; b.write_text("file_b_unique_content\n")
        big=root/"big.txt"
        lines=["line" for _ in range(60)]
        lines[29]="needle_marker"
        big.write_text("\n".join(lines)+"\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(a)])
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
        def ctrl6(seconds=.2):os.write(fd,b"\x1e");drain(seconds)
        def text():
            return "\n".join(screen.display)
        try:
            drain(.4)
            # --- Ctrl-6 / :b# alternate buffer ---------------------------
            key(f":e {b}\r",.3)
            assert "file_b_unique_content" in text() and "file_a_unique_content" not in text(), \
                (":e did not switch to b.txt\n"+text())
            ctrl6(.3)
            assert "file_a_unique_content" in text() and "file_b_unique_content" not in text(), \
                ("Ctrl-6 did not switch to the alternate buffer (a.txt)\n"+text())
            ctrl6(.3)
            assert "file_b_unique_content" in text() and "file_a_unique_content" not in text(), \
                ("Ctrl-6 again did not toggle back to b.txt\n"+text())
            key(":b#\r",.3)
            assert "file_a_unique_content" in text() and "file_b_unique_content" not in text(), \
                (":b# did not toggle to the alternate buffer\n"+text())
            # --- Search-jump recentering ---------------------------------
            key(f":e {big}\r",.3)
            key("/needle_marker\r",.3)
            t=text()
            assert "needle_marker" in t, ("search did not land on the marker\n"+t)
            display_lines=screen.display
            marker_row=next(i for i,l in enumerate(display_lines) if "needle_marker" in l)
            # Centered: some content ("line") should be visible both above
            # and below the marker's row, not just below it (a minimal
            # scroll that merely brought the match onto the bottom edge
            # would show nothing but blank/status rows above it instead).
            above=[l for l in display_lines[:marker_row] if l.strip()=="line"]
            below=[l for l in display_lines[marker_row+1:] if l.strip()=="line"]
            assert above and below, \
                (f"search jump should center the match, not edge-scroll to it (marker at row {marker_row})\n"+t)
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
    print(f"Alt-buffer/centering PTY passed: {cols}x{rows}")
