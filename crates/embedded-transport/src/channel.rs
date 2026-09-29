//! Compact channel envelope: binds each frame payload to a verified manifest entry.
//!
//! The envelope sits inside an ordinary frame payload; the frame format and CRC
//! are unchanged:
//!
//! ```text
//! 'N' (0x4E) | version:u8 | compact_id:u16 LE | manifest_tag:[u8; 4] | body[wire_len]
//! ```
//!
//! A [`ChannelTable`] is built from the host's verified channel manifest
//! (`neuradix_contracts::ChannelManifest`, format
//! [`CHANNEL_MANIFEST_VERSION`]). Its constructor recomputes the manifest digest
//! from the supplied bindings, allocation-free, and refuses a digest they do not
//! produce, so a digest from one manifest can never front bindings from
//! another. [`ChannelTable::open`]
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

use sha2::{Digest, Sha256};

/// Envelope magic byte.
pub const ENVELOPE_MAGIC: u8 = 0x4E;
/// Envelope format version.
pub const ENVELOPE_VERSION: u8 = 1;
/// Envelope header length in bytes.
pub const ENVELOPE_HEADER: usize = 8;
/// Compact ID 0 is reserved and never bound.
pub const RESERVED_COMPACT_ID: u16 = 0;
/// Length of a full `sha256:<64 hex>` wire or schema ID.
pub const WIRE_ID_LEN: usize = 71;
/// Channel manifest format whose digest preimage [`ChannelTable::new`] recomputes.
pub const CHANNEL_MANIFEST_VERSION: &str = "neuradix.channel-manifest.v2";
/// The only payload codec a channel may bind.
pub const CODEC_ID: &str = "neuradix.scalar-le.v2";
/// Longest channel name, in bytes.
pub const MAX_CHANNEL_NAME_LEN: usize = 128;

/// One compact channel binding: a complete manifest entry, normally emitted by
/// `neuradix_embedded_codegen::generate_channel_table`. Every field is covered
/// by the manifest digest the table verifies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelBinding {
    /// Compact ID carried on the wire.
    pub compact_id: u16,
    /// Exact body length.
    pub wire_len: u16,
    /// Channel name from the manifest.
    pub name: &'static str,
    /// Payload codec; must be [`CODEC_ID`].
    pub codec_id: &'static str,
    /// Semantic schema identity (hashed, never used to authorize decoding).
    pub schema_id: &'static str,
    /// Producer's full wire ID from the manifest (never the receiver's own constant).
    pub wire_id: &'static str,
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
    /// Bindings are not in strictly ascending compact-ID order (the manifest's
    /// canonical order, which the digest covers).
    Unordered {
        /// First out-of-order compact ID.
        compact_id: u16,
    },
    /// A codec other than [`CODEC_ID`].
    UnsupportedCodec {
        /// Offending compact ID.
        compact_id: u16,
    },
    /// Empty, overlong, control-character or duplicate name.
    InvalidName {
        /// Offending compact ID.
        compact_id: u16,
    },
    /// A schema ID is not `sha256:` plus 64 lowercase hex digits.
    MalformedSchemaId {
        /// Offending compact ID.
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
    /// The supplied digest is not the digest of these exact bindings: they come
    /// from a different manifest, or were edited after generation.
    DigestMismatch,
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
    /// Build a table from one manifest's digest and its complete bindings, in
    /// ascending compact-ID order. Each binding is checked (reserved ID,
    /// collision, order, codec, name, identity format, length), then the
    /// manifest digest is recomputed from the bindings and must equal
    /// `manifest_digest`. Any failure rejects the whole table, so a gateway
    /// never runs with a partial, ambiguous or mismatched mapping.
    ///
    /// This proves only that the pair is self-consistent. It does not prove
    /// which manifest a peer holds; the per-envelope tag detects that, weakly.
    pub fn new(manifest_digest: [u8; 32], bindings: &[ChannelBinding]) -> Result<Self, BindError> {
        if bindings.is_empty() {
            return Err(BindError::Empty);
        }
        if bindings.len() > N {
            return Err(BindError::TableFull);
        }
        for (i, b) in bindings.iter().enumerate() {
            let compact_id = b.compact_id;
            if compact_id == RESERVED_COMPACT_ID {
                return Err(BindError::ReservedId);
            }
            if let Some(prev) = i.checked_sub(1).map(|p| bindings[p].compact_id) {
                if prev == compact_id {
                    return Err(BindError::Collision { compact_id });
                }
                if prev > compact_id {
                    return Err(BindError::Unordered { compact_id });
                }
            }
            if b.codec_id != CODEC_ID {
                return Err(BindError::UnsupportedCodec { compact_id });
            }
            if !valid_name(b.name) || bindings[..i].iter().any(|p| p.name == b.name) {
                return Err(BindError::InvalidName { compact_id });
            }
            if !is_sha256_id(b.schema_id) {
                return Err(BindError::MalformedSchemaId { compact_id });
            }
            if !is_sha256_id(b.wire_id) {
                return Err(BindError::MalformedWireId { compact_id });
            }
            if b.wire_len == 0 || usize::from(b.wire_len) > usize::from(u16::MAX) - ENVELOPE_HEADER
            {
                return Err(BindError::InvalidWireLen { compact_id });
            }
        }
        if self::manifest_digest(bindings) != manifest_digest {
            return Err(BindError::DigestMismatch);
        }
        let mut table = Self {
            bindings: [None; N],
            len: bindings.len(),
            manifest_digest,
        };
        for (slot, b) in table.bindings.iter_mut().zip(bindings) {
            *slot = Some(*b);
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

/// SHA-256 of the channel manifest digest preimage for `bindings`, in the
/// given order (see `neuradix_contracts::ChannelManifest::digest_preimage`),
/// streamed without allocation. [`ChannelTable::new`] validates bindings before
/// calling this; for unvalidated input, string and count lengths are truncated
/// to `u16` and the result matches no host manifest.
pub fn manifest_digest(bindings: &[ChannelBinding]) -> [u8; 32] {
    fn put(h: &mut Sha256, s: &str) {
        h.update((s.len() as u16).to_le_bytes());
        h.update(s.as_bytes());
    }
    let mut h = Sha256::new();
    put(&mut h, CHANNEL_MANIFEST_VERSION);
    h.update((bindings.len() as u16).to_le_bytes());
    for b in bindings {
        h.update(b.compact_id.to_le_bytes());
        h.update(b.wire_len.to_le_bytes());
        put(&mut h, b.name);
        put(&mut h, b.codec_id);
        put(&mut h, b.schema_id);
        put(&mut h, b.wire_id);
    }
    h.finalize().into()
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= MAX_CHANNEL_NAME_LEN && !name.chars().any(char::is_control)
}

/// `sha256:` followed by exactly 64 lowercase hex digits.
fn is_sha256_id(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == WIRE_ID_LEN
        && b.starts_with(b"sha256:")
        && b[7..]
            .iter()
            .all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
}
