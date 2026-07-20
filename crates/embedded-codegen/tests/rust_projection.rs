//! Compile and run the *generated* `no_std` Rust projection against the golden
//! vectors — proving the generated code is valid and byte-agrees with the Rust
//! reference and the C++ projection.

mod generated {
    include!("golden/vehicle_depth.rs");
}
use generated::VehicleDepth;

use std::path::PathBuf;

use neuradix_contracts::{Contract, load_file};
use neuradix_embedded_codegen::{GoldenSet, ScalarValue, golden_vectors};

fn contract() -> Contract {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/standard/navigation/vehicle-depth.yaml");
    load_file(&path).unwrap()
}

fn f64_of(v: ScalarValue) -> f64 {
    match v {
        ScalarValue::F64(x) => x,
        other => panic!("expected f64, got {other:?}"),
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::new();
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[test]
fn generated_rust_encodes_and_decodes_the_golden_vectors() {
    let c = contract();
    let golden = golden_vectors(&c).unwrap();
    let rows = GoldenSet::value_rows(&c).unwrap();

    assert_eq!(VehicleDepth::WIRE_LEN, 16);
    assert_eq!(
        VehicleDepth::SCHEMA_ID,
        "sha256:4c9c5d9381658f7779ef0d3ef11eda3f29006f7e751d06dba40a12d6f4ce2a73"
    );

    for ((_, values), vector) in rows.iter().zip(&golden.vectors) {
        let sample = VehicleDepth {
            depth: f64_of(values[0]),
            uncertainty: f64_of(values[1]),
        };

        // encode == the golden bytes (agrees with C++ and the reference).
        let mut buf = [0u8; VehicleDepth::WIRE_LEN];
        assert_eq!(sample.encode(&mut buf), Some(VehicleDepth::WIRE_LEN));
        assert_eq!(hex(&buf), vector.bytes_hex, "vector `{}`", vector.name);

        // decode(golden bytes) round-trips exactly.
        let decoded = VehicleDepth::decode(&buf).expect("decode");
        assert_eq!(decoded, sample);
    }
}

#[test]
fn generated_rust_rejects_a_short_buffer() {
    let sample = VehicleDepth {
        depth: 1.0,
        uncertainty: 2.0,
    };
    let mut small = [0u8; 8];
    assert_eq!(sample.encode(&mut small), None);
    assert!(VehicleDepth::decode(&small).is_none());
}
