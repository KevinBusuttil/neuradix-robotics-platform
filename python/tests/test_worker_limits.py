"""SDK byte limits are independently checked; no subprocess or optional skips."""
import io
import json
import os
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import neuradix_worker as worker


class Stream:
    def __init__(self, data=b""):
        self.buffer = io.BytesIO(data)


class WorkerLimits(unittest.TestCase):
    def test_exact_output_and_oversize_emit_no_partial_line(self):
        output = Stream()
        with patch.dict(os.environ, {"NEURADIX_WORKER_MAX_OUTPUT_BYTES": "64"}), patch.object(sys, "stdout", output):
            worker._send({"s": "x" * 55})
            self.assertEqual(len(output.buffer.getvalue()), 64)
            output.buffer.seek(0)
            output.buffer.truncate()
            with self.assertRaises(ValueError):
                worker._send({"s": "x" * 56})
            self.assertEqual(output.buffer.getvalue(), b"")

    def test_utf8_nonfinite_and_depth_boundaries(self):
        output = Stream()
        with patch.dict(os.environ, {"NEURADIX_WORKER_MAX_OUTPUT_BYTES": "64"}), patch.object(sys, "stdout", output):
            worker._send({"s": "☃" * 18})
            self.assertEqual(len(output.buffer.getvalue()), 63)
            with self.assertRaises(ValueError):
                worker._send({"s": "☃" * 19})
            with self.assertRaises(ValueError):
                worker._send({"value": float("nan")})
            value = None
            for _ in range(33):
                value = [value]
            with self.assertRaises(ValueError):
                worker._send(value)

    def test_input_limit_includes_newline_and_precedes_handler(self):
        base = json.dumps({"kind": "request", "seq": 1, "payload": None}, separators=(",", ":")).encode()
        for count in (64, 65):
            line = base + b" " * (count - len(base) - 1) + b"\n"
            calls = []
            output = Stream()
            settings = {"NEURADIX_WORKER_MAX_INPUT_BYTES": "64", "NEURADIX_WORKER_MAX_OUTPUT_BYTES": "256", "NEURADIX_WORKER_CONFIG": "null"}
            with patch.dict(os.environ, settings), patch.object(sys, "stdin", Stream(line)), patch.object(sys, "stdout", output):
                if count == 64:
                    worker.run(lambda payload, config: calls.append(payload), name="test")
                    self.assertEqual(calls, [None])
                else:
                    with self.assertRaises(ValueError):
                        worker.run(lambda payload, config: calls.append(payload), name="test")
                    self.assertEqual(calls, [])

    def test_invalid_startup_configuration_is_not_silently_replaced(self):
        with patch.dict(os.environ, {"NEURADIX_WORKER_CONFIG": "invalid JSON"}):
            with self.assertRaises(ValueError):
                worker.run(lambda payload, config: None)


if __name__ == "__main__":
    unittest.main()
