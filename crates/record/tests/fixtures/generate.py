"""Independent fixtures: official Python MCAP writer, never Neuradix code.

Run with the adjacent pinned requirements. --check compares committed hex
byte-for-byte; --memory DIR also creates regular-file workloads for RSS tests.
"""
import argparse
import hashlib
import importlib.metadata
import io
import json
from pathlib import Path

from mcap.writer import CompressionType, IndexType, Writer


def fixture(chunked, compression, attachment=False):
    output = io.BytesIO()
    writer = Writer(output, use_chunking=chunked, compression=compression,
                    chunk_size=256, enable_data_crcs=True)
    writer.start(profile="independent-fixture")
    schema = writer.register_schema("example/Bytes", "custom/schema", b"\x00schema\xff")
    first = writer.register_channel("sensor/bytes", "opaque/custom", schema,
                                    {"clock.source": "device-boot", "clock.log": "host-boot"})
    second = writer.register_channel("raw", "", 0, {"note": "no inferred epoch"})
    for channel, sequence, log, publish, data in [
        (first, 0, 1, 0, b"\x00\x01\xff"),
        (second, 0, 9, 7, b""),
        (first, 4294967295, 18446744073709551615, 18446744073709551614, b"hello")]:
        writer.add_message(channel, log, data, publish, sequence)
    writer.add_metadata("provenance", {"fixture": "python-mcap", "time": "raw ns"})
    if attachment:
        writer.add_attachment(1, 2, "unsupported", "text/plain", b"attachment")
    writer.finish()
    return output.getvalue()


def memory_fixture(path, mib):
    # Bounded producer buffering too; no full workload retained in Python.
    with path.open("wb") as output:
        writer = Writer(output, compression=CompressionType.LZ4, chunk_size=1 << 20,
                        index_types=IndexType.NONE, repeat_channels=False,
                        repeat_schemas=False, use_summary_offsets=False,
                        enable_data_crcs=True)
        writer.start(profile="rss-workload")
        channel = writer.register_channel("memory", "opaque", 0)
        payload = bytes(range(256)) * 256
        for i in range(mib * 16):
            writer.add_message(channel, i, payload, i, i)
        writer.finish()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--memory", type=Path)
    args = parser.parse_args()
    versions = {name: importlib.metadata.version(name) for name in ("mcap", "lz4", "zstandard")}
    assert versions == {"mcap": "1.3.1", "lz4": "4.4.4", "zstandard": "0.23.0"}, versions
    outputs = {}
    for name, chunked, compression, attachment in [
        ("uncompressed", False, CompressionType.NONE, False),
        ("chunked", True, CompressionType.NONE, False),
        ("lz4", True, CompressionType.LZ4, False),
        ("zstd-unsupported", True, CompressionType.ZSTD, False),
        ("attachment-unsupported", False, CompressionType.NONE, True)]:
        data = fixture(chunked, compression, attachment)
        outputs[name + ".mcap.hex"] = data.hex() + "\n"
    provenance = {"producer": "official Python mcap.writer.Writer", "versions": versions,
                  "sha256": {name: hashlib.sha256(bytes.fromhex(value)).hexdigest()
                             for name, value in outputs.items()}}
    outputs["provenance.json"] = json.dumps(provenance, indent=2, sort_keys=True) + "\n"
    root = Path(__file__).parent
    for name, text in outputs.items():
        if args.check:
            assert (root / name).read_text() == text, name
        else:
            (root / name).write_text(text)
    print(json.dumps(provenance, sort_keys=True))
    if args.memory:
        args.memory.mkdir(parents=True, exist_ok=True)
        for size in (8, 128):
            memory_fixture(args.memory / f"{size}mib.mcap", size)


if __name__ == "__main__":
    main()
