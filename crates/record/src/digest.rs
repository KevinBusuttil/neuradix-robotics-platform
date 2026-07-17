//! The replay digest: a deterministic content hash over a recording's records.
//!
//! Two replays of the same recording produce the same digest, and any change to
//! the ordered records changes it. This is the basis of replay-equivalence
//! checks (CLI exit code 9 on mismatch).

use sha2::{Digest, Sha256};

use crate::native::NativeRecording;

/// Compute the `sha256:<hex>` replay digest of a recording.
///
/// The digest covers, for every record in order: channel id, sequence, clock
/// domain, timestamp and payload bytes. It deliberately excludes the manifest so
/// that provenance notes do not perturb the behavioural digest.
pub fn replay_digest(recording: &NativeRecording) -> String {
    let mut hasher = Sha256::new();
    for record in recording.records() {
        hasher.update(record.channel_id.to_le_bytes());
        hasher.update(record.sequence.to_le_bytes());
        hasher.update([record.timestamp.domain().code()]);
        hasher.update(record.timestamp.as_nanos().to_le_bytes());
        hasher.update((record.payload.len() as u64).to_le_bytes());
        hasher.update(&record.payload);
    }
    format!("sha256:{}", to_hex(&hasher.finalize()))
}

fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}
