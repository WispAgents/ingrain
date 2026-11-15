//! One module per subcommand.

pub mod check;
pub mod decisions;
pub mod eval;
pub mod hook;
pub mod init;
pub mod mine;
pub mod propose;
pub mod review;
pub mod rules;

use std::process::ExitCode;

pub type Outcome = anyhow::Result<ExitCode>;
