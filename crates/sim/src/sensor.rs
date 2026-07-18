//! A deterministic depth sensor model.
//!
//! The sensor observes the plant's true depth and returns a *measured* depth,
//! modelling constant bias and finite resolution (quantization). It is
//! deliberately noise-free and deterministic: the same true depth always yields
//! the same measurement, so a closed-loop run is reproducible.

use crate::error::SimError;

/// Parameters of the depth sensor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SensorParams {
    /// A constant additive bias (m) applied to every reading. Must be finite.
    pub bias: f64,
    /// The measurement resolution (m): readings are rounded to the nearest
    /// multiple of this value. Zero disables quantization. Must be finite and
    /// `>= 0`.
    pub quantum: f64,
}

impl Default for SensorParams {
    /// An ideal sensor: no bias, no quantization.
    fn default() -> Self {
        Self {
            bias: 0.0,
            quantum: 0.0,
        }
    }
}

impl SensorParams {
    /// Validate the parameters.
    pub fn validate(&self) -> Result<(), SimError> {
        if !self.bias.is_finite() {
            return Err(SimError::InvalidParameter {
                name: "bias",
                reason: "must be a finite number".to_owned(),
            });
        }
        if !self.quantum.is_finite() || self.quantum < 0.0 {
            return Err(SimError::InvalidParameter {
                name: "quantum",
                reason: "must be a finite, non-negative number".to_owned(),
            });
        }
        Ok(())
    }
}

/// A depth sensor.
#[derive(Debug, Clone, Copy)]
pub struct DepthSensor {
    params: SensorParams,
}

impl DepthSensor {
    /// Create a sensor with validated parameters.
    pub fn new(params: SensorParams) -> Result<Self, SimError> {
        params.validate()?;
        Ok(Self { params })
    }

    /// An ideal (bias-free, full-resolution) sensor.
    pub fn ideal() -> Self {
        Self {
            params: SensorParams::default(),
        }
    }

    /// Observe a true depth, returning the measured depth.
    pub fn observe(&self, true_depth: f64) -> f64 {
        let measured = true_depth + self.params.bias;
        if self.params.quantum > 0.0 {
            (measured / self.params.quantum).round() * self.params.quantum
        } else {
            measured
        }
    }
}
