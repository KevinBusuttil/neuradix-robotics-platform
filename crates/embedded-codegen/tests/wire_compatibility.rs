mod common;

use std::process::Command;

use neuradix_contracts::{PrimitiveType, schema_identity};
use neuradix_embedded_codegen::{
    CODEC_ID, CppTarget, WireLayout, generate_cpp, generate_cpp_for_target, generate_nostd_rust,
};
use sha2::{Digest, Sha256};

use common::{TempDir, depth, run_cpp, run_rust, tiny};

#[test]
fn wire_identity_covers_codec_schema_and_ordered_offsets() {
    let c = depth();
    let layout = WireLayout::for_contract(&c).unwrap();
    assert_eq!(layout.schema_id, schema_identity(&c).to_string());
    assert_eq!(layout.codec_id, CODEC_ID);
    assert_eq!(layout.fields[0].name, "depth");
    assert_eq!(layout.fields[1].offset, 8);
    let descriptor = format!(
        "{{\"codec_id\":\"neuradix.scalar-le.v2\",\"schema_id\":\"{}\",\"wire_len\":16,\"fields\":[{{\"name\":\"depth\",\"ty\":\"float64\",\"offset\":0,\"size\":8}},{{\"name\":\"uncertainty\",\"ty\":\"float64\",\"offset\":8,\"size\":8}}]}}",
        layout.schema_id
    );
    assert_eq!(layout.descriptor_bytes(), descriptor.as_bytes());
    assert_eq!(
        layout.wire_id,
        format!("sha256:{:x}", Sha256::digest(descriptor))
    );
    let mut changed_codec = layout.clone();
    changed_codec.codec_id = "neuradix.scalar-le.v1".to_owned();
    assert_ne!(layout.descriptor_bytes(), changed_codec.descriptor_bytes());
    let mut changed = c.clone();
    changed.spec.payload.fields[0].ty = PrimitiveType::Float32;
    assert_ne!(
        layout.wire_id,
        WireLayout::for_contract(&changed).unwrap().wire_id
    );
}

#[test]
fn independently_generated_cpp_sender_and_reordered_rust_receiver_agree() {
    let dir = TempDir::new("independent-wire");
    let original = depth();
    let mut reordered = original.clone();
    reordered.spec.payload.fields.reverse();
    let producer = generate_cpp(&original).unwrap();
    let consumer = generate_nostd_rust(&reordered).unwrap();
    let mut legacy_layout = WireLayout::for_contract(&original).unwrap();
    legacy_layout.codec_id = "neuradix.scalar-le.v1".to_owned();
    let legacy_id = format!(
        "sha256:{:x}",
        Sha256::digest(legacy_layout.descriptor_bytes())
    );
    std::fs::write(dir.0.join("vehicle_depth.h"), producer.code).unwrap();
    std::fs::write(dir.0.join("consumer.rs"), consumer.code).unwrap();
    // Transfer bytes and the PRODUCER's identity via a file. Do not substitute
    // the consumer's constant for the peer identity being verified.
    run_cpp(
        &dir,
        r#"
#include "vehicle_depth.h"
#include <stdio.h>
int main() {
  neuradix::VehicleDepth v = {12.5, 0.125};
  uint8_t bytes[16];
  if (v.encode(bytes, sizeof(bytes)) != 16) return 1;
  FILE* f = fopen("peer.bin", "wb");
  if (!f) return 2;
  bool ok = fwrite(neuradix::VehicleDepth::WIRE_ID, 1, 71, f) == 71
    && fwrite(bytes, 1, sizeof(bytes), f) == sizeof(bytes);
  fclose(f);
  return ok ? 0 : 3;
}
"#,
    );
    let source = format!(
        r#"
mod consumer {{ include!("consumer.rs"); }}
use consumer::VehicleDepth;
fn main() {{
    let peer = std::fs::read({:?}).unwrap();
    let peer_id = std::str::from_utf8(&peer[..71]).unwrap();
    let v = VehicleDepth::decode(&peer[71..], peer_id).unwrap();
    assert_eq!(v.depth, 12.5);
    assert_eq!(v.uncertainty, 0.125);
    let mut out = [0u8; 16];
    assert_eq!(v.encode(&mut out), Some(16));
    assert_eq!(out, peer[71..]);
    assert!(VehicleDepth::decode(&peer[71..], VehicleDepth::SCHEMA_ID).is_none());
    assert!(VehicleDepth::decode(&peer[71..], "legacy-unversioned").is_none());
    assert!(VehicleDepth::decode(&peer[71..], "{legacy_id}").is_none());
    assert!(VehicleDepth::decode(&peer[71..86], peer_id).is_none());
    assert!(VehicleDepth::decode(&[0; 17], peer_id).is_none());
}}
"#,
        dir.0.join("peer.bin")
    );
    run_rust(&dir, &source);
}

#[test]
fn avr_profile_rejects_binary64_before_generation() {
    let error = generate_cpp_for_target(&depth(), CppTarget::AvrUno).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("field `depth` has type `float64`")
    );
    assert!(error.to_string().contains("avr-uno"));
    assert!(generate_cpp_for_target(&tiny(), CppTarget::AvrUno).is_ok());
}

