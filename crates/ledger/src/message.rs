//! Text shown to coding agents.

use crate::check::Violation;
use crate::record::Record;

/// Decisions that apply to a file the agent is about to edit.
pub fn scoped(path: &str, records: &[&Record], more: usize) -> String {
    let mut s = format!(
        "Project decisions that apply to `{path}` (from the repo's decision ledger; follow them):\n"
    );
    for r in records {
        s.push_str(&format!("- {}\n", r.line()));
    }
    if more > 0 {
        s.push_str(&format!(
            "({more} more: run `ingrain decisions for {path}`)\n"
        ));
    }
    s.push_str(HOW_TO_CHANGE);
    s
}

/// Repo-wide decisions at session start.
pub fn session(records: &[&Record], scoped_count: usize, more: usize) -> String {
    let mut s = String::from(
        "This repository keeps a decision ledger (`decisions/`). Repo-wide decisions:\n",
    );
    for r in records {
        s.push_str(&format!("- {}\n", r.line()));
    }
    if more > 0 {
        s.push_str(&format!("({more} more: run `ingrain decisions list`)\n"));
    }
    if scoped_count > 0 {
        s.push_str(&format!("{scoped_count} more decisions apply to specific files; they're shown when you edit those files.\n"));
    }
    s.push_str(
        "When the user states a lasting decision (a convention, constraint, or choice future code must follow), \
         record it for review: `ingrain decisions add --propose \"<decision>\" --reason \"<why>\" --scope <glob>`.\n",
    );
    s.push_str(HOW_TO_CHANGE);
    s
}

/// Why an edit or a turn was refused.
pub fn violations(found: &[&Violation], records: &[Record], blocking_edit: bool) -> String {
    let mut s = String::from(if blocking_edit {
        "This edit breaks accepted project decisions, so it wasn't applied:\n"
    } else {
        "Your changes break accepted project decisions:\n"
    });
    for v in found {
        let line = records
            .iter()
            .find(|r| r.id == v.decision)
            .map(Record::line)
            .unwrap_or_else(|| v.decision.clone());
        s.push_str(&format!(
            "- {}:{} {}: `{}`\n  {}\n",
            v.path, v.line, v.what, v.code, line
        ));
    }
    s.push_str(
        "\nRewrite the code to follow the decision. If the code is right as written, keep it and add a comment \
         `decisions-ignore <id>: <reason>` on that line; the person is asked to approve it.\n",
    );
    s.push_str(HOW_TO_CHANGE);
    s
}

const HOW_TO_CHANGE: &str = "If a decision itself should change, don't work around it: propose the change with \
`ingrain decisions supersede <id> --decision \"<new decision>\" --reason \"<why>\"` and tell the user. A human accepts it.\n";
