#!/usr/bin/env python3
"""What the local server refuses, spoken over raw sockets. Any page the user
visits can reach loopback, so these are the checks standing between a hostile
site and a session that can run shell commands."""
import base64, http.client, json, os, pathlib, re, signal, socket, struct, subprocess, sys, tempfile, time

binary = str(pathlib.Path(sys.argv[1]).resolve())
failures = []


def check(name, ok, detail=""):
    print(f"  {'ok  ' if ok else 'FAIL'} {name} {'' if ok else detail}")
    if not ok:
        failures.append(name)


def http_get(port, path, headers=None, method="GET", body=None):
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
    connection.putrequest(method, path, skip_host=True, skip_accept_encoding=True)
    merged = {"Host": f"127.0.0.1:{port}", **(headers or {})}
    for key, value in merged.items():
        if value is not None:
            connection.putheader(key, value)
    if body is not None:
        connection.putheader("Content-Length", str(len(body)))
    connection.endheaders(body.encode() if body is not None else None)
    response = connection.getresponse()
    data = response.read()
    connection.close()
    return response.status, dict(response.getheaders()), data


class WS:
    """Just enough of a WebSocket client to attach, send and read frames."""

    def __init__(self, port, cookie=None, origin=None, host=None):
        self.sock = socket.create_connection(("127.0.0.1", port), timeout=5)
        key = base64.b64encode(os.urandom(16)).decode()
        lines = [
            "GET /ws HTTP/1.1",
            f"Host: {host or f'127.0.0.1:{port}'}",
            "Upgrade: websocket",
            "Connection: Upgrade",
            f"Sec-WebSocket-Key: {key}",
            "Sec-WebSocket-Version: 13",
        ]
        if cookie:
            lines.append(f"Cookie: {cookie}")
        if origin:
            lines.append(f"Origin: {origin}")
        self.sock.sendall(("\r\n".join(lines) + "\r\n\r\n").encode())
        self.buffer = b""
        while b"\r\n\r\n" not in self.buffer:
            chunk = self.sock.recv(65536)
            if not chunk:
                break
            self.buffer += chunk
        head, _, self.buffer = self.buffer.partition(b"\r\n\r\n")
        self.status = int(head.split(b" ")[1]) if head else 0

    def send(self, message):
        data = json.dumps(message).encode()
        mask = os.urandom(4)
        header = bytes([0x81])
        if len(data) < 126:
            header += bytes([0x80 | len(data)])
        else:
            header += bytes([0x80 | 126]) + struct.pack(">H", len(data))
        self.sock.sendall(header + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(data)))

    def recv(self, timeout=5):
        self.sock.settimeout(timeout)
        def need(n):
            while len(self.buffer) < n:
                chunk = self.sock.recv(65536)
                if not chunk:
                    raise EOFError
                self.buffer += chunk
        need(2)
        opcode = self.buffer[0] & 0x0f
        length = self.buffer[1] & 0x7f
        offset = 2
        if length == 126:
            need(4)
            length = struct.unpack(">H", self.buffer[2:4])[0]
            offset = 4
        elif length == 127:
            need(10)
            length = struct.unpack(">Q", self.buffer[2:10])[0]
            offset = 10
        need(offset + length)
        payload = self.buffer[offset:offset + length]
        self.buffer = self.buffer[offset + length:]
        return opcode, payload

    def read_until(self, needle, timeout=5):
        seen = b""
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            try:
                opcode, payload = self.recv(max(0.05, end - time.monotonic()))
            except (socket.timeout, EOFError):
                break
            seen += payload
            if needle in seen:
                return seen
        return None


