//! Verified channel manifest: compact channel IDs resolved to full wire identity.
//!
//! A compact ID is an explicit assignment, never a truncated hash. Each entry
//! binds one `u16` compact ID to the full codec, schema and wire identity of the
//! payload the producer sends on that channel. Verification rejects compact-ID
//! collisions, reserved IDs, unknown codecs, malformed identities and a digest
//! that does not match the entries. The digest names a manifest; it does not
//! authenticate a peer.

use crate::layout::{CODEC_ID, WireLayout};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Manifest format label; also the first hashed field.
pub const CHANNEL_MANIFEST_VERSION: &str = "neuradix.channel-manifest.v1";

/// Compact ID 0 is reserved (unassigned / future link control).
pub const RESERVED_COMPACT_ID: u16 = 0;

/// Largest payload a compact channel envelope can carry: a frame's `u16`
/// length less the 8-byte envelope header.
pub const MAX_CHANNEL_WIRE_LEN: usize = u16::MAX as usize - 8;

/// One compact channel binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelEntry {
    /// Explicitly assigned compact ID carried on the wire.
    pub compact_id: u16,
    /// Unique channel name within the manifest.
    pub name: String,
    /// Versioned scalar codec of the producer's payload.
    pub codec_id: String,
    /// Semantic schema identity (informational; never authorizes decoding).
    pub schema_id: String,
    /// Full wire identity the receiver passes to the generated decoder.
    pub wire_id: String,
    /// Exact payload length.
    pub wire_len: usize,
}

impl ChannelEntry {
    /// Bind `compact_id` and `name` to the producer's resolved layout.
    pub fn for_layout(compact_id: u16, name: impl Into<String>, layout: &WireLayout) -> Self {
        Self {
            compact_id,
            name: name.into(),
            codec_id: layout.codec_id.clone(),
            schema_id: layout.schema_id.clone(),
            wire_id: layout.wire_id.clone(),
            wire_len: layout.wire_len,
        }
    }

    fn matches(&self, layout: &WireLayout) -> bool {
        self.codec_id == layout.codec_id
            && self.schema_id == layout.schema_id
            && self.wire_id == layout.wire_id
            && self.wire_len == layout.wire_len
    }
}

/// A verified set of compact channel bindings, ordered by compact ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelManifest {
    channels: Vec<ChannelEntry>,
    digest: [u8; 32],
}

#[derive(Serialize)]
struct Hashed<'a> {
    manifest_version: &'a str,
    channels: &'a [ChannelEntry],
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    manifest_version: String,
    channels: Vec<ChannelEntry>,
    digest: String,
}

impl ChannelManifest {
    /// Verify `channels` and compute the digest. Input order does not matter.
    pub fn new(mut channels: Vec<ChannelEntry>) -> Result<Self, ChannelManifestError> {
        if channels.is_empty() {
            return Err(ChannelManifestError::Empty);
        }
        channels.sort_by_key(|c| c.compact_id);
        for pair in channels.windows(2) {
            if pair[0].compact_id == pair[1].compact_id {
                return Err(ChannelManifestError::CompactIdCollision {
                    compact_id: pair[0].compact_id,
                    first: pair[0].name.clone(),
                    second: pair[1].name.clone(),
                });
            }
        }
        let mut names: Vec<&str> = channels.iter().map(|c| c.name.as_str()).collect();
        names.sort_unstable();
        if let Some(pair) = names.windows(2).find(|p| p[0] == p[1]) {
            return Err(ChannelManifestError::DuplicateName(pair[0].to_owned()));
        }
        for c in &channels {
            check_entry(c)?;
        }
        let digest = Sha256::digest(
            serde_json::to_vec(&Hashed {
                manifest_version: CHANNEL_MANIFEST_VERSION,
                channels: &channels,
            })
            .expect("channel manifest serialization cannot fail"),
        )
        .into();
        Ok(Self { channels, digest })
    }

    /// Parse and verify a manifest document, including its declared digest.
    pub fn parse(json: &str) -> Result<Self, ChannelManifestError> {
        let doc: Document =
            serde_json::from_str(json).map_err(|e| ChannelManifestError::Parse(e.to_string()))?;
        if doc.manifest_version != CHANNEL_MANIFEST_VERSION {
            return Err(ChannelManifestError::UnsupportedVersion(
                doc.manifest_version,
            ));
        }
        let manifest = Self::new(doc.channels)?;
        let computed = manifest.digest();
        if doc.digest != computed {
            return Err(ChannelManifestError::DigestMismatch {
                declared: doc.digest,
                computed,
            });
        }
        Ok(manifest)
    }

    /// Pretty JSON document with trailing newline; [`Self::parse`] accepts it.
    pub fn to_json_pretty(&self) -> String {
        let doc = Document {
            manifest_version: CHANNEL_MANIFEST_VERSION.to_owned(),
            channels: self.channels.clone(),
            digest: self.digest(),
        };
        serde_json::to_string_pretty(&doc).expect("channel manifest serialization cannot fail")
            + "\n"
    }

