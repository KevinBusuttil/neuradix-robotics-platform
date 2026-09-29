//! Immutable, verified routing state for one gateway instance.
//!
//! [`RoutingTable::new`] checks the **complete** configured mapping before any
//! traffic is processed, and either returns a table or no table at all:
//!
//! 1. The configured manifest artifact is at most [`MAX_MANIFEST_BYTES`] and
//!    passes `ChannelManifest::parse`: format, entry rules and the declared
//!    digest recomputed from the entries.
//! 2. The manifest and the route list fit the table capacity `N`.
//! 3. Routes are unique by compact ID and by name.
//! 4. Every route names a manifest entry with the same name, the entry carries
//!    exactly the expected contract's layout (`ChannelManifest::verify_layout`),
//!    and the registered generated decoder was built for that same layout
//!    (codec, schema ID, wire ID and length).
//! 5. Every manifest entry has a route: a channel the gateway cannot decode is
//!    a configuration error, not traffic to drop later.
//! 6. The compiled board table passes `ChannelTable::new` (which recomputes
//!    its digest from its bindings) and equals the configured manifest, digest
//!    and entries.
//!
//! The table exposes no mutation. A different mapping needs a new instance.

use neuradix_contracts::layout::{LayoutError, WireLayout};
use neuradix_contracts::{ChannelManifest, ChannelManifestError, Contract};
use neuradix_embedded_transport::{
    BindError, ChannelBinding, ChannelTable, ENVELOPE_HEADER, EnvelopeError,
};

use crate::decoder::Registration;

/// Largest configured manifest accepted, in bytes (the CLI's manifest limit).
pub const MAX_MANIFEST_BYTES: usize = 1 << 20;

/// What the gateway expects on one compact channel: an explicit ID and name,
/// and the layout derived from the expected contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedRoute {
    compact_id: u16,
    name: String,
    layout: WireLayout,
}

impl ExpectedRoute {
    /// Derive the expected layout from a validated contract. Contracts the
    /// scalar codec cannot represent (for example `string` fields) are refused.
    pub fn from_contract(
        compact_id: u16,
        name: impl Into<String>,
        contract: &Contract,
    ) -> Result<Self, InitError> {
        let name = name.into();
        let layout = WireLayout::for_contract(contract).map_err(|source| {
            InitError::UnsupportedContract {
                name: name.clone(),
                source,
            }
        })?;
        Ok(Self {
            compact_id,
            name,
            layout,
        })
    }

    /// The compact ID.
    pub fn compact_id(&self) -> u16 {
        self.compact_id
    }

    /// The channel name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The expected layout.
    pub fn layout(&self) -> &WireLayout {
        &self.layout
    }
}

/// An expected channel and the generated decoder that handles it.
#[derive(Debug)]
pub struct Route<H> {
    /// Expected channel.
    pub expected: ExpectedRoute,
    /// Generated decoder and handler.
    pub decoder: Registration<H>,
}

impl<H> Route<H> {
    /// Pair an expected channel with its decoder registration.
    pub fn new(expected: ExpectedRoute, decoder: Registration<H>) -> Self {
        Self { expected, decoder }
    }
}

/// The generated board table compiled into this build
/// (`neuradix channel table`, or `generate_channel_table`).
#[derive(Debug, Clone, Copy)]
pub struct BoardTable {
    /// Generated `MANIFEST_DIGEST`.
    pub digest: [u8; 32],
    /// Generated `CHANNELS`.
    pub channels: &'static [ChannelBinding],
}

