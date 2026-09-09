//! Bounded immutable component configuration, canonicalized before identity.
use serde_json::Value as Json;
use serde_yaml::Value;

/// Canonical object configuration. Floats, YAML tags and non-string keys are
/// explicitly unsupported. Integers retain their exact i64/u64 value.
///
/// ```compile_fail
/// let mut c = neuradix_graph::ComponentConfiguration::default();
/// c.canonical = String::new();
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentConfiguration {
    canonical: String,
}
impl Default for ComponentConfiguration {
    fn default() -> Self {
        Self {
            canonical: "{}".to_owned(),
        }
    }
}
impl ComponentConfiguration {
    /// Maximum compact JSON bytes per component (inclusive).
    pub const MAX_BYTES: usize = 65_536;
    /// Maximum values including the root object (inclusive).
    pub const MAX_VALUES: usize = 1024;
    /// Maximum nesting depth, root at zero (inclusive).
    pub const MAX_DEPTH: usize = 16;

    /// Validate a caller-owned YAML value before copying it into canonical JSON.
    /// Source parser allocation and caller-owned values are outside this budget.
    pub fn from_value(value: &Value) -> Result<Self, &'static str> {
        if !matches!(value, Value::Mapping(_)) {
            return Err("configuration must be an object");
        }
        let mut count = 0;
        let mut bytes = 0;
        inspect(value, 0, &mut count, &mut bytes)?;
        let canonical = serde_json::to_string(&convert(value)).expect("validated JSON subset");
        debug_assert_eq!(canonical.len(), bytes);
        Ok(Self { canonical })
    }
    /// Compact key-sorted JSON, arrays ordered; immutable and fully validated.
    pub fn canonical_json(&self) -> &str {
        &self.canonical
    }
}
fn charge(bytes: &mut usize, n: usize) -> Result<(), &'static str> {
    *bytes = bytes.checked_add(n).ok_or("configuration size overflow")?;
    if *bytes > ComponentConfiguration::MAX_BYTES {
        return Err("configuration byte limit");
    }
    Ok(())
}
fn string_size(s: &str, bytes: &mut usize) -> Result<(), &'static str> {
    charge(bytes, 2)?;
    for c in s.chars() {
        charge(
            bytes,
            match c {
                '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
                c if c <= '\u{1f}' => 6,
                c => c.len_utf8(),
            },
        )?;
    }
    Ok(())
}
fn inspect(
    v: &Value,
    depth: usize,
    count: &mut usize,
    bytes: &mut usize,
) -> Result<(), &'static str> {
    if depth > ComponentConfiguration::MAX_DEPTH {
        return Err("configuration depth limit");
    }
    *count += 1;
    if *count > ComponentConfiguration::MAX_VALUES {
        return Err("configuration value limit");
    }
    match v {
        Value::Null => charge(bytes, 4),
        Value::Bool(b) => charge(bytes, if *b { 4 } else { 5 }),
        Value::Number(n) => {
            if n.as_i64().is_none() && n.as_u64().is_none() {
                return Err("configuration requires exact integers; floats unsupported");
            }
            charge(bytes, n.to_string().len())
        }
        Value::String(s) => string_size(s, bytes),
        Value::Sequence(a) => {
            charge(bytes, 2)?;
            charge(bytes, a.len().saturating_sub(1))?;
            for x in a {
                inspect(x, depth + 1, count, bytes)?;
            }
            Ok(())
        }
        Value::Mapping(m) => {
            charge(bytes, 2)?;
            charge(bytes, m.len().saturating_sub(1))?;
            for (k, v) in m {
                let Value::String(k) = k else {
                    return Err("configuration keys must be strings");
                };
                string_size(k, bytes)?;
                charge(bytes, 1)?;
                inspect(v, depth + 1, count, bytes)?;
            }
            Ok(())
        }
        Value::Tagged(_) => Err("configuration YAML tags unsupported"),
    }
}
fn convert(v: &Value) -> Json {
    match v {
        Value::Null => Json::Null,
        Value::Bool(b) => Json::Bool(*b),
        Value::Number(n) => n
            .as_i64()
            .map_or_else(|| Json::from(n.as_u64().expect("integer")), Json::from),
        Value::String(s) => Json::String(s.clone()),
        Value::Sequence(a) => Json::Array(a.iter().map(convert).collect()),
        Value::Mapping(m) => Json::Object(
            m.iter()
                .map(|(k, v)| (k.as_str().expect("string key").to_owned(), convert(v)))
                .collect(),
        ),
        Value::Tagged(_) => unreachable!("validated"),
    }
}
