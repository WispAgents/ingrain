//! The decision ledger: reviewed project decisions in `decisions/`, scoped to files, shown to
//! coding agents before they edit, and checked on added lines.

pub mod check;
pub mod extract;
pub mod manifest;
pub mod message;
pub mod record;
pub mod scope;

pub use check::{Checker, Target, Violation};
pub use record::{Ledger, Record, Status};
