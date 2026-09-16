#!/usr/bin/env python3
"""File tree: y/x/p copy, cut, and paste a node into the cursor's target
directory, refusing a name collision -- driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-filetree-cp-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-filetree-cp-cfg-") as cfg:
        root=pathlib.Path(proj)
        cfg=pathlib.Path(cfg)
        (cfg/"vaayu").mkdir(parents=True)
        (cfg/"vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        (root/"dest").mkdir()
        source=root/"source.txt"; source.write_text("payload\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(cfg))
            os.execv(binary,[binary,str(source)])
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
        def right_text():
            half = cols // 2
            return "\n".join(row[half:] for row in screen.display)
        try:
            drain(.3)
            key(",ft",.3)
            assert "dest" in right_text() and "source.txt" in right_text(), \
                ("tree missing dest/ or source.txt\n"+right_text())
            # nodes: dest (dir, sorts first), source.txt -- cursor starts on dest.
            key("j",.2)   # onto source.txt
            key("y",.2)   # copy it
            key("k",.2)   # back onto dest/
            key("p",.2)   # paste -- copies into dest/
            assert (root/"dest"/"source.txt").exists(), \
                ("y then p did not copy source.txt into dest/\n"+right_text())
            assert source.exists(), "copy must keep the original"
            assert (root/"dest"/"source.txt").read_text() == "payload\n"
            # Cut+paste actually moves it: cut the root-level source.txt,
            # paste into dest/ again -- but dest/source.txt already exists
            # from the copy above, so this proves the collision refusal.
            key("j",.2)   # onto source.txt again
            key("x",.2)   # cut it
            key("k",.2)   # onto dest/
            key("p",.2)   # should refuse: dest/source.txt already exists
            assert source.exists(), \
                ("cut+paste onto an existing name must refuse, not overwrite\n"+right_text())
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
    print(f"File tree copy/paste PTY passed: {cols}x{rows}")
