//! Name-casing helpers shared by the emitters.

/// Convert a contract name such as `vehicle-depth` to `vehicle_depth`.
pub fn to_snake_case(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c == '-' || c == ' ' {
                '_'
            } else {
                c.to_ascii_lowercase()
            }
        })
        .collect()
}

/// Convert a contract name such as `vehicle-depth` to `VehicleDepth`.
pub fn to_pascal_case(name: &str) -> String {
    name.split(['-', '_', ' '])
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut chars = segment.chars();
            match chars.next() {
                Some(first) => {
                    let mut out = first.to_ascii_uppercase().to_string();
                    out.extend(chars.map(|c| c.to_ascii_lowercase()));
                    out
                }
                None => String::new(),
            }
        })
        .collect()
}
