#!/usr/bin/env python3
"""Minimap: `:set minimap` reserves a strip on the right of the pane with a
git lane, a Braille picture of the file, a tinted viewport slider and an
overview lane for search hits. `MARK:` comments become labels. Clicking the
strip jumps there, dragging the slider scrolls, and `:set nominimap` gives
the width back."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, subprocess, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
MAP_W=14
lines=[]
for i in range(400):
    if i==200:   lines.append("// MARK: Halfway")
    elif i%4==0: lines.append(f"fn item_{i}() {{")
    elif i%4==1: lines.append("    let value = compute(i);")
    elif i%4==2: lines.append("        return value + 1;")
    else:        lines.append("}")
CONTENT="\n".join(lines)+"\n"
def git(root,*a):
    subprocess.run(["git",*a],cwd=root,check=True,capture_output=True,
                   env={**os.environ,"GIT_AUTHOR_NAME":"t","GIT_AUTHOR_EMAIL":"t@t",
                        "GIT_COMMITTER_NAME":"t","GIT_COMMITTER_EMAIL":"t@t"})
BRAILLE=set(chr(c) for c in range(0x2801,0x2900))
for cols,rows in [(90,24),(120,30),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-mmp-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text("winbar=false\n"+'jk_escape=false\nnumber=false\n')
        f=root/"code.txt"  # .txt: no LSP, keeps the test focused
        f.write_text(CONTENT)
        git(root,"init","-q"); git(root,"add","code.txt"); git(root,"commit","-qm","init")
        # Uncommitted change on line 3 (0-based 2): the git lane marks row 0.
        changed=lines[:]; changed[2]="        return value + 2; // NEEDLE"
        f.write_text("\n".join(changed)+"\n")
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
        def key(s,seconds=.4):os.write(fd,s.encode());drain(seconds)
        def text():return "\n".join(screen.display)
        x0=cols-MAP_W
        content_rows=range(0,rows-2)
        def cell(x,y):return screen.buffer[y][x]
        def sep_count():
            return sum(1 for y in content_rows if cell(x0,y).data in "│▐▁")
        def dots():
            return sum(1 for y in content_rows for x in range(x0+1,cols-1)
                       if cell(x,y).data in BRAILLE)
        def tinted_rows():
            return [y for y in content_rows
                    if any(cell(x,y).bg!="default" for x in range(x0+1,cols-1))]
        def wait_for(pred,timeout=3.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.05)
            return False
        def mouse(btn,x,y,release=False):
            # SGR mouse report, 1-based coordinates.
            key(f"\x1b[<{btn};{x+1};{y+1}{'m' if release else 'M'}",.3)
        def first_line_number():
            # Line 0 shows "fn item_0", so read which item tops the pane.
            import re
            for y in content_rows:
                m=re.search(r"item_(\d+)",screen.display[y][:x0])
                if m:return int(m.group(1))-y
            return None
        try:
            drain(.6)
            key(":set minimap\r",.5)
            assert wait_for(lambda: sep_count()>=3), \
                ("minimap git lane / separator should appear on the right\n"+text())
            assert dots()>0, ("minimap should draw Braille dots\n"+text())
            tinted=tinted_rows()
            assert len(tinted)>0, ("viewport slider rows should be tinted\n"+text())
            assert len(tinted)<len(list(content_rows)), \
                ("out-of-view minimap rows should stay untinted\n"+text())
            # Git: line 2 was modified, so row 0's git lane is a change bar.
            assert wait_for(lambda: cell(x0,0).data=="▐"), \
                ("git lane should mark the modified line\n"+text())
            # Search: the overview lane marks the matching row.
            key("/NEEDLE\r",.5)
            assert wait_for(lambda: cell(cols-1,0).data=="■"), \
                ("overview lane should mark the search hit\n"+text())
            key(":noh\r",.3)
            # Section header: line 200 is `// MARK: Halfway`. 400 lines at 4
            # per row overflow the strip, so it shows once scrolled there.
            key("200G",.5)
            assert wait_for(lambda: any("Halfway" in screen.display[y][x0:] for y in content_rows)), \
                ("MARK: comment should show as a minimap label\n"+text())
            key("gg",.5)
            # Mouse: clicking low in the strip scrolls the editor down,
            # dragging to the top scrolls back, and nothing gets selected.
            before=first_line_number()
            mouse(0,x0+4,rows-4); mouse(0,x0+4,rows-4,release=True)
            assert wait_for(lambda: (first_line_number() or 0)>(before or 0)+5), \
                ("clicking the minimap should scroll the editor\n"+text())
            assert "VISUAL" not in text(), "a minimap click must not start a selection"
            top=first_line_number()
            mouse(0,x0+4,rows//2)                     # grab
            mouse(32,x0+4,0); mouse(0,x0+4,0,release=True)  # drag up, release
            assert wait_for(lambda: first_line_number() is not None and first_line_number()<top), \
                ("dragging the slider up should scroll the editor up\n"+text())
            assert "VISUAL" not in text(), "a minimap drag must not start a selection"
            key(":set nominimap\r",.4)
            assert wait_for(lambda: sep_count()==0 and len(tinted_rows())==0), \
                ("nominimap should reclaim the strip\n"+text())
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
    print(f"minimap PTY passed: {cols}x{rows}")
