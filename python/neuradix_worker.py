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


def _limit(name):
    value = int(os.environ.get(name, "65536"))
    if not 64 <= value <= 1048576:
        raise ValueError(f"invalid {name}")
    return value


def _check_value(value, limit, depth=0, remaining=None):
    """Bound recursion/visits before encoding; handler memory is not contained."""
    if remaining is None:
        remaining = [limit]
    remaining[0] -= 1
    if depth > 32 or remaining[0] < 0:
        raise ValueError("JSON exceeds depth/node limit")
    if isinstance(value, str) and len(value) > limit:
        raise ValueError("JSON string exceeds wire limit")
    if isinstance(value, dict):
        for key, item in value.items():
            if not isinstance(key, str) or len(key) > limit:
                raise ValueError("invalid JSON object key")
            _check_value(item, limit, depth + 1, remaining)
    elif isinstance(value, (list, tuple)):
        for item in value:
            _check_value(item, limit, depth + 1, remaining)


def _send(obj):
    limit = _limit("NEURADIX_WORKER_MAX_OUTPUT_BYTES")
    _check_value(obj, limit)
    data = bytearray()
    encoder = json.JSONEncoder(ensure_ascii=False, allow_nan=False, separators=(",", ":"))
    for piece in encoder.iterencode(obj):
        encoded = piece.encode("utf-8")
        if len(data) + len(encoded) + 1 > limit:
            raise ValueError("worker output exceeds wire limit")
        data.extend(encoded)
    data.append(10)
    sys.stdout.buffer.write(data)
    sys.stdout.buffer.flush()


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
    input_limit = _limit("NEURADIX_WORKER_MAX_INPUT_BYTES")
    _limit("NEURADIX_WORKER_MAX_OUTPUT_BYTES")
    raw_config = os.environ.get("NEURADIX_WORKER_CONFIG", "null")
    if len(raw_config.encode("utf-8")) > input_limit:
        raise ValueError("startup config exceeds wire limit")
    config = json.loads(raw_config)

    _send({"kind": "ready", "name": name, "skipPolicy": skip_policy})

    while True:
        line = sys.stdin.buffer.readline(input_limit + 1)
        if not line:
            break
        if len(line) > input_limit or not line.endswith(b"\n"):
            raise ValueError("unterminated or oversized request")
        try:
            message = json.loads(line)
            _check_value(message, input_limit)
            if not isinstance(message, dict):
                raise ValueError("request must be an object")
        except (ValueError, RecursionError, UnicodeError):
            raise ValueError("invalid request JSON") from None

        kind = message.get("kind")
        seq = message.get("seq", -1)

        if kind == "shutdown":
            break
        if type(seq) is not int or not 1 <= seq <= 18446744073709551615:
            raise ValueError("invalid request sequence")
        if kind == "ping":
            _send({"kind": "response", "seq": seq, "payload": {"pong": True}})
            continue
        if kind == "request":
            try:
                result = handler(message.get("payload"), config)
                _send({"kind": "response", "seq": seq, "payload": result})
            except Exception as exc:  # noqa: BLE001 - report any handler error
                _send({"kind": "error", "seq": seq, "message": str(exc)[:128]})
            continue

        _send({"kind": "error", "seq": seq, "message": f"unknown kind: {kind}"})
