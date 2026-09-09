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

## Independent export verification

`mcap_export` produces uncompressed files through the real Neuradix writer and a
File sink. `verify_export.py` uses the separately maintained official Python
`mcap.stream_reader.StreamReader` 1.3.1 to assert every schema, channel, encoding,
metadata entry, payload, sequence and both raw timestamps. It independently checks
record order, footer offsets and data/summary CRC ranges with Python struct/zlib.
It does not use Neuradix import or a Rust writer/reader round-trip for agreement.
The reader is the exact version in the existing requirements; dependencies and
lockfile need no additions. The Rust writer is mcap 0.25.0 (default features off).

```sh
cargo build --locked -p neuradix-record --example mcap_export
timeout --signal=TERM --kill-after=5s 90s /tmp/a06-fixtures/bin/python crates/record/tests/fixtures/verify_export.py target/debug/examples/mcap_export
```

Before measurement, writer qualification declares Linux child RSS <=64 MiB for
8/128 MiB payload workloads, <=12 MiB growth, 64 KiB borrowed payloads, fixed
schema/channel definitions, no growing sink buffer and 20-second child deadlines.
The wrapper adds a 90-second deadline. See [writer evidence](../../../../docs/implementation/WP-A06-Bounded-MCAP-Writer.md)
for results and limitations. Historical-layout Rust tests are compatibility
reconstructions, not independent producer evidence.

Primary independent reader documentation:
[Python StreamReader API](https://mcap.dev/docs/python/mcap-apidoc/mcap.stream_reader.html).
