//! Telling agent commits from human ones.

/// Built-in markers, matched case-insensitively against the commit message,
/// author name and author email.
const BUILTIN_MARKERS: &[&str] = &[
    "co-authored-by: claude",
    "generated with [claude code]",
    "generated with claude code",
    "noreply@anthropic.com",
    "claude[bot]",
    "co-authored-by: codex",
    "chatgpt-codex-connector",
    "co-authored-by: cursor",
    "cursoragent",
    "cursor agent",
    "copilot-swe-agent",
    "co-authored-by: copilot",
    "devin-ai-integration",
    "google-labs-jules",
    "co-authored-by: gemini",
    "co-authored-by: openhands",
    "openhands@all-hands.dev",
];

/// Automation that is neither an agent nor a human: dependency and CI bots.
const NON_HUMAN: &[&str] = &[
    "[bot]",
    "dependabot",
    "renovate",
    "github-actions",
    "pre-commit-ci",
    "greenkeeper",
    "semantic-release",
];

/// Who wrote a commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Author {
    Agent,
    Human,
    Bot,
}

#[derive(Debug, Clone, Default)]
pub struct Classifier {
    extra: Vec<String>,
}

impl Classifier {
    /// `extra` markers come from `.corrections/config.yml`.
    pub fn new(extra: &[String]) -> Self {
        Self {
            extra: extra.iter().map(|m| m.to_lowercase()).collect(),
        }
    }

    pub fn classify(&self, message: &str, author_name: &str, author_email: &str) -> Author {
        let haystack = format!("{message}\n{author_name}\n{author_email}").to_lowercase();
        let is_agent = BUILTIN_MARKERS.iter().any(|m| haystack.contains(m))
            || self.extra.iter().any(|m| haystack.contains(m.as_str()));
        if is_agent {
            return Author::Agent;
        }
        let who = format!("{author_name}\n{author_email}").to_lowercase();
        if NON_HUMAN.iter().any(|m| who.contains(m)) {
            Author::Bot
        } else {
            Author::Human
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies() {
        let c = Classifier::new(&["my-agent".into()]);
        let agent_msg = "fix: x\n\nCo-Authored-By: Claude <noreply@anthropic.com>";
        assert_eq!(c.classify(agent_msg, "Ana", "ana@x.dev"), Author::Agent);
        assert_eq!(
            c.classify(
                "feat",
                "devin-ai-integration[bot]",
                "158243242+devin-ai-integration[bot]@users.noreply.github.com"
            ),
            Author::Agent
        );
        assert_eq!(c.classify("bump", "dependabot[bot]", "x"), Author::Bot);
        assert_eq!(c.classify("fix typo", "Ana", "ana@x.dev"), Author::Human);
        assert_eq!(c.classify("via my-agent", "Ana", "a"), Author::Agent);
    }
}
