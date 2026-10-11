#!/usr/bin/env python3
"""Which-key behavior, popup contrast across themes, and :keymaps execution."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
themes=("default", "mono", "warm", "cool", "gruvbox", "gruvbox-light",
        "flexoki", "flexoki-light", "tokyonight", "tokyonight-day")
def contrast(fg,bg):
    def luminance(color):
        assert len(color)==6, ("popup must set explicit colors",color)
        values=[int(color[i:i+2],16)/255 for i in (0,2,4)]
        linear=[v/12.92 if v<=.04045 else ((v+.055)/1.055)**2.4 for v in values]
        return sum(v*w for v,w in zip(linear,(.2126,.7152,.0722)))
    a,b=luminance(fg),luminance(bg)
    return (max(a,b)+.05)/(min(a,b)+.05)
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-whichkey-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        # A short delay keeps the test fast without changing what the popup
        # asserts: it must still be absent before the delay and present after.
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nwatch=false\nwhichkey_delay_ms=120\n'
        )
        file=root/"f.txt"
        file.write_text("hello world\nsecond line\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.pop("NO_COLOR",None)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"),
                              XDG_DATA_HOME=str(root/"data"),XDG_STATE_HOME=str(root/"state"))
            os.execv(binary,[binary,str(file)])
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
        def key(s,seconds=.2):os.write(fd,s.encode());drain(seconds)
        try:
            drain(.3)
            # Completed mapping before the delay must never show the popup
            # and must still run: ",h" clears search highlighting immediately.
            key("/world\r",.15)
            key(",h",.05)
            text="\n".join(screen.display)
            assert "Diagnostics list" not in text, \
                ("popup leaked into a completed mapping\n"+text)
            # A bare prefix left hanging past the delay shows the popup with
            # every ,l* continuation, but not unrelated bindings.
            key(",l",0)
            drain(.4)
            text="\n".join(screen.display)
            assert "Diagnostics list" in text, ("which-key popup missing\n"+text)
            assert "Format buffer" in text, ("which-key popup incomplete\n"+text)
            assert "Write current buffer" not in text, \
                ("which-key popup leaked unrelated bindings\n"+text)
            key("\x1b",.2)  # cancel back to Normal
            # Reproduce the screenshot's ,f prefix in every light/dark theme.
            # Inspect actual terminal cells, including the title and padding:
            # a bold title must remain readable without a yellow highlight.
            for theme in themes:
                key(":colorscheme "+theme+"\r",.12)
                key(",f",.3)
                header_rows=[y for y,line in enumerate(screen.display)
                             if line.rstrip().endswith(",f")]
                assert len(header_rows)==1, (theme,"prefix header missing",screen.display)
                y=header_rows[0];x=screen.display[y].rfind(",f")
                header=screen.buffer[y][x]
                assert contrast(header.fg,header.bg)>=3.0, (theme,"unreadable header",header)
                assert header.bold, (theme,"prefix header should be bold",header)
                if theme in ("default","mono","warm","cool"):
                    assert contrast(header.fg,header.bg)>=4.5, (theme,"unreadable classic popup",header)
                for row in range(y,rows-1):
                    for col in range(x-1,cols):
                        cell=screen.buffer[row][col]
                        assert (cell.fg,cell.bg)==(header.fg,header.bg), \
                            (theme,"popup colors differ",row,col,cell,header)
                        assert not cell.reverse, (theme,"unexpected reverse highlight",cell)
                        if row>y:
                            assert not cell.bold, (theme,"header styling leaked into items",cell)
                text="\n".join(screen.display)
                assert "Recent files" in text, (theme,"file actions missing",text)
                key("\x1b",.08)
            # :keymaps opens a searchable palette generated from the same
            # registry, and selecting an entry actually executes it.
            key(":keymaps\r",.3)
            text="\n".join(screen.display)
            assert "Keymaps" in text and "results" in text, \
                ("keymaps palette did not open\n"+text)
            key("/Toggle line wrap\r",.2)
            key("\r",.3)
            text="\n".join(screen.display)
            assert "wrap: false" in text, ("selecting a keymap entry did not run it\n"+text)
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
    print(f"Which-key PTY passed: {cols}x{rows}, {len(themes)} themes")
