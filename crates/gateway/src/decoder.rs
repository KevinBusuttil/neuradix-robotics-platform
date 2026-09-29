//! Generated decoder registrations and typed handlers.
//!
//! A generated `no_std` payload type (`neuradix contract generate --language
//! nostd-rust`) exposes `WIRE_LEN`, `SCHEMA_ID`, `CODEC_ID`, `WIRE_ID` and
//! `decode(input, peer_wire_id)`. [`generated_decoder!`](crate::generated_decoder)
//! implements [`GeneratedDecoder`] by forwarding to exactly those items, so the
//! gateway compares and calls the generated code itself, not a copy of it.
//!
//! A [`Registration`] pairs one decoder type with the handler that receives
//! its values. Its dispatch function passes the **resolved binding's**
//! `wire_id` to `decode`; it never substitutes the decoder's own `WIRE_ID`.

use neuradix_embedded_transport::ChannelBinding;

/// The items every generated `no_std` payload type provides.
pub trait GeneratedDecoder: Sized {
    /// Type name, for reports.
    const TYPE_NAME: &'static str;
    /// Exact wire length.
    const WIRE_LEN: usize;
    /// Semantic schema identity the decoder was generated from.
    const SCHEMA_ID: &'static str;
    /// Codec the decoder implements.
    const CODEC_ID: &'static str;
    /// Wire identity the decoder accepts.
    const WIRE_ID: &'static str;
    /// The generated `decode(input, peer_wire_id)`.
    fn decode(input: &[u8], peer_wire_id: &str) -> Option<Self>;
}

/// Implement [`GeneratedDecoder`] for a generated payload type by forwarding to
/// its inherent constants and `decode`.
#[macro_export]
macro_rules! generated_decoder {
    ($ty:ident) => {
        impl $crate::GeneratedDecoder for $ty {
            const TYPE_NAME: &'static str = stringify!($ty);
            const WIRE_LEN: usize = $ty::WIRE_LEN;
            const SCHEMA_ID: &'static str = $ty::SCHEMA_ID;
            const CODEC_ID: &'static str = $ty::CODEC_ID;
            const WIRE_ID: &'static str = $ty::WIRE_ID;
            fn decode(input: &[u8], peer_wire_id: &str) -> Option<Self> {
                $ty::decode(input, peer_wire_id)
            }
        }
    };
}

/// A typed telemetry handler for values of `T`.
///
/// `channel` is the resolved manifest binding; one contract may back several
/// channels, so handlers can tell them apart.
pub trait Handle<T> {
    /// Receive one decoded value.
    fn handle(&mut self, channel: &ChannelBinding, value: T);
}

/// Decode `body` with `T` using the binding's wire ID and, only on success,
/// hand the value to `H`.
fn dispatch<T: GeneratedDecoder, H: Handle<T>>(
    handler: &mut H,
    binding: &ChannelBinding,
    body: &[u8],
) -> bool {
    match T::decode(body, binding.wire_id) {
        Some(value) => {
            handler.handle(binding, value);
            true
        }
        None => false,
    }
}

/// One generated decoder registered for handler type `H`.
pub struct Registration<H> {
    pub(crate) type_name: &'static str,
    pub(crate) wire_len: usize,
    pub(crate) schema_id: &'static str,
    pub(crate) codec_id: &'static str,
    pub(crate) wire_id: &'static str,
    pub(crate) dispatch: fn(&mut H, &ChannelBinding, &[u8]) -> bool,
}

impl<H> Registration<H> {
    /// Register generated decoder `T`, delivering to `H`'s [`Handle<T>`].
    pub fn of<T: GeneratedDecoder>() -> Self
    where
        H: Handle<T>,
    {
        Self {
            type_name: T::TYPE_NAME,
            wire_len: T::WIRE_LEN,
            schema_id: T::SCHEMA_ID,
            codec_id: T::CODEC_ID,
            wire_id: T::WIRE_ID,
            dispatch: dispatch::<T, H>,
        }
    }

    /// The registered type's name.
    pub fn type_name(&self) -> &'static str {
        self.type_name
    }

    /// The registered decoder's wire identity.
    pub fn wire_id(&self) -> &'static str {
        self.wire_id
    }
}

impl<H> Clone for Registration<H> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<H> Copy for Registration<H> {}

impl<H> core::fmt::Debug for Registration<H> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Registration")
            .field("type_name", &self.type_name)
            .field("wire_len", &self.wire_len)
            .field("wire_id", &self.wire_id)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference::{TinyTelemetry, VehicleDepth};
    use neuradix_embedded_transport::channel::CODEC_ID;

    #[derive(Default)]
    struct Seen(Vec<VehicleDepth>);
    impl Handle<VehicleDepth> for Seen {
        fn handle(&mut self, _: &ChannelBinding, value: VehicleDepth) {
            self.0.push(value);
        }
    }

    fn binding(wire_id: &'static str) -> ChannelBinding {
        ChannelBinding {
            compact_id: 1,
            wire_len: 16,
            name: "vehicle-depth",
            codec_id: CODEC_ID,
            schema_id: VehicleDepth::SCHEMA_ID,
            wire_id,
        }
    }

    /// The dispatch path decodes with the resolved binding's wire ID. If it
    /// substituted the decoder's own `WIRE_ID`, the foreign binding below
    /// would decode.
    #[test]
    fn dispatch_passes_the_binding_wire_id_to_the_generated_decoder() {
        let reg = Registration::<Seen>::of::<VehicleDepth>();
        let v = VehicleDepth {
            depth: 4.0,
            uncertainty: 0.5,
        };
        let mut body = [0; VehicleDepth::WIRE_LEN];
        v.encode(&mut body);
        let mut seen = Seen::default();
        assert!(!(reg.dispatch)(
            &mut seen,
            &binding(TinyTelemetry::WIRE_ID),
            &body
        ));
        assert!(seen.0.is_empty());
        assert!((reg.dispatch)(
            &mut seen,
            &binding(VehicleDepth::WIRE_ID),
            &body
        ));
        assert_eq!(seen.0, [v]);
        assert_eq!(reg.type_name(), "VehicleDepth");
        assert_eq!(reg.wire_id(), VehicleDepth::WIRE_ID);
    }
}
