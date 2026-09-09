# Independent MCAP fixtures

`generate.py` uses the official Python `mcap.writer.Writer`, with versions pinned
in `requirements.txt`: mcap 1.3.1, lz4 4.4.4 and zstandard 0.23.0. No Neuradix
writer is involved. Hex files encode exact binary MCAP bytes; `provenance.json`
records their SHA-256 hashes. Regeneration is checked byte-for-byte in CI.

```sh
python3 -m venv /tmp/a06-fixtures
/tmp/a06-fixtures/bin/pip install -r crates/record/tests/fixtures/requirements.txt
/tmp/a06-fixtures/bin/python crates/record/tests/fixtures/generate.py --check --memory /tmp/a06-memory
```

Three supported files carry identical semantic data, with uncompressed records,
uncompressed chunks or LZ4 chunks. They contain a complete custom schema, two
channels (one schemaless), opaque encodings/payloads, channel and named metadata,
three messages including empty data, duplicate zero sequences, `u32::MAX` and
`u64::MAX`, unequal logging/publishing times, data/chunk/summary CRCs and indexes.
Zstandard and attachment fixtures demonstrate explicit unsupported results.

The generator is the expected-data definition; Rust tests independently assert
all message fields and the schema/metadata contents. Header declarations and
clock labels are retained producer claims, not trusted clock synchronization.
This is container interoperability evidence, not decoded ROS interoperability.

The optional memory files carry 8 or 128 MiB of 64 KiB messages in roughly 1 MiB
LZ4 chunks, without growing indexes. `memory.py` measures the importing child
using Linux `/usr/bin/time`, excluding fixture generation. Each child has a
20-second timeout; the CI wrapper adds an outer timeout. Budgets: 64 MiB peak RSS
for streaming/rejection and 192 MiB for 128 MiB materialization; streaming growth
between workloads must be at most 12 MiB. See the
[A06 evidence](../../../../docs/implementation/WP-A06-Bounded-MCAP-Import.md)
for actual measurements and their limitations.

Primary producer documentation:
[Python writer API](https://mcap.dev/docs/python/mcap-apidoc/mcap.writer.html),
[Python package release](https://pypi.org/project/mcap/1.3.1/).