#[test]
fn mixed_scalar_boundaries_and_invalid_input_match_across_languages() {
    let dir = TempDir::new("scalar-boundaries");
    let c = tiny();
    std::fs::write(
        dir.0.join("tiny_telemetry.h"),
        generate_cpp(&c).unwrap().code,
    )
    .unwrap();
    std::fs::write(dir.0.join("tiny.rs"), generate_nostd_rust(&c).unwrap().code).unwrap();
    // Authored fields are shuffled; expected bytes are hand-written in canonical
    // name order: true, -0f32, i32::MIN, i64::MIN, u32::MAX, u64::MAX.
    let expected =
        "1,0,0,0,128,0,0,0,128,0,0,0,0,0,0,0,128,255,255,255,255,255,255,255,255,255,255,255,255";
    run_cpp(
        &dir,
        &format!(
            r#"
#include "tiny_telemetry.h"
#include <assert.h>
using T = neuradix::TinyTelemetry;
int main() {{
  const uint8_t expected[] = {{{expected}}};
  T v = {{true, -0.0f, INT32_MIN, INT64_MIN, UINT32_MAX, UINT64_MAX}};
  uint8_t buf[T::WIRE_LEN];
  assert(sizeof(expected) == T::WIRE_LEN);
  assert(v.encode(buf, sizeof(buf)) == sizeof(expected));
  assert(memcmp(buf, expected, sizeof(buf)) == 0);
  T out = {{false, 42.0f, 1, 2, 3, 4}};
  assert(!T::decode(expected, sizeof(expected), out, T::SCHEMA_ID));
  assert(!T::decode(expected, sizeof(expected), out, nullptr));
  assert(!T::decode(nullptr, sizeof(expected), out, T::WIRE_ID));
  assert(!T::decode(expected, sizeof(expected)-1, out, T::WIRE_ID));
  assert(!T::decode(expected, sizeof(expected)+1, out, T::WIRE_ID));
  buf[0] = 2;
  assert(!T::decode(buf, sizeof(buf), out, T::WIRE_ID));
  assert(!out.a_enabled && out.b_measurement == 42.0f && out.z_unsigned64 == 4);
  assert(v.encode(buf, sizeof(buf)-1) == 0);
  assert(v.encode(nullptr, sizeof(buf)) == 0);
  assert(T::decode(expected, sizeof(expected), out, T::WIRE_ID));
  assert(out.c_signed32 == INT32_MIN && out.d_signed64 == INT64_MIN);
  assert(out.encode(buf, sizeof(buf)) == sizeof(buf));
  assert(memcmp(buf, expected, sizeof(buf)) == 0);
  // Preserve infinity and a NaN payload at the byte level.
  const uint32_t patterns[] = {{0x7f800000u, 0xff800000u, 0x7fc01234u, 1u}};
  for (uint32_t bits : patterns) {{
    memcpy(buf, expected, sizeof(buf));
    for (size_t k=0; k<4; ++k) buf[1+k] = (uint8_t)(bits >> (8*k));
    assert(T::decode(buf, sizeof(buf), out, T::WIRE_ID));
    uint8_t roundtrip[T::WIRE_LEN];
    assert(out.encode(roundtrip, sizeof(roundtrip)) == sizeof(roundtrip));
    assert(memcmp(buf, roundtrip, sizeof(buf)) == 0);
  }}
}}
"#
        ),
    );
    run_rust(
        &dir,
        &format!(
            r#"
mod generated {{ include!("tiny.rs"); }}
use generated::TinyTelemetry as T;
fn main() {{
    let expected: [u8; 29] = [{expected}];
    let v = T {{ a_enabled: true, b_measurement: -0.0, c_signed32: i32::MIN,
        d_signed64: i64::MIN, e_unsigned32: u32::MAX, z_unsigned64: u64::MAX }};
    let mut buf = [0; T::WIRE_LEN];
    assert_eq!(v.encode(&mut buf), Some(29));
    assert_eq!(buf, expected);
    let out = T::decode(&expected, T::WIRE_ID).unwrap();
    assert_eq!(out.b_measurement.to_bits(), (-0.0f32).to_bits());
    assert_eq!(out.c_signed32, i32::MIN);
    assert_eq!(out.d_signed64, i64::MIN);
    buf[0] = 255;
    assert!(T::decode(&buf, T::WIRE_ID).is_none());
    assert!(T::decode(&expected, T::SCHEMA_ID).is_none());
    for bits in [0x7f800000u32, 0xff800000, 0x7fc01234, 1] {{
        buf = expected;
        buf[1..5].copy_from_slice(&bits.to_le_bytes());
        let decoded = T::decode(&buf, T::WIRE_ID).unwrap();
        assert_eq!(decoded.b_measurement.to_bits(), bits);
        let mut roundtrip = [0; 29];
        assert_eq!(decoded.encode(&mut roundtrip), Some(29));
        assert_eq!(roundtrip, buf);
    }}
}}
"#
        ),
    );
}

#[test]
fn generated_rust_builds_without_std() {
    let dir = TempDir::new("nostd");
    let code = format!("#![no_std]\n{}", generate_nostd_rust(&tiny()).unwrap().code);
    let path = dir.0.join("lib.rs");
    std::fs::write(&path, code).unwrap();
    common::success(
        Command::new("rustc")
            .args(["--edition=2024", "--crate-type=lib", "-Dwarnings"])
            .arg(path)
            .arg("--out-dir")
            .arg(&dir.0)
            .output()
            .unwrap(),
    );
}
