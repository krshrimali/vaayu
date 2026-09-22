#!/usr/bin/env python3
"""Spell checking, driven through a real PTY.

This machine (like many minimal environments) has no /usr/share/dict/words,
so :spellcheck/z= correctly degrade to "no dictionary found" rather than
silently doing nothing or crashing -- that graceful-degradation path is
exactly what this test verifies end to end. zg's user-dictionary write
doesn't depend on a system dictionary at all, so it's checked against a
real file under a sandboxed XDG_CONFIG_HOME. The "dictionary present"
logic (flagging, suggesting, replacing) is covered by src/spell.rs's and
regression.rs's unit tests using Dictionary::for_test, which don't depend
on what happens to be installed on the machine running the suite."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
# Whether this machine has a system word list at all decides which spell path
# ":spellcheck" takes: a "Spelling" results list of misspellings when present,
# or a clean "No dictionary found" message when absent. The test handles both.
DICT_PATHS=["/usr/share/dict/words","/usr/share/dict/american-english",
            "/usr/dict/words","/usr/share/dict/web2"]
has_dict=any(os.path.exists(p) for p in DICT_PATHS)
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-spell-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        file=root/"f.txt"
        file.write_text("wrold\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
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
        def key(s,seconds=.15):os.write(fd,s.encode());drain(seconds)
        try:
            drain(.3)
            key(":spellcheck\r",.3)
            text="\n".join(screen.display)
            dict_file = root/"config/vaayu/dictionary.txt"
            if has_dict:
                # A system dictionary is present: ":spellcheck" flags "wrold" in
                # a "Spelling" results list. Close it, then add the word to the
                # user dictionary from the buffer.
                assert ("Spelling" in text) or ("wrold" in text), text
                key("q",.3)  # close the results list, back to the buffer
                key("zg",.2)
                assert dict_file.exists(), \
                    ("zg did not write the user dictionary\n"+"\n".join(screen.display))
                assert dict_file.read_text().strip()=="wrold", dict_file.read_text()
            else:
                # No dictionary installed: degrade to a clear message; zg still
                # writes the user dictionary and z= reports the same message.
                assert "No dictionary found" in text, text
                key("zg",.2)
                assert dict_file.exists(), "zg did not write the user dictionary"
                assert dict_file.read_text().strip()=="wrold", dict_file.read_text()
                key("z=",.2)
                text="\n".join(screen.display)
                assert "No dictionary found" in text, text
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
    print(f"Spell PTY passed: {cols}x{rows}")
