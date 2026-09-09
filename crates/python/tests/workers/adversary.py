"""Real hostile stdio fixtures. Self-expiry also cleans up failed test runs."""
import json
import os
import subprocess
import sys
import threading
import time

threading.Timer(6.0, lambda: os._exit(124)).start()
mode = sys.argv[1]


def send(value):
    os.write(1, json.dumps(value, separators=(",", ":")).encode() + b"\n")


if mode == "retry_handshake":
    counter = sys.argv[2]
    try:
        with open(counter) as source:
            count = int(source.read())
    except FileNotFoundError:
        count = 0
    with open(counter, "w") as target:
        target.write(str(count + 1))
    if count:
        time.sleep(6)
if mode == "no_handshake":
    time.sleep(6)
if mode == "bad_handshake":
    os.write(1, b"not-json\n")
    time.sleep(6)
if mode == "huge_handshake":
    os.write(1, b"x" * 4096)
    time.sleep(6)

send({"kind": "ready", "name": "test", "skipPolicy": "may-skip"})
if mode == "blocked_stdin":
    time.sleep(6)
if mode == "write_then_hang":
    time.sleep(0.55)

for raw in sys.stdin.buffer:
    command = json.loads(raw)
    if command["kind"] == "shutdown":
        if mode in ("ignore_shutdown", "descendant"):
            time.sleep(6)
        os._exit(0)
    seq = command["seq"]
    payload = command.get("payload")
    if mode in ("hang", "ignore_shutdown", "write_then_hang"):
        time.sleep(6)
    elif mode == "close_stdout":
        os.close(1)
        time.sleep(6)
    elif mode == "flood":
        while True:
            os.write(1, b'{"kind":"log"}\n' * 64)
    elif mode == "slow_noise":
        while True:
            send({"kind": "response", "seq": 0, "payload": None})
            time.sleep(0.003)
    elif mode == "oversized":
        os.write(1, b"x" * 4096)
        time.sleep(6)
    elif mode == "invalid_json":
        os.write(1, b"{\xff}\n")
        time.sleep(6)
    elif mode == "queue_bytes":
        # Two valid lines, each below 160 bytes, but a combined burst >192.
        line = b'{"kind":"log","text":"' + b"x" * 110 + b'"}\n'
        os.write(1, line * 2)
        time.sleep(6)
    elif mode == "descendant":
        child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(5)"])
        send({"kind": "response", "seq": seq, "payload": {"pid": child.pid}})
        # Leader stays alive so the next operation times out while descendant
        # holds the pipe. Cleanup must kill the group without waiting for EOF.
        time.sleep(6)
    elif mode == "exit_with_descendant":
        child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(5)"])
        with open(sys.argv[2], "w") as target:
            target.write(str(child.pid))
        os._exit(13)
    elif mode == "retry_handshake":
        os._exit(13)
    elif mode == "sized_response":
        empty = {"kind": "response", "seq": seq, "payload": ""}
        base = len(json.dumps(empty, separators=(",", ":")).encode()) + 1
        empty["payload"] = "x" * (payload["bytes"] - base)
        send(empty)
    elif mode == "remote_error":
        if payload:
            send({"kind": "error", "seq": seq, "message": "expected application error"})
        else:
            send({"kind": "response", "seq": seq, "payload": seq})
    else:
        send({"kind": "response", "seq": seq, "payload": {"seq": seq, "bytes": len(raw)}})
