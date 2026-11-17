//! Shared building blocks for ingrain: the data model, rule files, store layout and git access.

pub mod config;
pub mod diff;
pub mod git;
pub mod layout;
pub mod model;
pub mod process;
pub mod rulefile;
pub mod store;

pub use config::Config;
pub use git::{DiffBase, Git};
pub use layout::{Layout, Store};
pub use model::{AgentHunk, CorrectedLines, Correction, LineRange, Snippet, Source};
pub use rulefile::{Backtest, Example, IngrainMeta, Mode, RuleFile};
