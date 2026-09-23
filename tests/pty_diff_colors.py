#!/usr/bin/env python3
"""Diff mode colors each side distinctly: the first `:diffthis` buffer (old
side) tints its differing lines one color and the second (new side) another,
instead of both sharing one highlight. PTY: assert the two panes' differing
lines have different, non-default backgrounds."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(120,24),(160,40)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-dc-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        la=[f"same {i}" for i in range(12)]; la[5]="OLDLINE"
        lb=[f"same {i}" for i in range(12)]; lb[5]="NEWLINE"
        a=root/"a.txt"; a.write_text("\n".join(la)+"\n")
        b=root/"b.txt"; b.write_text("\n".join(lb)+"\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(a)])
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",rows,cols,0,0))
        screen=pyte.Screen(cols,rows);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder("utf-8")("replace")
        def drain(seconds=.35):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                r,_,_=select.select([fd],[],[],.05)
                if r:
                    try:data=os.read(fd,65536)
                    except OSError:break
                    if not data:break
                    stream.feed(decoder.decode(data))
        def key(s,seconds=.35):os.write(fd,s.encode());drain(seconds)
        def text():return "\n".join(screen.display)
        def cell_bg_of(word):
            # Background of the first char of the row that contains `word`.
            for y in range(rows):
                row="".join(screen.buffer[y][x].data for x in range(cols))
                idx=row.find(word)
                if idx!=-1:
                    return screen.buffer[y][idx].bg
            return None
        def wait_for(pred,timeout=5.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.15)
            return False
        try:
            drain(.5)
            key(":diffthis\r",.4)
            key(":vsplit b.txt\r",.6)
            key(":diffthis\r",.5)
            assert wait_for(lambda: "OLDLINE" in text() and "NEWLINE" in text()), \
                ("both diffed panes visible\n"+text())
            old_bg=cell_bg_of("OLDLINE")
            new_bg=cell_bg_of("NEWLINE")
            assert old_bg not in (None,"default"), (f"old side should be tinted, got {old_bg}\n"+text())
            assert new_bg not in (None,"default"), (f"new side should be tinted, got {new_bg}\n"+text())
            assert old_bg!=new_bg, (f"each side should get its own color (old={old_bg} new={new_bg})\n"+text())
            key(":diffoff\r",.3)
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
    print(f"diff colors PTY passed: {cols}x{rows}")
