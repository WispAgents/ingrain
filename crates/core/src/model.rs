//! Data captured from repositories: corrections and the agent hunks they apply to.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A slice of a file with a marked range of changed lines.
///
/// `text` holds the changed lines plus some surrounding context so it parses as code.
/// `changed` is a 1-based inclusive line range relative to `text`. An empty range
/// (`start > end`) means nothing changed on this side, e.g. a pure deletion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snippet {
    pub text: String,
    pub changed: LineRange,
    /// Line number in the original file of the first line of `text` (1-based).
    pub first_line: u32,
}

impl Snippet {
    /// The changed lines only.
    pub fn changed_text(&self) -> String {
        if self.changed.is_empty() {
            return String::new();
        }
        self.text
            .lines()
            .skip(self.changed.start as usize - 1)
            .take(self.changed.len() as usize)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// A 1-based inclusive line range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineRange {
    pub start: u32,
    pub end: u32,
}

impl LineRange {
    pub fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }
    pub fn empty() -> Self {
        Self { start: 1, end: 0 }
    }
    pub fn is_empty(&self) -> bool {
        self.start > self.end
    }
    pub fn len(&self) -> u32 {
        if self.is_empty() {
            0
        } else {
            self.end - self.start + 1
        }
    }
    pub fn contains(&self, line: u32) -> bool {
        line >= self.start && line <= self.end
    }
    /// True if the two ranges share at least one line.
    pub fn overlaps(&self, other: &LineRange) -> bool {
        !self.is_empty() && !other.is_empty() && self.start <= other.end && other.start <= self.end
    }
}

/// Where a correction came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// A human commit rewrote lines an agent commit added.
    GitFixup {
        agent_commit: String,
        fix_commit: String,
    },
    /// A reviewer left a ```suggestion block on an agent PR.
    ReviewSuggestion { url: String },
    /// A human changed what an agent wrote in a local session before committing.
    Transcript { session: String },
}

/// One human correction of agent-written code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Correction {
    pub id: String,
    pub path: String,
    /// ast-grep language name, e.g. `TypeScript`.
    pub lang: String,
    pub before: Snippet,
    pub after: Snippet,
    pub source: Source,
    /// Unix time the agent wrote the code.
    pub agent_time: i64,
    /// Unix time of the correction.
    pub time: i64,
    /// Review comment or commit subject explaining the change, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Correction {
    /// Stable id derived from the content, so re-mining produces the same ids.
    pub fn make_id(path: &str, before: &Snippet, after: &Snippet, source: &Source) -> String {
        let source_key = match source {
            Source::GitFixup { fix_commit, .. } => fix_commit.as_str(),
            Source::ReviewSuggestion { url } => url.as_str(),
            Source::Transcript { session } => session.as_str(),
        };
        short_hash(&[
            path,
            &before.changed_text(),
            &after.changed_text(),
            source_key,
        ])
    }
}

/// Lines an agent commit added. This is the population precision is measured on:
/// a rule hit on `corrected` lines is a true positive, a hit anywhere else in the
/// hunk is on code humans kept, so it counts as a false alarm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentHunk {
    pub commit: String,
    pub path: String,
    pub lang: String,
    pub time: i64,
    /// `changed` is the range the agent added.
    pub snippet: Snippet,
    /// Lines (relative to `snippet.text`) that a human later corrected.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub corrected: Vec<CorrectedLines>,
}

/// A corrected range inside an agent hunk, and when it was corrected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorrectedLines {
    pub lines: LineRange,
    pub time: i64,
    pub correction: String,
}

impl AgentHunk {
    /// Corrected ranges known at `time` (corrections made before it).
    pub fn corrected_before(&self, time: i64) -> impl Iterator<Item = &LineRange> {
        self.corrected
            .iter()
            .filter(move |c| c.time < time)
            .map(|c| &c.lines)
    }
    pub fn corrected_from(&self, time: i64) -> impl Iterator<Item = &LineRange> {
        self.corrected
            .iter()
            .filter(move |c| c.time >= time)
            .map(|c| &c.lines)
    }
}

/// First 12 hex chars of a SHA-256 over the parts, separated by NUL.
pub fn short_hash(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0u8]);
    }
    let digest = hasher.finalize();
    digest.iter().take(6).map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_range_overlap() {
        let a = LineRange::new(3, 5);
        assert!(a.overlaps(&LineRange::new(5, 9)));
        assert!(!a.overlaps(&LineRange::new(6, 9)));
        assert!(!a.overlaps(&LineRange::empty()));
        assert_eq!(LineRange::empty().len(), 0);
    }

    #[test]
    fn changed_text_extracts_range() {
        let s = Snippet {
            text: "a\nb\nc\nd".into(),
            changed: LineRange::new(2, 3),
            first_line: 10,
        };
        assert_eq!(s.changed_text(), "b\nc");
    }

    #[test]
    fn ids_are_stable() {
        let s = Snippet {
            text: "x".into(),
            changed: LineRange::new(1, 1),
            first_line: 1,
        };
        let src = Source::GitFixup {
            agent_commit: "a".into(),
            fix_commit: "b".into(),
        };
        assert_eq!(
            Correction::make_id("p", &s, &s, &src),
            Correction::make_id("p", &s, &s, &src)
        );
        assert_eq!(Correction::make_id("p", &s, &s, &src).len(), 12);
    }
}
