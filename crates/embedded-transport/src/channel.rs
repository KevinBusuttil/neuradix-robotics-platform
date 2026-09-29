//! Compact channel envelope: binds each frame payload to a verified manifest entry.
//!
//! The envelope sits inside an ordinary frame payload; the frame format and CRC
//! are unchanged:
//!
//! ```text
//! 'N' (0x4E) | version:u8 | compact_id:u16 LE | manifest_tag:[u8; 4] | body[wire_len]
//! ```
//!
//! A receiver's [`ChannelTable`] is built from the host's verified channel
//! manifest (`neuradix_contracts::ChannelManifest`). [`ChannelTable::open`]
//! rejects unknown or reserved compact IDs, a body whose length differs from
//! the bound entry, and a manifest tag (the first four bytes of the manifest
//! digest) that differs from the table's. The resolved
//! [`ChannelBinding::wire_id`] is the peer wire ID passed to the generated
//! `decode(body, peer_wire_id)`, which rejects a layout it was not built for.
//!
//! The manifest tag detects endpoints provisioned from different manifests; it
//! is not an identity, a session or authentication. A whole link is configured
//! for envelopes: the magic and version cannot prove that an unenveloped legacy
//! payload is not one. Exchanging the full manifest digest at link start is
//! session work (WP-B06).

/// Envelope magic byte.
pub const ENVELOPE_MAGIC: u8 = 0x4E;
/// Envelope format version.
pub const ENVELOPE_VERSION: u8 = 1;
/// Envelope header length in bytes.
pub const ENVELOPE_HEADER: usize = 8;
/// Compact ID 0 is reserved and never bound.
pub const RESERVED_COMPACT_ID: u16 = 0;
/// Length of a full `sha256:<64 hex>` wire ID.
pub const WIRE_ID_LEN: usize = 71;

/// One verified compact channel binding (a manifest entry).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelBinding {
    /// Compact ID carried on the wire.
    pub compact_id: u16,
    /// Producer's full wire ID from the manifest (never the receiver's own constant).
    pub wire_id: &'static str,
    /// Exact body length.
    pub wire_len: u16,
}

/// Why a [`ChannelTable`] could not be built. No table exists after an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindError {
    /// A binding uses compact ID 0.
    ReservedId,
    /// Two bindings share a compact ID.
    Collision {
        /// Colliding compact ID.
        compact_id: u16,
    },
    /// A wire ID is not `sha256:` plus 64 lowercase hex digits.
    MalformedWireId {
        /// Offending compact ID.
        compact_id: u16,
    },
    /// Zero or oversized body length.
    InvalidWireLen {
        /// Offending compact ID.
        compact_id: u16,
    },
    /// More bindings than the table capacity.
    TableFull,
    /// No bindings.
    Empty,
}

/// Why an envelope was rejected or could not be written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvelopeError {
    /// Shorter than the envelope header.
    Truncated,
    /// Wrong magic byte (for example, an unenveloped payload).
    BadMagic,
    /// Unknown envelope version.
    UnsupportedVersion(u8),
    /// The sender's manifest tag differs from this table's manifest.
    ManifestMismatch,
    /// The compact ID is reserved or not bound in this table.
    UnknownChannel(u16),
    /// Body length differs from the bound wire length.
    LengthMismatch {
        /// Channel.
        compact_id: u16,
        /// Bound length.
        expected: u16,
        /// Received length.
        actual: usize,
    },
    /// The output buffer is too small.
    BufferTooSmall,
}

impl core::fmt::Display for BindError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl core::error::Error for BindError {}
impl core::fmt::Display for EnvelopeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl core::error::Error for EnvelopeError {}

/// A received body resolved to its channel binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Envelope<'a> {
    /// The binding the body was resolved to.
    pub binding: ChannelBinding,
    /// Exactly `binding.wire_len` bytes.
    pub body: &'a [u8],
}

/// A fixed-capacity, collision-free compact channel table for one manifest.
#[derive(Debug, Clone)]
pub struct ChannelTable<const N: usize> {
    bindings: [Option<ChannelBinding>; N],
    len: usize,
    manifest_digest: [u8; 32],
}

