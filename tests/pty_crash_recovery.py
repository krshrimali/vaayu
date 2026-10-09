#!/usr/bin/env python3
"""SIGKILL recovery restores unsaved buffers without overwriting disk or newer edits."""
import codecs, fcntl, json, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix='vaayu-crash-recovery-') as tmp:
        root=pathlib.Path(tmp);cfg=root/'config/vaayu';cfg.mkdir(parents=True)
        (cfg/'config.toml').write_text('jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\nwatch=false\n')
        first=root/'first.txt';first.write_text('first original\n')
        second=root/'second.txt';second.write_text('second original\n')
        def start():
            pid,fd=pty.fork()
            if pid==0:
                os.chdir(root);os.environ.update(TERM='xterm-256color',XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'))
                os.execv(binary,[binary,str(first)])
            fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack('HHHH',rows,cols,0,0))
            return pid,fd,pyte.Screen(cols,rows)
        pid,fd,screen=start();stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder('utf-8')('replace')
        def drain(seconds=.15):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                ready,_,_=select.select([fd],[],[],min(.02,max(0,end-time.monotonic())))
                if ready:
                    try:data=os.read(fd,65536)
                    except OSError:break
                    if not data:break
                    stream.feed(decoder.decode(data))
        def key(value,seconds=.15):os.write(fd,value.encode());drain(seconds)
        def text():return '\n'.join(screen.display)
        snapshot=root/'.vaayu'/f'recovery-{pid}.json'
        try:
            drain(.3);key('iFIRST-DRAFT 世界 \x1b');key(':e second.txt\r');key('iSECOND-DRAFT café \x1b')
            end=time.monotonic()+5
            while time.monotonic()<end:
                if snapshot.exists() and len(json.loads(snapshot.read_text()))==2:break
                drain(.05)
            assert snapshot.exists(), 'recovery checkpoint was not written'
            drafts=json.loads(snapshot.read_text());assert len(drafts)==2
            assert first.read_text()=='first original\n' and second.read_text()=='second original\n'
            assert snapshot.stat().st_mode & 0o777 == 0o600
            os.kill(pid,signal.SIGKILL);os.waitpid(pid,0);os.close(fd)
            pid,fd,screen=start();stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder('utf-8')('replace')
            drain(.3);key(':recover\r')
            assert 'Recovery drafts' in text(), text()
            key('/first.txt\r');key('\r')
            assert 'FIRST-DRAFT' in text(), 'crashed draft was not restored'
            assert first.read_text()=='first original\n', 'restore wrote disk without save'
            key('u');assert 'FIRST-DRAFT' not in text(), 'recovery restore was not undoable'
            key('iNEWER \x1b');key(':recover\r');key('/first.txt\r');key('\r')
            assert 'Save or discard' in text(), 'recovery replaced newer unsaved edits'
            key('q');assert 'NEWER first original' in text() and 'FIRST-DRAFT' not in text()
            key(':e second.txt\r');key(':recover\r');key('/second.txt\r');key('\r')
            assert 'SECOND-DRAFT' in text();assert second.read_text()=='second original\n'
            key(':w\r');assert second.read_text()=='SECOND-DRAFT café second original\n'
            key(':qa!\r')
        finally:
            try:os.kill(pid,signal.SIGKILL)
            except ProcessLookupError:pass
            os.waitpid(pid,0);os.close(fd)
    print(f'Crash recovery passed: {cols}x{rows}')