/// Why a gateway was not created. No routing state exists after an error, so
/// no handler can be called.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InitError {
    /// The configured manifest exceeds [`MAX_MANIFEST_BYTES`].
    #[error("configured manifest is {len} bytes, above the {limit}-byte limit")]
    ManifestTooLarge {
        /// Size of the supplied manifest.
        len: usize,
        /// The limit.
        limit: usize,
    },
    /// The configured manifest is malformed, breaks a rule or declares a
    /// digest its entries do not produce.
    #[error("configured manifest rejected: {0}")]
    Manifest(#[from] ChannelManifestError),
    /// More manifest channels or routes than the table capacity.
    #[error("{what} count {count} exceeds the capacity of {capacity}")]
    Capacity {
        /// `"manifest channel"` or `"route"`.
        what: &'static str,
        /// Supplied count.
        count: usize,
        /// Table capacity.
        capacity: usize,
    },
    /// Two routes share a compact ID.
    #[error("compact ID {compact_id} has more than one route")]
    DuplicateRoute {
        /// Duplicated compact ID.
        compact_id: u16,
    },
    /// Two routes share a name.
    #[error("channel name `{name}` has more than one route")]
    DuplicateRouteName {
        /// Duplicated name.
        name: String,
    },
    /// An expected contract cannot be represented by the scalar codec.
    #[error("route `{name}`: unsupported contract: {source}")]
    UnsupportedContract {
        /// Route name.
        name: String,
        /// Why the layout could not be derived.
        source: LayoutError,
    },
    /// An expected contract document is not a valid contract.
    #[error("route `{name}`: invalid contract: {reason}")]
    InvalidContract {
        /// Route name.
        name: String,
        /// Validation failure.
        reason: String,
    },
    /// A route names a compact ID the manifest does not bind.
    #[error("route for compact ID {compact_id} has no manifest entry")]
    UnknownRouteChannel {
        /// Route compact ID.
        compact_id: u16,
    },
    /// A manifest entry has no route.
    #[error("manifest channel {compact_id} (`{name}`) has no route")]
    UnroutedChannel {
        /// Manifest compact ID.
        compact_id: u16,
        /// Manifest name.
        name: String,
    },
    /// Route and manifest disagree on the channel name.
    #[error("compact ID {compact_id}: route expects `{expected}`, manifest names `{manifest}`")]
    NameMismatch {
        /// Compact ID.
        compact_id: u16,
        /// Route name.
        expected: String,
        /// Manifest name.
        manifest: String,
    },
    /// The manifest entry does not carry the expected contract's layout.
    #[error("compact ID {compact_id} (`{name}`) does not carry the expected contract layout")]
    ContractMismatch {
        /// Compact ID.
        compact_id: u16,
        /// Channel name.
        name: String,
    },
    /// The registered generated decoder was built for a different layout.
    #[error(
        "compact ID {compact_id}: decoder `{decoder}` was not generated for the expected contract"
    )]
    DecoderMismatch {
        /// Compact ID.
        compact_id: u16,
        /// Registered type.
        decoder: &'static str,
    },
    /// The compiled board table is inconsistent (`ChannelTable::new` failed).
    #[error("compiled board table rejected: {0}")]
    BoardTable(BindError),
    /// The compiled board table is consistent but is not the configured manifest.
    #[error("compiled board table does not match the configured manifest")]
    BoardTableMismatch,
    /// The frame buffer cannot hold the largest bound envelope.
    #[error("frame buffer of {capacity} bytes cannot hold a {needed}-byte envelope")]
    FrameCapacity {
        /// Largest envelope.
        needed: usize,
        /// Frame buffer capacity.
        capacity: usize,
    },
}

/// Why one frame reached no handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    /// The frame failed its CRC or exceeded the frame buffer; the transport
    /// reports both as one event.
    CorruptFrame,
    /// The envelope was refused: truncated, wrong magic or version, another
    /// manifest's tag, an unknown or reserved compact ID, or a wrong length.
    Envelope(EnvelopeError),
    /// The generated decoder refused the body (for example a non-canonical
    /// boolean, or a wire identity other than its own).
    Decode {
        /// Resolved compact ID.
        compact_id: u16,
    },
}

/// A frame delivered to a typed handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Delivered {
    /// Resolved compact ID.
    pub compact_id: u16,
    /// Resolved channel name.
    pub name: &'static str,
}

struct RouteEntry<H> {
    compact_id: u16,
    decoder: Registration<H>,
}

/// Verified, immutable routing state: the compiled board table and one
/// generated decoder per bound channel.
pub struct RoutingTable<H, const N: usize> {
    table: ChannelTable<N>,
    routes: Box<[RouteEntry<H>]>,
    digest: String,
    max_envelope: usize,
}

