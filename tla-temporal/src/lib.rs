//! Liveness checking of a bound model: temporal templates are instantiated
//! against the model's constants, and behavior graphs are built from and
//! validated against exploration reports.
pub mod behavior;
mod error;
mod instantiate;

pub use error::Error;
pub use instantiate::{Instantiated, instantiate};
