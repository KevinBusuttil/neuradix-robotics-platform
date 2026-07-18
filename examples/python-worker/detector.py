"""A tiny Python component: flag depth samples below a configured threshold.

Demonstrates that an ordinary Python handler participates in the platform while
running as an isolated process. Sending ``{"crash": true}`` hard-crashes the
worker so the Rust side can show crash isolation and FDIR safing.
"""

import os

import neuradix_worker


def on_depth(payload, config):
    payload = payload or {}
    if payload.get("crash"):
        os._exit(7)
    depth = float(payload["depth"])
    threshold = float((config or {}).get("threshold", 12.0))
    return {"depth": depth, "belowThreshold": depth > threshold}


neuradix_worker.run(on_depth, name="depth-detector", skip_policy="may-skip")
