"""Adversarial heartbeat sessions; external Rust test deadlines are mandatory."""
import json
import os
import sys
import threading
import time

threading.Timer(6.0, lambda: os._exit(124)).start()
mode = sys.argv[1]
if mode in ("replace", "failed_handshake"):
    path = sys.argv[2]
    try:
        with open(path) as source:
            count = int(source.read())
    except FileNotFoundError:
        count = 0
    with open(path, "w") as target:
        target.write(str(count + 1))
    if count and mode == "failed_handshake":
        time.sleep(6)
    mode = "healthy" if count else "hang"


def send(value):
    os.write(1, json.dumps(value, separators=(",", ":")).encode() + b"\n")


ready = {"kind": "ready", "name": "heartbeat", "skipPolicy": "may-skip"}
if mode.startswith("preplay"):
    prefix = json.dumps(ready, separators=(",", ":")).encode() + b"\n"
    forged = b'{"kind":"pong","seq":1}\n'
    if mode == "preplay_response":
        forged = b'{"kind":"response","seq":1,"payload":true}\n'
    if mode == "preplay_partial":
        os.write(1, prefix + forged[:-2])
        time.sleep(0.15)
        os.write(1, forged[-2:])
    else:
        os.write(1, prefix + forged)
    time.sleep(6)
send(ready)
previous = 0
for raw in sys.stdin.buffer:
    value = json.loads(raw)
    if value["kind"] == "shutdown":
        os._exit(0)
    seq = value["seq"]
    if value["kind"] == "request":
        payload = value.get("payload")
        if payload == "hang":
            time.sleep(6)
        elif payload == "remote":
            send({"kind": "error", "seq": seq, "message": "application failure"})
        else:
            send({"kind": "response", "seq": seq, "payload": seq})
        continue
    if mode == "hang":
        time.sleep(6)
    if mode == "late":
        time.sleep(0.45)
    if mode == "flood":
        while True:
            os.write(1, b'{"kind":"pong","seq":0}\n' * 64)
    if mode == "unrelated" or (mode == "duplicate" and previous):
        while True:
            for frame in ({"kind": "pong", "seq": previous},
                          {"kind": "response", "seq": seq, "payload": {"pong": True}},
                          {"kind": "error", "seq": seq, "message": "not a pong"}):
                send(frame)
                time.sleep(0.01)
    send({"kind": "pong", "seq": seq})
    previous = seq