with tempfile.TemporaryDirectory(prefix="vaayu-web-server-") as directory:
    tmp = pathlib.Path(directory)
    project = tmp / "proj"
    project.mkdir()
    (project / "file.txt").write_text("server test\n")
    (project / "other.txt").write_text("the other file\n")
    (tmp / "cfg/vaayu").mkdir(parents=True)
    (tmp / "cfg/vaayu/config.toml").write_text("jk_escape=false\nprogress=false\n")
    env = dict(os.environ, XDG_CONFIG_HOME=str(tmp / "cfg"), XDG_DATA_HOME=str(tmp / "data"),
               HOME=str(tmp), BROWSER="true")
    process = subprocess.Popen([binary, "--web", "--no-open", "--port", "0", "file.txt"], cwd=project,
                               env=env, stdout=subprocess.PIPE, text=True)
    url = process.stdout.readline().strip()
    match = re.fullmatch(r"http://127\.0\.0\.1:(\d+)/\?token=([0-9a-f]{48})", url)
    check("prints a loopback URL with a long token", match is not None, url)
    port, token = int(match[1]), match[2]
    cookie = f"vaayu_{port}={token}"
    try:
        listening = subprocess.run(["ss", "-ltnH", f"sport = :{port}"], capture_output=True, text=True).stdout
        check("listens on loopback only", listening.split()[3] == f"127.0.0.1:{port}", listening)
        state = project / ".vaayu/web.json"
        check("state file is private", state.stat().st_mode & 0o077 == 0, oct(state.stat().st_mode))
        check("state dir is private", state.parent.stat().st_mode & 0o077 == 0)

        status, _, _ = http_get(port, "/")
        check("no token: page refused", status == 401, status)
        status, _, _ = http_get(port, "/?token=" + "0" * 48)
        check("wrong token: page refused", status == 401, status)
        status, _, _ = http_get(port, "/app.js")
        check("no token: script refused", status == 401, status)
        status, headers, _ = http_get(port, f"/?token={token}")
        check("token exchanged for a cookie", status == 302 and headers.get("Location") == "/", (status, headers))
        flags = headers.get("Set-Cookie", "")
        check("cookie is HttpOnly and SameSite=Strict", "HttpOnly" in flags and "SameSite=Strict" in flags, flags)
        status, headers, body = http_get(port, "/", {"Cookie": cookie})
        check("cookie serves the page", status == 200 and b"<canvas" in body, status)
        policy = headers.get("Content-Security-Policy", "")
        check("page has a strict CSP", "default-src 'none'" in policy and "frame-ancestors 'none'" in policy, policy)
        check("responses are not sniffed or cached",
              headers.get("X-Content-Type-Options") == "nosniff" and headers.get("Cache-Control") == "no-store")

        status, _, _ = http_get(port, "/", {"Host": "evil.example", "Cookie": cookie})
        check("DNS rebinding: foreign Host refused", status == 403, status)
        status, _, _ = http_get(port, f"/?token={token}", {"Host": "127.0.0.1.evil.example"})
        check("DNS rebinding: lookalike Host refused", status == 403, status)
        status, _, _ = http_get(port, "/", {"Origin": "https://evil.example", "Cookie": cookie})
        check("foreign Origin refused", status == 403, status)
        status, _, _ = http_get(port, "/../../etc/passwd", {"Cookie": cookie})
        check("no path traversal", status == 404, status)
        status, _, _ = http_get(port, "/nope", {"Cookie": cookie})
        check("unknown path is 404", status == 404, status)

        check("WebSocket without a cookie refused", WS(port).status == 401)
        check("WebSocket from another site refused", WS(port, cookie, origin="https://evil.example").status == 403)
        check("WebSocket from another local port refused",
              WS(port, cookie, origin="http://127.0.0.1:1").status == 403)
        check("WebSocket with a foreign Host refused", WS(port, cookie, host="evil.example:80").status == 403)

        status, _, _ = http_get(port, "/api/ping")
        check("API needs the token", status == 401 or status == 404, status)
        status, _, _ = http_get(port, "/api/open", method="POST", body="/etc/passwd")
        check("opening a file needs the token", status in (401, 404), status)

        # Malformed input must not bring the session down.
        raw = socket.create_connection(("127.0.0.1", port), timeout=5)
        raw.sendall(b"\x00\xff garbage\r\n\r\n")
        raw.close()
        raw = socket.create_connection(("127.0.0.1", port), timeout=5)
        raw.sendall(b"GET / HTTP/1.1\r\nX: " + b"a" * 70000 + b"\r\n\r\n")
        raw.close()

        ws = WS(port, cookie, origin=f"http://127.0.0.1:{port}")
        check("WebSocket with cookie and own Origin attaches", ws.status == 101, ws.status)
        opcode, payload = ws.recv()
        hello = json.loads(payload)
        check("hello carries the tables", hello["t"] == "hello" and len(hello["diacritics"]) == 256 and hello["widths"])
        for bad in ({"t": "nonsense"}, {"t": "key"}, {"t": "mouse", "kind": "down"}, {"t": "size", "cols": 0, "rows": 0, "cw": 0, "ch": 0}):
            ws.send(bad)
        ws.sock.sendall(b"\x81\x84\x00\x00\x00\x00{{{{")
        ws.send({"t": "size", "cols": 80, "rows": 24, "cw": 8, "ch": 16})
        check("first frame paints the file", ws.read_until(b"server test") is not None)
        ws.send({"t": "key", "key": "G"})
        check("session survived the garbage", process.poll() is None and ws.read_until(b"H") is not None)

        # Frames must keep flowing however small the grid gets.
        for cols, rows in ((1, 1), (2, 2), (500, 200), (80, 24)):
            ws.send({"t": "size", "cols": cols, "rows": rows, "cw": 8, "ch": 16})
        ws.send({"t": "key", "key": "G"})
        check("extreme grid sizes survive", process.poll() is None and ws.read_until(b"NORMAL") is not None)

        reuse = subprocess.run([binary, "--web", "--no-open", "other.txt"], cwd=project, env=env,
                               capture_output=True, text=True, timeout=10)
        check("a second launch reuses the session", reuse.returncode == 0 and reuse.stdout.startswith(url),
              (reuse.returncode, reuse.stdout, reuse.stderr))
        check("the second launch's file opens in it", ws.read_until(b"the other file") is not None)

        # Ending with unsaved work leaves recovery data; SIGTERM must be clean.
        ws.send({"t": "key", "key": "i"})
        for c in "recoverable":
            ws.send({"t": "key", "key": c})
        ws.send({"t": "key", "key": "Escape"})
        ws.read_until(b"recoverable")
        time.sleep(1.5)
        process.send_signal(signal.SIGTERM)
        check("SIGTERM stops the server", process.wait(timeout=8) == 0, process.returncode)
        check("SIGTERM removes the state file", not state.exists())
        check("unsaved edits stay recoverable", list((project / ".vaayu").glob("recovery-*.json")))
        check("the tab is told", ws.read_until(b'"bye"', 2) is not None)
    finally:
        if process.poll() is None:
            process.kill()
        process.wait()

    # --help prints and exits, without starting an editor or a server.
    for args in (["--help"], ["-h"], ["--web", "--help"], ["some-file.txt", "--help"]):
        result = subprocess.run([binary, *args], cwd=project, env=env, stdin=subprocess.DEVNULL,
                                capture_output=True, text=True, timeout=10)
        check(f"{' '.join(args)} prints the help",
              result.returncode == 0 and result.stdout.startswith("Vaayu") and "--host ADDR" in result.stdout
              and result.stderr == "", result.stderr)
    check("--help creates no file", not (project / "some-file.txt").exists())

    # --host: reachable from elsewhere, with the token and Origin checks intact.
    route = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    route.connect(("192.0.2.1", 9))
    lan = route.getsockname()[0]
    route.close()
    process = subprocess.Popen([binary, "--web", "--no-open", "--host", "0.0.0.0", "--port", "0"], cwd=project,
                               env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    url = process.stdout.readline().strip()
    other_machine = process.stdout.readline().strip()
    warning = process.stderr.readline()
    match = re.fullmatch(r"http://127\.0\.0\.1:(\d+)/\?token=([0-9a-f]{48})", url)
    check("--host: first line is still the local link", match is not None, url)
    port, token = int(match[1]), match[2]
    cookie = f"vaayu_{port}={token}"
    lan_host = f"{lan}:{port}"

    def lan_get(path, headers):
        with socket.create_connection((lan, port), timeout=5) as stream:
            lines = [f"GET {path} HTTP/1.1", *[f"{k}: {v}" for k, v in headers.items()], "Connection: close", "", ""]
            stream.sendall("\r\n".join(lines).encode())
            return int(stream.makefile("rb").readline().split()[1])

    try:
        listening = subprocess.run(["ss", "-ltnH", f"sport = :{port}"], capture_output=True, text=True).stdout
        check("--host 0.0.0.0 listens on every interface", listening.split()[3] == f"0.0.0.0:{port}", listening)
        check("--host prints a link for other machines", other_machine.endswith(f"http://{lan_host}/?token={token}"), other_machine)
        check("--host warns about clear text", warning.startswith("Warning:") and "encryption" in warning, warning)
        check("--host: no token refused", lan_get("/", {"Host": lan_host}) == 401)
        check("--host: wrong token refused", lan_get("/?token=" + "0" * 48, {"Host": lan_host}) == 401)
        check("--host: token accepted", lan_get(f"/?token={token}", {"Host": lan_host}) == 302)
        check("--host: cookie accepted", lan_get("/", {"Host": lan_host, "Cookie": cookie}) == 200)
        check("--host: cross-site Origin refused",
              lan_get("/", {"Host": lan_host, "Cookie": cookie, "Origin": "http://evil.example"}) == 403)
        check("--host: malformed Host refused", lan_get("/", {"Host": "a;script-src *", "Cookie": cookie}) == 403)
        check("--host: WebSocket without cookie refused", WS(port, host=lan_host).status == 401)
        check("--host: WebSocket with cross-site Origin refused",
              WS(port, cookie, origin="http://evil.example", host=lan_host).status == 403)
        conflict = subprocess.run([binary, "--web", "--no-open", "--host", "127.0.0.1"], cwd=project, env=env,
                                  capture_output=True, text=True, timeout=10)
        check("--host: a conflicting address is an error, not silent reuse",
              conflict.returncode != 0 and "0.0.0.0" in conflict.stderr, conflict.stderr)
        reuse = subprocess.run([binary, "--web", "--no-open"], cwd=project, env=env,
                               capture_output=True, text=True, timeout=10)
        check("--host: reusing an exposed session repeats the warning",
              reuse.returncode == 0 and reuse.stderr.startswith("Warning:"), reuse.stderr)
    finally:
        process.terminate()
        process.wait(timeout=10)

    result = subprocess.run([binary, "--web", "--no-open", "--host", "not-an-ip"], cwd=project, env=env,
                            capture_output=True, text=True, timeout=10)
    check("--host needs an IP address", result.returncode != 0 and "not-an-ip" in result.stderr, result.stderr)

    # --port on a taken port is an error, not a silently different address.
    with socket.socket() as taken:
        taken.bind(("127.0.0.1", 0))
        taken.listen()
        busy = taken.getsockname()[1]
        result = subprocess.run([binary, "--web", "--no-open", "--port", str(busy)], cwd=project, env=env,
                                capture_output=True, text=True, timeout=10)
        check("an occupied --port fails loudly", result.returncode != 0 and str(busy) in result.stderr, result.stderr)
    result = subprocess.run([binary, "--web", "--bogus"], cwd=project, env=env, capture_output=True, text=True, timeout=10)
    check("an unknown flag is an error", result.returncode != 0 and "--bogus" in result.stderr, result.stderr)

if failures:
    print(f"{len(failures)} failed:", *failures, sep="\n  - ")
    sys.exit(1)
print("web server checks passed")
