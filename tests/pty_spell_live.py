#!/usr/bin/env python3
"""Live spell underline + `]s` navigation. When the machine has a system word
list, `:set spell` underlines misspelled words; otherwise it degrades cleanly
(`]s` reports there's nothing to jump to). The underline/nav logic itself is
covered by src/regression.rs's unit test with Dictionary::for_test."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
DICT_PATHS=["/usr/share/dict/words","/usr/share/dict/american-english",
            "/usr/dict/words","/usr/share/dict/web2"]
has_dict=any(os.path.exists(p) for p in DICT_PATHS)
for cols,rows in [(60,14),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-spelllive-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        f=root/"f.txt"; f.write_text("zqxwv correctly spelled\n")
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
        def row0_underlined():
            return any(screen.buffer[0][x].underscore for x in range(cols))
        def wait_for(pred,timeout=3.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.05)
            return False
        try:
            drain(.4)
            assert not row0_underlined(), ("no underline before :set spell\n"+text())
            key(":set spell\r",.4)
            if has_dict:
                assert wait_for(row0_underlined), \
                    ("misspelled word should be underlined\n"+text())
            else:
                # Graceful degradation: no dictionary -> no underline, and `]s`
                # reports nothing to jump to (no crash).
                key("]s",.3)
                assert wait_for(lambda: "No misspellings" in text() or "spell is off" in text()), \
                    ("without a dictionary, ]s should report no misspellings\n"+text())
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
    print(f"spell-live PTY passed ({'dict' if has_dict else 'no-dict'}): {cols}x{rows}")
