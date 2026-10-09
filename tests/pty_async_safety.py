#!/usr/bin/env python3
"""Delayed and disconnected LSP responses must not replace newer user edits."""
import codecs, fcntl, json, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
mock=str((pathlib.Path(__file__).parent/'mock_lsp.py').resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    for scenario in ['edited-format','switched-definition','disconnected-hover','timed-out-save']:
        with tempfile.TemporaryDirectory(prefix='vaayu-lsp-safety-') as tmp:
            root=pathlib.Path(tmp);cfg=root/'config/vaayu';cfg.mkdir(parents=True)
            log=root/'lsp.jsonl';release=pathlib.Path(str(log)+'.release')
            method={'edited-format':'textDocument/formatting','switched-definition':'textDocument/definition',
                    'disconnected-hover':'textDocument/hover','timed-out-save':'textDocument/formatting'}[scenario]
            flag='--disconnect-method' if scenario=='disconnected-hover' else '--gate-method'
            (cfg/'config.toml').write_text('jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\nwatch=false\n'
                f'format_on_save={str(scenario=="timed-out-save").lower()}\n'
                '[lsp.fixture]\n'+f'cmd=["python3",{json.dumps(mock)},{json.dumps(str(log))},{json.dumps(flag)},{json.dumps(method)}]\nfiletypes=["rust"]\n')
            target=root/'first.rs';target.write_text('abc def\nsecond\nthird\nfourth\ntarget_line\n')
            other=root/'other.rs';other.write_text('other contents\n')
            pid,fd=pty.fork()
            if pid==0:
                os.chdir(root);os.environ.update(TERM='xterm-256color',XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'))
                os.execv(binary,[binary,str(target)])
            fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack('HHHH',rows,cols,0,0))
            screen=pyte.Screen(cols,rows);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder('utf-8')('replace')
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
            def wait_for(pred,seconds=5):
                end=time.monotonic()+seconds
                while time.monotonic()<end:
                    if pred():return True
                    drain(.03)
                return False
            def requested():return log.exists() and any(json.loads(line).get('method')==method for line in log.read_text().splitlines())
            try:
                drain(.3)
                assert wait_for(lambda:'W' in screen.display[0][:2]), 'LSP did not initialize'
                if scenario=='edited-format':
                    key(':format\r');assert wait_for(requested)
                    key('iUSER \x1b');release.touch();drain(.4)
                    assert 'FMT' not in text() and 'USER abc' in text(), 'late format replaced newer edits'
                    key(':w\r');assert target.read_text().startswith('USER abc')
                elif scenario=='switched-definition':
                    key('gd');assert wait_for(requested)
                    key(':e other.rs\r');release.touch();drain(.4)
                    key('iUSER \x1b');key(':w\r')
                    assert other.read_text()=='USER other contents\n', 'late definition stole buffer focus'
                    assert target.read_text().startswith('abc def')
                elif scenario=='disconnected-hover':
                    key('K');assert wait_for(requested)
                    assert wait_for(lambda:'Language server exited' in text()), 'LSP disconnect was not reported'
                    key('iUSER \x1b');key(':w\r');assert target.read_text().startswith('USER abc')
                    key(':lsprestart\r');assert wait_for(lambda:'W' in screen.display[0][:2])
                else:
                    key('iUSER \x1b');key(':w\r',2.4);assert wait_for(requested)
                    assert target.read_text().startswith('USER abc'), 'timeout prevented saving'
                    release.touch();drain(.5)
                    assert 'FMT' not in text(), 'format-on-save applied after its timeout and plain save'
                    assert 'USER abc' in text(), 'late formatter changed the saved buffer'
                key(':qa!\r')
            finally:
                try:os.kill(pid,signal.SIGKILL)
                except ProcessLookupError:pass
                os.waitpid(pid,0);os.close(fd)
        print(f'LSP async safety passed: {scenario} {cols}x{rows}')
