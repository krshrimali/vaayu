#!/usr/bin/env python3
"""Actual clangd diagnostics, explicit formatting and format-on-save through a PTY.

Run explicitly with a working clangd on PATH. No mock server is used.
"""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte

binary = str(pathlib.Path(sys.argv[1]).resolve())
for cols, rows in [(40, 12), (100, 24), (180, 50)]:
    with tempfile.TemporaryDirectory(prefix='vaayu-real-clangd-') as tmp:
        root = pathlib.Path(tmp)
        cfg = root / 'config/vaayu'; cfg.mkdir(parents=True)
        (cfg / 'config.toml').write_text('jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
            'watch=false\nformat_on_save=true\n'
            '[lsp.clangd]\ncmd=["clangd", "--background-index=false"]\nfiletypes=["c"]\n')
        target = root / 'main.c'; target.write_text('int main(){return missing_symbol;}\n')
        pid, fd = pty.fork()
        if pid == 0:
            os.chdir(root)
            os.environ.update(TERM='xterm-256color', XDG_CONFIG_HOME=str(root / 'config'), XDG_DATA_HOME=str(root / 'data'))
            os.execv(binary, [binary, str(target)])
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack('HHHH', rows, cols, 0, 0))
        screen = pyte.Screen(cols, rows); stream = pyte.Stream(screen)
        decoder = codecs.getincrementaldecoder('utf-8')('replace')
        def drain(seconds=.1):
            end = time.monotonic() + seconds
            while time.monotonic() < end:
                ready, _, _ = select.select([fd], [], [], min(.02, max(0, end - time.monotonic())))
                if ready:
                    try: data = os.read(fd, 65536)
                    except OSError: break
                    if not data: break
                    stream.feed(decoder.decode(data))
        def key(value, seconds=.1): os.write(fd, value.encode()); drain(seconds)
        def wait_for(predicate):
            end = time.monotonic() + 15
            while time.monotonic() < end:
                if predicate(): return True
                drain()
            return False
        try:
            drain(.3)
            assert wait_for(lambda: screen.display[0].startswith('E')), ('clangd diagnostic missing', screen.display)
            key(',lf')
            assert wait_for(lambda: 'main() {' in '\n'.join(screen.display)), ('formatting missing', screen.display)
            key(':w\r')
            assert wait_for(lambda: 'main() {' in target.read_text()), ('formatted save missing', target.read_text())
            # A fresh unformatted disk version exercises the synchronous save path.
            target.write_text('int main(){return 0;}\n'); key(':e!\r'); key(':w\r')
            assert wait_for(lambda: 'main() {' in target.read_text()), ('format-on-save missing', target.read_text())
            assert 'return 0;' in target.read_text()
            key(':qa!\r')
            end = time.monotonic() + 3
            while time.monotonic() < end:
                done, status = os.waitpid(pid, os.WNOHANG)
                if done:
                    assert os.waitstatus_to_exitcode(status) == 0
                    pid = None; break
                drain(.02)
            assert pid is None, 'Editor failed to exit'
        finally:
            if pid:
                try: os.kill(pid, signal.SIGKILL)
                except ProcessLookupError: pass
                os.waitpid(pid, 0)
            os.close(fd)
    print(f'Real clangd PTY passed: {cols}x{rows}')
