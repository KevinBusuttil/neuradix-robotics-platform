"""Actual kernel resource probes; the Rust parent imposes an external timeout."""
import json
import mmap
import os
import resource
import signal
import subprocess
import sys
import time

import neuradix_worker

signal.alarm(10)
if len(sys.argv) > 1:
    with open(sys.argv[1], "w") as marker:
        marker.write("worker code executed")


def limits():
    return {"cpu": resource.getrlimit(resource.RLIMIT_CPU),
            "as": resource.getrlimit(resource.RLIMIT_AS)}


def send(value):
    neuradix_worker._send(value)


send({"kind": "ready", "name": "resources", "skipPolicy": "may-skip"})
for raw in sys.stdin.buffer:
    command = json.loads(raw)
    if command["kind"] == "shutdown":
        break
    seq = command["seq"]
    if command["kind"] == "ping":
        send({"kind": "pong", "seq": seq})
        continue
    action = command.get("payload")
    if action == "cpu":
        # Even ignoring a soft-limit signal cannot exceed the equal hard limit.
        signal.signal(signal.SIGXCPU, signal.SIG_IGN)
        send({"kind": "response", "seq": seq, "payload": "started"})
        while True:
            pass
    if action in ("exit", "signal"):
        send({"kind": "response", "seq": seq, "payload": "exiting"})
        # Separate successful request observation from the exit being audited.
        time.sleep(0.1)
        if action == "exit":
            os._exit(42)
        os.kill(os.getpid(), signal.SIGKILL)
    if action == "memory":
        try:
            bytearray(resource.getrlimit(resource.RLIMIT_AS)[0] + 4096)
        except MemoryError:
            send({"kind": "error", "seq": seq, "message": "MemoryError"})
            continue
        raise AssertionError("allocation exceeded address space")
    if action == "mmap":
        with mmap.mmap(-1, 8 * 1024 * 1024):
            pass
        try:
            mmap.mmap(-1, resource.getrlimit(resource.RLIMIT_AS)[0] + 4096)
        except OSError as error:
            result = {"errno": error.errno}
        else:
            raise AssertionError("mapping exceeded address space")
    elif action == "raise":
        denied = []
        for key in (resource.RLIMIT_CPU, resource.RLIMIT_AS):
            _, hard = resource.getrlimit(key)
            try:
                resource.setrlimit(key, (hard + 1, hard + 1))
            except (ValueError, PermissionError):
                denied.append(True)
            else:
                denied.append(False)
        result = {"denied": denied, **limits()}
    elif action == "descendant":
        code = "import json,resource; print(json.dumps({'cpu':resource.getrlimit(resource.RLIMIT_CPU),'as':resource.getrlimit(resource.RLIMIT_AS),'nnp':next(line.split()[1] for line in open('/proc/self/status') if line.startswith('NoNewPrivs:'))}))"
        result = json.loads(subprocess.check_output([sys.executable, "-c", code]))
    elif action == "environment":
        result = {key: os.environ.get(key) for key in ("LD_PRELOAD", "LD_LIBRARY_PATH", "HOME", "NEURADIX_WORKER_CONFIG")}
    elif action == "idle":
        time.sleep(1.1)
        result = limits()
    elif action == "forge-setup":
        send({"kind": "limits-v1", "cpu": 86400, "as": 1099511627776})
        result = limits()
    else:
        result = limits()
    send({"kind": "response", "seq": seq, "payload": result})
