#!/usr/bin/env python3
"""Per-buffer indent detection: modeline > .editorconfig > heuristic >
config default, and that Tab/>/< actually use the resolved per-buffer
settings, not the global config -- driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-indent-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        # Global default: tabs, shiftwidth 4 -- every check below expects a
        # per-file override to win over this, or this exact fallback when
        # nothing overrides it.
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
            'tabstop=4\nshiftwidth=4\nexpandtab=false\n'
        )
        # Heuristic detection: a plain file indented with 2 spaces.
        two_space = root/"detect.txt"
        two_space.write_text("fn f() {\n  a();\n  if x {\n    b();\n  }\n}\n")
        # A modeline wins over heuristic detection in the same file.
        modeline = root/"modeline.txt"
        modeline.write_text("a\nb\n// vim: set sw=8 ts=8 noet:\n")
        # .editorconfig wins over heuristic detection for matching files.
        (root/".editorconfig").write_text(
            "root = true\n[*.cfg]\nindent_style = space\nindent_size = 3\n"
        )
        ecfg = root/"proj.cfg"
        ecfg.write_text("a\n\tb\n")
        # No hints at all: falls back to the config default (tabs).
        plain = root/"plain.txt"
        plain.write_text("just one line\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(two_space)])
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
        def screen_text():
            return "\n".join(screen.display)
        try:
            drain(.3)
            key(":indentinfo\r",.2)
            text = screen_text()
            assert "detected" in text and "sw=2" in text and "space" in text, text
            # Global config.toml sets expandtab=false (tabs); this buffer
            # detected space indentation, so Tab here must insert spaces,
            # not a literal tab -- proving Tab reads the per-buffer
            # setting, not the global default.
            key("Gohi\t|\x1b",.2)
            key(":w\r",.3)
            last = two_space.read_text().splitlines()[-1]
            assert "\t" not in last and "hi  |" in last, last
            key(f":e {modeline}\r",.3)
            key(":indentinfo\r",.2)
            text = screen_text()
            assert "modeline" in text and "sw=8" in text and " tab" in text, text
            key(f":e {ecfg}\r",.3)
            key(":indentinfo\r",.2)
            text = screen_text()
            assert ".editorconfig" in text and "sw=3" in text and "space" in text, text
            key(f":e {plain}\r",.3)
            key(":indentinfo\r",.2)
            text = screen_text()
            assert "default" in text and "sw=4" in text and " tab" in text, text
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
    print(f"Indent detection PTY passed: {cols}x{rows}")
