//! Guards that the committed generated type is a faithful, up-to-date product of
//! the authored contract. If this fails, regenerate with
//! `neuradix contract generate contracts/standard/navigation/vehicle-depth.yaml
//! --language rust --out-dir examples/minimal-depth-stream/src/generated`.

use neuradix_testkit::golden::assert_generated_matches;

const CONTRACT_YAML: &str =
    include_str!("../../../contracts/standard/navigation/vehicle-depth.yaml");
const COMMITTED: &str = include_str!("../src/generated/vehicle_depth.rs");

#[test]
fn committed_generated_file_is_up_to_date() {
    assert_generated_matches(CONTRACT_YAML, COMMITTED);
}
