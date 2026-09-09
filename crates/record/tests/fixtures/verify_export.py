"""Pinned official Python MCAP 1.3.1 linear reader verifies Neuradix exports.

Linux memory budgets declared before measurement: each 8/128 MiB workload <=64
MiB child peak RSS; growth <=12 MiB. Producer timeout 20s, whole CI step 90s.
"""
import importlib.metadata
import json
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import zlib

from mcap.stream_reader import StreamReader
from mcap.records import Header, Schema, Channel, Message, Metadata, Statistics, Footer, DataEnd

assert importlib.metadata.version("mcap") == "1.3.1"
assert sys.platform == "linux", "Linux RSS evidence required; unavailable on this OS"
binary = sys.argv[1]
with tempfile.TemporaryDirectory() as directory:
    path = Path(directory) / "export.mcap"
    subprocess.run([binary, "fixture", str(path)], check=True, timeout=20)
    with path.open("rb") as stream:
        records = list(StreamReader(stream, validate_crcs=True).records)
    header = [r for r in records if isinstance(r, Header)]
    assert len(header) == 1
    assert (header[0].profile, header[0].library) == ("neuradix-export-evidence", "neuradix-record/0.0.1")
    schemas = [r for r in records if isinstance(r, Schema)]
    assert [(s.id, s.name, s.encoding, s.data) for s in schemas] == [(42, "example/Bytes", "custom/schema", b"\0schema\xff")]
    channels = [r for r in records if isinstance(r, Channel)]
    assert [(c.id, c.schema_id, c.topic, c.message_encoding, c.metadata) for c in channels] == [
        (7, 42, "sensor/bytes", "opaque/custom", {"clock.source": "device-boot", "clock.log": "host-boot"}),
        (0, 0, "raw", "", {"note": "no inferred epoch"})]
    messages = [r for r in records if isinstance(r, Message)]
    assert [(m.channel_id, m.sequence, m.log_time, m.publish_time, m.data) for m in messages] == [
        (7, 0, 1, 0, b"\0\x01\xff"), (0, 5, 9, 7, b""), (7, 2**32-1, 2**64-1, 2**64-2, b"hello")]
    metadata = [r for r in records if isinstance(r, Metadata)]
    assert [(m.name, m.metadata) for m in metadata] == [("provenance", {"producer": "neuradix", "time": "raw ns"})]
    stats = [r for r in records if isinstance(r, Statistics)]
    assert len(stats) == 1
    s = stats[0]
    assert (s.message_count, s.schema_count, s.channel_count, s.metadata_count, s.attachment_count, s.chunk_count) == (3, 1, 2, 1, 0, 0)
    assert (s.message_start_time, s.message_end_time, s.channel_message_counts) == (1, 2**64-1, {0: 1, 7: 2})
    assert len(records) == 11, [type(r).__name__ for r in records]
    # Validate structure/CRC independently of both Neuradix and the Rust vendor.
    data = path.read_bytes()
    pos, frames = 8, []
    while pos < len(data)-8:
        op, size = struct.unpack_from("<BQ", data, pos)
        frames.append((op, pos, pos+9+size))
        pos += 9+size
    assert [op for op, _, _ in frames] == [1, 3, 4, 4, 12, 5, 5, 5, 15, 11, 2]
    footer = next(r for r in records if isinstance(r, Footer))
    data_end = next(r for r in records if isinstance(r, DataEnd))
    assert footer.summary_start == frames[-2][1]
    assert footer.summary_offset_start == 0
    assert data_end.data_section_crc == zlib.crc32(data[:frames[-3][1]]) != 0
    assert footer.summary_crc == zlib.crc32(data[footer.summary_start:frames[-1][1]+25]) != 0
    assert data[:8] == data[-8:] == b"\x89MCAP0\r\n"
    print("INDEPENDENT_EXPORT_EVIDENCE=mcap-python-1.3.1 exact semantic/structure/CRC agreement")
    results = []
    for mib in [8, 128]:
        rss = Path(directory) / "rss.txt"
        run = subprocess.run(["/usr/bin/time", "-f", "%M", "-o", str(rss), binary, str(mib), str(path)],
                             check=True, timeout=20, capture_output=True, text=True)
        peak = int(rss.read_text())
        assert peak <= 64*1024, (mib, peak)
        assert path.stat().st_size > mib*1024*1024
        print(run.stdout.strip())
        results.append({"payload_mib": mib, "messages": mib*16, "peak_rss_kib": peak, "budget_mib": 64})
    assert results[1]["peak_rss_kib"] <= results[0]["peak_rss_kib"] + 12*1024, results
    print("MCAP_WRITER_MEMORY_EVIDENCE=" + json.dumps(results, sort_keys=True))
