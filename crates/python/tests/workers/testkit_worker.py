"""Test worker for neuradix-python integration tests.

Echoes payloads, hard-crashes on ``{"crash": true}`` and sleeps on ``{"sleep": s}``.
"""

import os
import time

import neuradix_worker


def handle(payload, config):
    payload = payload or {}
    if payload.get("crash"):
        # Hard, uncatchable crash (no cleanup) to exercise crash isolation.
        os._exit(13)
    if "sleep" in payload:
        time.sleep(float(payload["sleep"]))
    return {"echo": payload, "config": config}


neuradix_worker.run(handle, name="testkit-worker", skip_policy="may-skip")
