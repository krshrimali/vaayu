#!/usr/bin/env python3
"""Filesystem watcher: with no keypress and no focus event, an external write
reloads an unmodified buffer; a modified buffer is kept and flagged
"[changed on disk]" until :e!; a file created outside the editor appears in
the open file tree and in the file picker (whose index used to be scanned
only once); :set nowatch stops the live reloads; :checkhealth reports the
watcher. Driven through a real PTY at several sizes."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(70,16),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-watch-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\ntree_icons=false\n')
        proj=root/"proj"; proj.mkdir()
        f=proj/"f.txt"; f.write_text("ORIGINALLINE\n")
        g=proj/"g.txt"; g.write_text("GFILELINE\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(proj)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"),HOME=str(root))
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
        def wait_for(pred,what,seconds=3):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                drain(.05)
                if pred(text()):return
            raise AssertionError(what+"\n"+text())
        try:
            drain(.6)
            assert "ORIGINALLINE" in text(), ("initial content\n"+text())
            # 1. Clean buffer: an external write reloads it, no key, no focus.
            f.write_text("CHANGEDEXTERNALLY\n")
            wait_for(lambda t:"CHANGEDEXTERNALLY" in t and "ORIGINALLINE" not in t,"clean buffer should autoread")
            wait_for(lambda t:"reloaded (changed on disk)" in t,"reload should be announced")
            # 2. Modified buffer: kept, flagged, warned; :e! reloads and clears.
            key("ODIRTYEDIT\x1b",.3)
            f.write_text("SECONDWRITE\n")
            wait_for(lambda t:"[changed on disk]" in t,"dirty buffer should be flagged in the status line")
            t=text()
            assert "DIRTYEDIT" in t and "CHANGEDEXTERNALLY" in t and "SECONDWRITE" not in t, ("dirty buffer must not be clobbered\n"+t)
            assert "unsaved changes" in t, ("conflict warning expected\n"+t)
            key(":e!\r",.4)
            t=text()
            assert "SECONDWRITE" in t and "DIRTYEDIT" not in t, (":e! should reload\n"+t)
            assert "[changed on disk]" not in t, (":e! should clear the flag\n"+t)
            # 3. File tree: an externally created file appears on its own.
            key(",e",.6)
            assert "g.txt" in text(), ("tree should list g.txt\n"+text())
            (proj/"watchednew.txt").write_text("x\n")
            wait_for(lambda t:"watchednew.txt" in t,"tree should show the new file",2)
            (proj/"watchednew.txt").unlink()
            wait_for(lambda t:"watchednew.txt" not in t,"tree should drop the removed file",2)
            key(",e",.3)  # close the focused tree
            # 4. File picker: the index re-scans after files appear.
            (proj/"pickerfreshfile.txt").write_text("y\n")
            drain(2.6)  # past the watcher's index-rescan rate limit
            key("\x10",.4); key("pickerfresh",.6)
            wait_for(lambda t:"pickerfreshfile.txt" in t,"picker should list the new file",2)
            key("\x1b",.3); key("\x1b",.3)  # leave the query, then close
            # 5. :checkhealth reports the watcher.
            key(":checkhealth\r",.6)
            assert "file watcher on" in text(), ("checkhealth should report the watcher\n"+text())
            key("q",.3)
            # 6. :set nowatch stops live reloads.
            key(":e! "+str(f)+"\r",.3)
            key(":set nowatch\r",.3)
            f.write_text("UNWATCHEDWRITE\n")
            drain(1.2)
            assert "UNWATCHEDWRITE" not in text(), ("nowatch must not autoread\n"+text())
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
    print(f"watcher PTY passed: {cols}x{rows}")
