//! `ingrain rules …`: lint rules learned from the corrections people make to agent code.

pub mod eval;
pub mod mine;
pub mod modes;
pub mod propose;
pub mod review;

use crate::cmd::Outcome;
use clap::Subcommand;
use std::path::Path;

#[derive(Subcommand)]
pub enum Command {
    /// Capture corrections from git history, PR review suggestions and agent transcripts.
    Mine(mine::Args),
    /// Cluster corrections and write backtested rule proposals to .corrections/pending/.
    Propose(propose::Args),
    /// Go through proposed rules one by one: accept, edit, reject or skip.
    Review,
    /// Accept proposed rules (moves them to .corrections/rules/ and stages them).
    Accept {
        #[arg(required = true)]
        ids: Vec<String>,
    },
    /// Reject a proposed rule so it is never proposed again.
    Reject {
        id: String,
        #[arg(long)]
        reason: String,
    },
    /// List accepted and proposed rules with their mode and hit counts.
    List,
    /// Set a rule's mode by hand.
    Mode {
        id: String,
        mode: ingrain_core::Mode,
    },
    /// Suggest (or apply) promotions and demotions from the hit log.
    Promote {
        #[arg(long)]
        apply: bool,
    },
    /// Time-split evaluation: learn from older corrections, score on newer ones.
    Eval(eval::Args),
}

pub fn run(dir: &Path, command: Command) -> Outcome {
    match command {
        Command::Mine(args) => mine::run(dir, args),
        Command::Propose(args) => propose::run(dir, args),
        Command::Review => review::interactive(dir),
        Command::Accept { ids } => review::accept(dir, &ids),
        Command::Reject { id, reason } => review::reject(dir, &id, &reason),
        Command::List => modes::list(dir),
        Command::Mode { id, mode } => modes::set_mode(dir, &id, mode),
        Command::Promote { apply } => modes::promote(dir, apply),
        Command::Eval(args) => eval::run(dir, args),
    }
}
