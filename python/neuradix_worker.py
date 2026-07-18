"""Neuradix Python worker runtime.

A component author writes an ordinary handler function and calls ``run(handler)``.
This module implements the newline-delimited JSON protocol the Rust supervisor
(``neuradix-python``) speaks over stdin/stdout:

* on startup the worker emits ``{"kind": "ready", "name", "skipPolicy"}``;
* the supervisor sends ``{"kind": "request", "seq", "payload"}`` lines;
* the worker replies ``{"kind": "response", "seq", "payload"}`` or
  ``{"kind": "error", "seq", "message"}``;
* ``{"kind": "ping", "seq"}`` is answered with ``{"pong": true}``;
* ``{"kind": "shutdown"}`` ends the loop.

Only protocol JSON goes to stdout; logs and tracebacks go to stderr, so a crash
is visible and the supervisor observes it as a clean process exit.
"""

import json
import os
import sys


def _send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


def log(message):
    """Write a diagnostic line to stderr (never stdout)."""
    sys.stderr.write(str(message) + "\n")
    sys.stderr.flush()


def run(handler, name="python-worker", skip_policy="may-skip"):
    """Run the worker loop, dispatching each request payload to ``handler``.

    ``handler`` is called as ``handler(payload, config)`` and returns a
    JSON-serialisable result. The structured ``config`` is read once from the
    ``NEURADIX_WORKER_CONFIG`` environment variable.
    """
    try:
        config = json.loads(os.environ.get("NEURADIX_WORKER_CONFIG", "null"))
    except json.JSONDecodeError:
        config = None

    _send({"kind": "ready", "name": name, "skipPolicy": skip_policy})

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            message = json.loads(line)
        except json.JSONDecodeError as exc:
            _send({"kind": "error", "seq": -1, "message": f"bad request json: {exc}"})
            continue

        kind = message.get("kind")
        seq = message.get("seq", -1)

        if kind == "shutdown":
            break
        if kind == "ping":
            _send({"kind": "response", "seq": seq, "payload": {"pong": True}})
            continue
        if kind == "request":
            try:
                result = handler(message.get("payload"), config)
                _send({"kind": "response", "seq": seq, "payload": result})
            except Exception as exc:  # noqa: BLE001 - report any handler error
                _send({"kind": "error", "seq": seq, "message": str(exc)})
            continue

        _send({"kind": "error", "seq": seq, "message": f"unknown kind: {kind}"})