impl<const N: usize> ChannelTable<N> {
    /// Build a table from a verified manifest's digest and bindings. Any
    /// reserved ID, collision, malformed wire ID or bad length rejects the whole
    /// table, so a gateway never runs with a partial or ambiguous mapping.
    pub fn new(manifest_digest: [u8; 32], bindings: &[ChannelBinding]) -> Result<Self, BindError> {
        if bindings.is_empty() {
            return Err(BindError::Empty);
        }
        if bindings.len() > N {
            return Err(BindError::TableFull);
        }
        let mut table = Self {
            bindings: [None; N],
            len: 0,
            manifest_digest,
        };
        for b in bindings {
            if b.compact_id == RESERVED_COMPACT_ID {
                return Err(BindError::ReservedId);
            }
            if !is_wire_id(b.wire_id) {
                return Err(BindError::MalformedWireId {
                    compact_id: b.compact_id,
                });
            }
            if b.wire_len == 0 || usize::from(b.wire_len) > usize::from(u16::MAX) - ENVELOPE_HEADER
            {
                return Err(BindError::InvalidWireLen {
                    compact_id: b.compact_id,
                });
            }
            if table.find(b.compact_id).is_some() {
                return Err(BindError::Collision {
                    compact_id: b.compact_id,
                });
            }
            table.bindings[table.len] = Some(*b);
            table.len += 1;
        }
        Ok(table)
    }

    /// Number of bindings.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Always false for a constructed table.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Full digest of the manifest this table was provisioned from, for
    /// reporting alongside firmware identity.
    pub fn manifest_digest(&self) -> &[u8; 32] {
        &self.manifest_digest
    }

    /// The per-envelope manifest tag: the digest's first four bytes.
    pub fn manifest_tag(&self) -> [u8; 4] {
        [
            self.manifest_digest[0],
            self.manifest_digest[1],
            self.manifest_digest[2],
            self.manifest_digest[3],
        ]
    }

    /// The binding for `compact_id`, if bound.
    pub fn resolve(&self, compact_id: u16) -> Option<&ChannelBinding> {
        self.find(compact_id)
    }

    fn find(&self, compact_id: u16) -> Option<&ChannelBinding> {
        self.bindings[..self.len]
            .iter()
            .flatten()
            .find(|b| b.compact_id == compact_id)
    }

    /// Write an envelope for `body` on `compact_id` into `out`, returning its
    /// length. The body must match the bound length exactly.
    pub fn seal(
        &self,
        compact_id: u16,
        body: &[u8],
        out: &mut [u8],
    ) -> Result<usize, EnvelopeError> {
        let b = self
            .find(compact_id)
            .ok_or(EnvelopeError::UnknownChannel(compact_id))?;
        if body.len() != usize::from(b.wire_len) {
            return Err(EnvelopeError::LengthMismatch {
                compact_id,
                expected: b.wire_len,
                actual: body.len(),
            });
        }
        let total = ENVELOPE_HEADER + body.len();
        if out.len() < total {
            return Err(EnvelopeError::BufferTooSmall);
        }
        out[0] = ENVELOPE_MAGIC;
        out[1] = ENVELOPE_VERSION;
        out[2..4].copy_from_slice(&compact_id.to_le_bytes());
        out[4..8].copy_from_slice(&self.manifest_tag());
        out[ENVELOPE_HEADER..total].copy_from_slice(body);
        Ok(total)
    }

    /// Resolve a received frame payload to its binding and exact body.
    pub fn open<'a>(&self, payload: &'a [u8]) -> Result<Envelope<'a>, EnvelopeError> {
        if payload.len() < ENVELOPE_HEADER {
            return Err(EnvelopeError::Truncated);
        }
        if payload[0] != ENVELOPE_MAGIC {
            return Err(EnvelopeError::BadMagic);
        }
        if payload[1] != ENVELOPE_VERSION {
            return Err(EnvelopeError::UnsupportedVersion(payload[1]));
        }
        if payload[4..8] != self.manifest_tag() {
            return Err(EnvelopeError::ManifestMismatch);
        }
        let compact_id = u16::from_le_bytes([payload[2], payload[3]]);
        let binding = *self
            .find(compact_id)
            .ok_or(EnvelopeError::UnknownChannel(compact_id))?;
        let body = &payload[ENVELOPE_HEADER..];
        if body.len() != usize::from(binding.wire_len) {
            return Err(EnvelopeError::LengthMismatch {
                compact_id,
                expected: binding.wire_len,
                actual: body.len(),
            });
        }
        Ok(Envelope { binding, body })
    }
}

/// `sha256:` followed by exactly 64 lowercase hex digits.
fn is_wire_id(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == WIRE_ID_LEN
        && b.starts_with(b"sha256:")
        && b[7..]
            .iter()
            .all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
}
