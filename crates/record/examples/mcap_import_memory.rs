//! Regular-file import workload for externally measured Linux peak RSS.
use neuradix_record::{McapArchive, McapImportLimits, import_mcap};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let input = std::fs::File::open(&args[2])?;
    let limits = McapImportLimits::default()
        .with_input_bytes(256 << 20)?
        .with_record_bytes(2 << 20)?
        .with_chunk_bytes(2 << 20)?
        .with_decoded_bytes(256 << 20)?
        .with_message_bytes(64 << 10)?
        .with_state_bytes(1 << 20)?
        .with_retained_bytes(256 << 20)?;
    if args[1] == "stream" {
        let summary = import_mcap(input, limits, |_| Ok(()))?;
        println!("stream {:?}", summary.stats());
    } else if args[1] == "bounded-reject" {
        let error = McapArchive::from_reader(input, limits.with_retained_bytes(8 << 20)?)
            .expect_err("materialization must reject at its budget");
        assert!(matches!(error, neuradix_record::RecordError::ImportLimit { kind: "retained bytes", .. }));
        println!("bounded-reject: {error}");
    } else {
        let archive = McapArchive::from_reader(input, limits)?;
        println!("archive {:?}, retained={}", archive.summary().stats(), archive.retained_bytes());
    }
    Ok(())
}
