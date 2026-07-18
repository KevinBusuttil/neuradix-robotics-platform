//! The `version` command service.

use serde_json::json;

use neuradix_contracts::SUPPORTED_API_VERSION;

use crate::app::{AppError, Outcome};

/// `neuradix version`
pub fn run() -> Result<Outcome, AppError> {
    Ok(Outcome::new(json!({
        "name": "neuradix",
        "version": env!("CARGO_PKG_VERSION"),
        "contractApiVersion": SUPPORTED_API_VERSION,
    })))
}
