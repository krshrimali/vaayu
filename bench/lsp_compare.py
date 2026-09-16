#!/usr/bin/env python3
"""Cold clangd formatting comparison through each editor's normal UI path."""
import argparse
import codecs
import fcntl
import json
import os
import pathlib
import pty
import select
import signal
import statistics
import struct
import tempfile
import termios
import time

import pyte


def run(command, keys, source, cwd, env, attempts=5, warmup=b"l"):
    samples = []
    failures = []
    for _ in range(attempts):
        source.write_text("int main(){int unused=1;return 0;}\n")
        pid, fd = pty.fork()
        if pid == 0:
            os.chdir(cwd)
            os.environ.update(env)
            os.execvp(command[0], command + [str(source)])
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 100, 0, 0))
        screen = pyte.Screen(100, 24)
        stream = pyte.Stream(screen)
        decoder = codecs.getincrementaldecoder("utf-8")("replace")

        def drain(seconds):
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                ready, _, _ = select.select([fd], [], [], min(0.02, deadline - time.monotonic()))
                if ready:
                    try:
                        data = os.read(fd, 65536)
                    except OSError:
                        break
                    if not data:
                        break
                    stream.feed(decoder.decode(data))

        # Let clangd initialize and force one ordinary editor event so every
        # client has sent didOpen before the formatting request. This measures
        # the warm request/edit/redraw path; process startup is measured by the
        # separate editor comparison harness.
        drain(1.5)
        os.write(fd, warmup)
        drain(1.0)
        started = time.perf_counter()
        os.write(fd, keys)
        deadline = started + 15
        success = False
        while time.perf_counter() < deadline:
            drain(0.02)
            rendered = "\n".join(screen.display)
            if "int main() {" in rendered or ("int main()" in rendered and "unused = 1" in rendered):
                success = True
                break
        elapsed = time.perf_counter() - started if success else None
        samples.append(elapsed)
        if not success:
            failures.append("\n".join(line.rstrip() for line in screen.display if line.strip())[-2000:])
        try:
            os.kill(pid, signal.SIGKILL)
            os.waitpid(pid, 0)
        except ProcessLookupError:
            pass
        os.close(fd)
    good = [s * 1000 for s in samples if s is not None]
    return {
        "n": len(samples),
        "successes": len(good),
            "warm_format_to_visible_p50_ms": round(statistics.median(good), 3) if good else None,
        "samples_ms": [round(s * 1000, 3) if s is not None else None for s in samples],
        "failure_screens": sorted(set(failures)),
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--vaayu", default="target/release/vaayu")
    parser.add_argument("--nvim", default="nvim")
    parser.add_argument("--helix", required=True)
    parser.add_argument("--helix-runtime", required=True)
    parser.add_argument("--out", default="bench/lsp-comparison.json")
    parser.add_argument("--attempts", type=int, default=5)
    args = parser.parse_args()
    root = pathlib.Path.cwd()
    with tempfile.TemporaryDirectory(prefix="vaayu-lsp-compare-") as temp:
        temp = pathlib.Path(temp)
        (temp / ".git").mkdir()
        source = temp / "main.c"
        vaayu_cfg = temp / "vaayu-config/vaayu"
        vaayu_cfg.mkdir(parents=True)
        (vaayu_cfg / "config.toml").write_text(
            "jk_escape=false\nclipboard_unnamedplus=false\n"
            "[lsp.clangd]\ncmd=['clangd','--background-index=false']\n"
            "filetypes=['c']\nroot_markers=['.git']\n"
        )
        nvim_cfg = temp / "nvim-config/nvim"
        nvim_cfg.mkdir(parents=True)
        (nvim_cfg / "init.lua").write_text(
            "vim.opt.swapfile=false\n"
            "vim.api.nvim_create_autocmd('BufEnter',{once=true,callback=function() "
            "vim.lsp.start({name='clangd',cmd={'clangd','--background-index=false'},root_dir=vim.fn.getcwd()}) end})\n"
            "vim.keymap.set('n',',lf',function() vim.lsp.buf.format({async=false,timeout_ms=10000}) end)\n"
        )
        base = {"TERM": "xterm-256color"}
        results = {
            "method": "fresh processes; warm clangd format request through visible edit",
            "vaayu": run(
                [str(pathlib.Path(args.vaayu).resolve())], b",lf", source, temp,
                {**base, "XDG_CONFIG_HOME": str(temp / "vaayu-config")},
                attempts=args.attempts,
            ),
            "nvim_bare_lsp": run(
                [args.nvim], b",lf", source, temp,
                {**base, "XDG_CONFIG_HOME": str(temp / "nvim-config")},
                attempts=args.attempts,
            ),
            "helix": run(
                [str(pathlib.Path(args.helix).resolve())], b":format\r", source, temp,
                {**base, "XDG_CONFIG_HOME": str(temp / "empty"), "HELIX_RUNTIME": str(pathlib.Path(args.helix_runtime).resolve())},
                attempts=args.attempts,
                warmup=b":hover\r",
            ),
        }
        pathlib.Path(args.out).write_text(json.dumps(results, indent=2) + "\n")
        print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