    /// `sha256:<hex>` over the compact JSON of the version and ordered entries.
    pub fn digest(&self) -> String {
        let mut s = String::from("sha256:");
        for b in self.digest {
            s.push_str(&format!("{b:02x}"));
        }
        s
    }

    /// Raw digest bytes, as provisioned into an embedded channel table.
    pub fn digest_bytes(&self) -> [u8; 32] {
        self.digest
    }

    /// Entries in compact-ID order.
    pub fn channels(&self) -> &[ChannelEntry] {
        &self.channels
    }

    /// The entry bound to `compact_id`, if any.
    pub fn resolve(&self, compact_id: u16) -> Option<&ChannelEntry> {
        self.channels
            .binary_search_by_key(&compact_id, |c| c.compact_id)
            .ok()
            .map(|i| &self.channels[i])
    }

    /// Resolve `compact_id` and require it to carry exactly `layout`, the
    /// layout a receiver's generated decoder was built from.
    pub fn verify_layout(
        &self,
        compact_id: u16,
        layout: &WireLayout,
    ) -> Result<&ChannelEntry, ChannelManifestError> {
        let entry = self
            .resolve(compact_id)
            .ok_or(ChannelManifestError::UnknownCompactId(compact_id))?;
        if !entry.matches(layout) {
            return Err(ChannelManifestError::LayoutMismatch {
                name: entry.name.clone(),
            });
        }
        Ok(entry)
    }
}

fn check_entry(c: &ChannelEntry) -> Result<(), ChannelManifestError> {
    if c.compact_id == RESERVED_COMPACT_ID {
        return Err(ChannelManifestError::ReservedCompactId {
            name: c.name.clone(),
        });
    }
    if c.name.is_empty() || c.name.len() > 128 || c.name.chars().any(char::is_control) {
        return Err(ChannelManifestError::InvalidName(c.name.clone()));
    }
    if c.codec_id != CODEC_ID {
        return Err(ChannelManifestError::UnsupportedCodec {
            name: c.name.clone(),
            codec_id: c.codec_id.clone(),
        });
    }
    if !is_sha256_id(&c.schema_id) || !is_sha256_id(&c.wire_id) {
        return Err(ChannelManifestError::MalformedIdentity {
            name: c.name.clone(),
        });
    }
    if c.wire_len == 0 || c.wire_len > MAX_CHANNEL_WIRE_LEN {
        return Err(ChannelManifestError::InvalidWireLen {
            name: c.name.clone(),
            wire_len: c.wire_len,
        });
    }
    Ok(())
}

/// `sha256:` followed by exactly 64 lowercase hex digits.
fn is_sha256_id(s: &str) -> bool {
    s.strip_prefix("sha256:")
        .is_some_and(|h| h.len() == 64 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

/// Why a channel manifest was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChannelManifestError {
    /// The document is not valid manifest JSON (or has unknown fields).
    #[error("channel manifest parse error: {0}")]
    Parse(String),
    /// Unknown manifest format.
    #[error("unsupported channel manifest version `{0}`")]
    UnsupportedVersion(String),
    /// No channels declared.
    #[error("channel manifest declares no channels")]
    Empty,
    /// Two entries share a compact ID.
    #[error("compact ID {compact_id} is bound to both `{first}` and `{second}`")]
    CompactIdCollision {
        /// Colliding compact ID.
        compact_id: u16,
        /// First channel name, in input order.
        first: String,
        /// Second channel name.
        second: String,
    },
    /// Two entries share a name.
    #[error("channel name `{0}` is declared more than once")]
    DuplicateName(String),
    /// Compact ID 0 is reserved.
    #[error("channel `{name}` uses reserved compact ID 0")]
    ReservedCompactId {
        /// Offending channel.
        name: String,
    },
    /// Empty, overlong or control-character name.
    #[error("invalid channel name `{0}`")]
    InvalidName(String),
    /// Codec other than the supported scalar codec.
    #[error("channel `{name}` uses unsupported codec `{codec_id}`")]
    UnsupportedCodec {
        /// Offending channel.
        name: String,
        /// Declared codec.
        codec_id: String,
    },
    /// Schema or wire ID is not `sha256:` plus 64 lowercase hex digits.
    #[error("channel `{name}` has a malformed schema or wire identity")]
    MalformedIdentity {
        /// Offending channel.
        name: String,
    },
    /// Zero or oversized payload length.
    #[error("channel `{name}` has invalid wire length {wire_len}")]
    InvalidWireLen {
        /// Offending channel.
        name: String,
        /// Declared length.
        wire_len: usize,
    },
    /// Declared digest differs from the recomputed digest.
    #[error("channel manifest digest mismatch: declared {declared}, computed {computed}")]
    DigestMismatch {
        /// Digest in the document.
        declared: String,
        /// Digest of the verified entries.
        computed: String,
    },
    /// No entry for the compact ID.
    #[error("compact ID {0} is not bound")]
    UnknownCompactId(u16),
    /// The bound entry does not carry the receiver's layout.
    #[error("channel `{name}` does not carry the expected wire layout")]
    LayoutMismatch {
        /// Offending channel.
        name: String,
    },
}