impl<H, const N: usize> RoutingTable<H, N> {
    /// Verify the configured manifest, the expected routes, their decoders
    /// and the compiled board table together (see the module documentation).
    pub fn new(
        manifest_json: &str,
        board: BoardTable,
        routes: Vec<Route<H>>,
    ) -> Result<Self, InitError> {
        if manifest_json.len() > MAX_MANIFEST_BYTES {
            return Err(InitError::ManifestTooLarge {
                len: manifest_json.len(),
                limit: MAX_MANIFEST_BYTES,
            });
        }
        let manifest = ChannelManifest::parse(manifest_json)?;
        for (what, count) in [
            ("manifest channel", manifest.channels().len()),
            ("route", routes.len()),
        ] {
            if count > N {
                return Err(InitError::Capacity {
                    what,
                    count,
                    capacity: N,
                });
            }
        }

        let mut routes = routes;
        routes.sort_by_key(|r| r.expected.compact_id);
        for pair in routes.windows(2) {
            if pair[0].expected.compact_id == pair[1].expected.compact_id {
                return Err(InitError::DuplicateRoute {
                    compact_id: pair[0].expected.compact_id,
                });
            }
        }
        for (i, r) in routes.iter().enumerate() {
            if routes[..i]
                .iter()
                .any(|p| p.expected.name == r.expected.name)
            {
                return Err(InitError::DuplicateRouteName {
                    name: r.expected.name.clone(),
                });
            }
        }

        for r in &routes {
            let (id, expected) = (r.expected.compact_id, &r.expected);
            let entry = manifest
                .resolve(id)
                .ok_or(InitError::UnknownRouteChannel { compact_id: id })?;
            if entry.name != expected.name {
                return Err(InitError::NameMismatch {
                    compact_id: id,
                    expected: expected.name.clone(),
                    manifest: entry.name.clone(),
                });
            }
            manifest.verify_layout(id, &expected.layout).map_err(|_| {
                InitError::ContractMismatch {
                    compact_id: id,
                    name: entry.name.clone(),
                }
            })?;
            let d = &r.decoder;
            let layout = &expected.layout;
            if d.codec_id != layout.codec_id
                || d.schema_id != layout.schema_id
                || d.wire_id != layout.wire_id
                || d.wire_len != layout.wire_len
            {
                return Err(InitError::DecoderMismatch {
                    compact_id: id,
                    decoder: d.type_name,
                });
            }
        }
        for entry in manifest.channels() {
            if routes
                .binary_search_by_key(&entry.compact_id, |r| r.expected.compact_id)
                .is_err()
            {
                return Err(InitError::UnroutedChannel {
                    compact_id: entry.compact_id,
                    name: entry.name.clone(),
                });
            }
        }

        let table =
            ChannelTable::<N>::new(board.digest, board.channels).map_err(InitError::BoardTable)?;
        let same_entries = manifest.channels().len() == board.channels.len()
            && manifest
                .channels()
                .iter()
                .zip(board.channels)
                .all(|(m, b)| {
                    m.compact_id == b.compact_id
                        && m.wire_len == usize::from(b.wire_len)
                        && m.name == b.name
                        && m.codec_id == b.codec_id
                        && m.schema_id == b.schema_id
                        && m.wire_id == b.wire_id
                });
        if board.digest != manifest.digest_bytes() || !same_entries {
            return Err(InitError::BoardTableMismatch);
        }

        let max_envelope = ENVELOPE_HEADER
            + board
                .channels
                .iter()
                .map(|b| usize::from(b.wire_len))
                .max()
                .unwrap_or(0);
        Ok(Self {
            table,
            routes: routes
                .into_iter()
                .map(|r| RouteEntry {
                    compact_id: r.expected.compact_id,
                    decoder: r.decoder,
                })
                .collect(),
            digest: manifest.digest(),
            max_envelope,
        })
    }

    /// Resolve one frame payload and, only if every check passes, decode it
    /// with the channel's generated decoder and call its typed handler.
    /// Allocation-free.
    pub fn route(&self, payload: &[u8], handler: &mut H) -> Result<Delivered, Rejection> {
        let envelope = self.table.open(payload).map_err(Rejection::Envelope)?;
        let binding = envelope.binding;
        let entry = self
            .routes
            .binary_search_by_key(&binding.compact_id, |r| r.compact_id)
            .map(|i| &self.routes[i])
            // Unreachable: every bound channel has a route (checked at init).
            .map_err(|_| Rejection::Envelope(EnvelopeError::UnknownChannel(binding.compact_id)))?;
        if (entry.decoder.dispatch)(handler, &binding, envelope.body) {
            Ok(Delivered {
                compact_id: binding.compact_id,
                name: binding.name,
            })
        } else {
            Err(Rejection::Decode {
                compact_id: binding.compact_id,
            })
        }
    }

    /// The full verified manifest digest, `sha256:<64 hex>`.
    pub fn manifest_digest(&self) -> &str {
        &self.digest
    }

    /// The per-envelope manifest tag (first four digest bytes). A
    /// misconfiguration detector, not an identity or authentication.
    pub fn manifest_tag(&self) -> [u8; 4] {
        self.table.manifest_tag()
    }

    /// Largest bound envelope (header plus body), in bytes.
    pub fn max_envelope_len(&self) -> usize {
        self.max_envelope
    }

    /// The routes, in compact-ID order: binding and decoder type name.
    pub fn routes(&self) -> impl Iterator<Item = (&ChannelBinding, &'static str)> + '_ {
        self.routes.iter().filter_map(|r| {
            self.table
                .resolve(r.compact_id)
                .map(|b| (b, r.decoder.type_name))
        })
    }
}

impl<H, const N: usize> core::fmt::Debug for RoutingTable<H, N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RoutingTable")
            .field("digest", &self.digest)
            .field("routes", &self.routes.len())
            .finish_non_exhaustive()
    }
}
