//! Development-only access to the faithful evaluator and its run observations.
//!
//! `model` scopes a faithful session, `lower` turns its processed
//! specification into the native IR, `checker` drives it as a transition
//! system, `values` converts values both ways, and `observed` runs the
//! faithful checker end to end and parses what it reports.

pub mod checker;
pub mod lower;
pub mod model;
pub mod observed;
pub mod values;

pub use model::{Model, ModelError, with_loaded_model, with_model, with_register_model};
pub use observed::{ObserveError, from_outcome, observe_faithful, run_faithful};
pub use tlaplus_oracle::ModelRequest;
pub use tlaplus_oracle::{Answers, Comparison, Difference, Mode, Outcome, compare, corpus, tool};
pub use tlaplus_oracle::{ErrorCategory, ObservedRun, PropertyKind, TraceEnd, Verdict};
pub use tla_tlc::args::{RunMode as FaithfulRunMode, TlcArgs as FaithfulArgs};
pub use tla_tlc::{RunEnv, RunOutcome};
pub use tla_tlc_eval::mp as recording;
