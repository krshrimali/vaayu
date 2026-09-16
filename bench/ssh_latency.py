#!/usr/bin/env python3
"""Measure real SSH PTY first-response latency against an installed remote binary.
Requires an explicitly supplied host with working noninteractive authentication.
Uploads only the supplied benchmark source into a private temporary directory.
"""
import argparse, json, pathlib, shlex, subprocess
from latency import run_session, summarize
p=argparse.ArgumentParser()
p.add_argument("host");p.add_argument("--binary",default="vaayu")
p.add_argument("--file",required=True);p.add_argument("--cols",type=int,default=100)
p.add_argument("--rows",type=int,default=40);p.add_argument("--out")
a=p.parse_args()
ssh=["ssh","-oBatchMode=yes","-oConnectTimeout=5","--",a.host]
remote=subprocess.check_output(ssh+["umask 077; mktemp -d /tmp/vaayu-ssh.XXXXXXXX"],text=True,timeout=15).strip()
if not remote.startswith("/tmp/vaayu-ssh.") or any(c not in "/.-_abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789" for c in remote):
    raise RuntimeError("Unexpected remote temporary path")
try:
    target=remote+"/source"+pathlib.Path(a.file).suffix
    subprocess.run(ssh+["cat > "+shlex.quote(target)],input=pathlib.Path(a.file).read_bytes(),check=True,timeout=30)
    # SSH joins command arguments remotely. Quote each remote argument explicitly.
    command="cd "+shlex.quote(remote)+" && exec env TERM=xterm-256color "+shlex.quote(a.binary)
    samples,timeouts=run_session(["ssh","-tt","-oBatchMode=yes","--",a.host,command],shlex.quote(target),a.cols,a.rows,warmup=3)
    result={"transport":"real SSH","host":a.host,"binary":a.binary,"cols":a.cols,"rows":a.rows,**summarize(samples),"timeouts":timeouts}
    text=json.dumps(result,indent=2);print(text)
    if a.out:pathlib.Path(a.out).write_text(text+"\n")
finally:
    subprocess.run(ssh+["rm -rf -- "+shlex.quote(remote)],timeout=15,check=True)
