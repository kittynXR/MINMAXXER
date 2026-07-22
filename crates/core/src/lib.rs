//! Pure Ecliptica log parsing and combat aggregation.
//!
//! This crate deliberately has no UI, database, network, or Windows dependencies so the
//! parser can be tested against captured log fixtures and reused by import tools.

pub mod aggregate;
pub mod model;
pub mod parser;

pub use aggregate::{analyze_runs, events_for_run, CombatEngine, EngineSnapshot};
pub use model::*;
pub use parser::{parse_log_line, EclipticaParser, ParseOutcome};
