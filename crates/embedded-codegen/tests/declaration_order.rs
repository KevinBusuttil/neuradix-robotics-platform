//! This regression uses only APIs present at c8aa467, so it can also be run
//! against the pre-fix baseline to demonstrate the field-order defect.

use std::path::PathBuf;

use neuradix_contracts::{load_file, schema_identity};
use neuradix_embedded_codegen::{generate_cpp, generate_nostd_rust, golden_vectors};

#[test]
fn reordering_equal_width_fields_preserves_generated_wire() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/standard/navigation/vehicle-depth.yaml");
    let original = load_file(&path).unwrap();
    let mut reordered = original.clone();
    reordered.spec.payload.fields.reverse();
    assert_eq!(schema_identity(&original), schema_identity(&reordered));

    // Both contracts have distinct depth/uncertainty values of the same width.
    // Schema identity alone could not detect their swapped legacy offsets.
    assert_eq!(
        generate_cpp(&original).unwrap().code,
        generate_cpp(&reordered).unwrap().code,
        "equal schema identities must not hide different scalar offsets"
    );
    assert_eq!(
        generate_nostd_rust(&original).unwrap().code,
        generate_nostd_rust(&reordered).unwrap().code
    );
    assert_eq!(
        serde_json::to_value(golden_vectors(&original).unwrap()).unwrap(),
        serde_json::to_value(golden_vectors(&reordered).unwrap()).unwrap()
    );
}
