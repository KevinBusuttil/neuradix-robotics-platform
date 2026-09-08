//! Versioned binding of full command metadata to the existing serial frame.
//!
//! The outer u16 frame sequence is transport diagnostics only. The inner u64
//! command sequence, generation and original source time are checked by the gate.
//! CRC and these identifiers do not authenticate the source.
use neuradix_embedded_core::{Command, CommandMeta, Generation};
use neuradix_time::{ClockDomain, Timestamp};

/// A04.2 command payload version (legacy four-byte scalar payloads are rejected).
pub const COMMAND_VERSION: u8 = 1;
/// Fixed payload size in bytes, excluding outer frame overhead.
pub const COMMAND_BYTES: usize = 87;

/// Encode without changing or synthesizing command validity metadata.
pub fn encode_command(command: Command) -> [u8; COMMAND_BYTES] {
    let mut out = [0; COMMAND_BYTES];
    out[0] = COMMAND_VERSION;
    out[1..9].copy_from_slice(&command.holder.to_le_bytes());
    out[9..17].copy_from_slice(&command.capability.to_le_bytes());
    out[17..33].copy_from_slice(&command.meta.generation.get().to_le_bytes());
    out[33..41].copy_from_slice(&command.meta.sequence.to_le_bytes());
    out[41..49].copy_from_slice(&command.meta.timeline.to_le_bytes());
    out[49] = command.meta.source_at.domain().code();
    out[50..66].copy_from_slice(&command.meta.source_at.as_nanos().to_le_bytes());
    out[66] = command.meta.deadline.domain().code();
    out[67..83].copy_from_slice(&command.meta.deadline.as_nanos().to_le_bytes());
    out[83..87].copy_from_slice(&command.value.to_le_bytes());
    out
}

/// Decode exact version/size/representation. Source validity and non-finite
/// scalar values remain the gate's responsibility. Arrival time is not an input.
pub fn decode_command(bytes: &[u8]) -> Option<Command> {
    if bytes.len() != COMMAND_BYTES || bytes[0] != COMMAND_VERSION {
        return None;
    }
    Some(Command {
        holder: u64::from_le_bytes(bytes[1..9].try_into().ok()?),
        capability: u64::from_le_bytes(bytes[9..17].try_into().ok()?),
        value: f32::from_le_bytes(bytes[83..87].try_into().ok()?),
        meta: CommandMeta {
            generation: Generation::new(u128::from_le_bytes(bytes[17..33].try_into().ok()?))?,
            sequence: u64::from_le_bytes(bytes[33..41].try_into().ok()?),
            timeline: u64::from_le_bytes(bytes[41..49].try_into().ok()?),
            source_at: Timestamp::new(
                ClockDomain::from_code(bytes[49])?,
                i128::from_le_bytes(bytes[50..66].try_into().ok()?),
            ),
            deadline: Timestamp::new(
                ClockDomain::from_code(bytes[66])?,
                i128::from_le_bytes(bytes[67..83].try_into().ok()?),
            ),
        },
    })
}
