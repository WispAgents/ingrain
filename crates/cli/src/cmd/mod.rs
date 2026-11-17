//! One module per subcommand.

pub mod check;
pub mod decisions;
pub mod hook;
pub mod init;
pub mod rules;
pub mod view;

use std::process::ExitCode;

pub type Outcome = anyhow::Result<ExitCode>;
