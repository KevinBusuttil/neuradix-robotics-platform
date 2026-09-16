//! Offline delay declarations, not runtime buffering or scheduling enforcement.
use serde_yaml::Value;

/// Validated connection delay. Default is instantaneous; positive delays require
/// one shared logical evaluation schedule and trusted seed history before tick 0.
///
/// ```compile_fail
/// let delay = neuradix_graph::ConnectionDelay { ticks: 0 };
/// ```
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConnectionDelay {
    ticks: u16,
}
impl ConnectionDelay {
    /// Maximum positive delay per edge, inclusive.
    pub const MAX_TICKS: u64 = 1024;
    /// Maximum sum of declared history slots per graph, inclusive.
    pub const MAX_TOTAL_TICKS: u64 = 65_536;

    /// Validate a positive delay declaration. Units and initialization policy
    /// are explicit; there is no milliseconds conversion or implicit zero seed.
    pub fn new(ticks: u64, unit: &str, initialization: &str) -> Result<Self, &'static str> {
        if !(1..=Self::MAX_TICKS).contains(&ticks) {
            return Err("delay ticks must be in 1..=1024");
        }
        if unit != "evaluation-ticks" {
            return Err("delay unit must be evaluation-ticks");
        }
        if initialization != "require-seed" {
            return Err("delay initialization must be require-seed");
        }
        Ok(Self {
            ticks: ticks as u16,
        })
    }
    /// Zero denotes an instantaneous edge. Positive values select earlier ticks.
    pub fn ticks(self) -> u16 {
        self.ticks
    }
    /// Whether same-tick dependency analysis must include this edge.
    pub fn is_instantaneous(self) -> bool {
        self.ticks == 0
    }
    /// Decode a raw declaration; explicit null, tags, floats and unknown keys fail.
    pub fn from_value(value: &Value) -> Result<Self, &'static str> {
        if matches!(value, Value::String(s) if s == "instantaneous") {
            return Ok(Self::default());
        }
        let Value::Mapping(map) = value else {
            return Err("delay must be instantaneous or an explicit mapping");
        };
        if map.values().any(|v| matches!(v, Value::Tagged(_))) {
            return Err("tagged delay values are unsupported");
        }
        if map.len() != 3 {
            return Err("delay requires exactly ticks, unit and initialization");
        }
        let ticks = map
            .get(Value::String("ticks".to_owned()))
            .and_then(Value::as_u64)
            .ok_or("delay ticks must be an unsigned integer")?;
        let unit = map
            .get(Value::String("unit".to_owned()))
            .and_then(Value::as_str)
            .ok_or("delay unit is required")?;
        let initialization = map
            .get(Value::String("initialization".to_owned()))
            .and_then(Value::as_str)
            .ok_or("delay initialization is required")?;
        Self::new(ticks, unit, initialization)
    }
}

pub(crate) fn instantaneous() -> Value {
    Value::String("instantaneous".to_owned())
}
