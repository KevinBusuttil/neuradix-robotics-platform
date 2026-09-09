"""Measure importer child RSS on Linux, excluding independent fixture generation."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile

binary, directory = sys.argv[1:]
assert sys.platform == "linux", "RSS qualification requires Linux; unsupported here"
results = []
with tempfile.TemporaryDirectory() as scratch:
    for mode, mib, budget in [("stream", 8, 64), ("stream", 128, 64),
                              ("archive", 128, 192), ("bounded-reject", 128, 64)]:
        rss = Path(scratch) / "rss.txt"
        result = subprocess.run(["/usr/bin/time", "-f", "%M", "-o", str(rss),
                                 binary, mode, str(Path(directory) / f"{mib}mib.mcap")],
                                timeout=20, check=True, capture_output=True, text=True)
        peak = int(rss.read_text())
        assert peak <= budget * 1024, (mode, mib, peak, budget)
        print(result.stdout.strip())
        results.append({"mode": mode, "payload_mib": mib, "peak_rss_kib": peak,
                        "budget_mib": budget})
assert results[1]["peak_rss_kib"] <= results[0]["peak_rss_kib"] + 12 * 1024, results
print("MCAP_MEMORY_EVIDENCE=" + json.dumps(results, sort_keys=True))
