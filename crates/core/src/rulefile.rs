//! Rule files: plain ast-grep YAML with an `ingrain` block under `metadata`.
//!
//! ```yaml
//! id: no-raw-fetch
//! language: TypeScript
//! rule: { pattern: "fetch($$$ARGS)" }
//! message: Use apiClient instead of fetch
//! metadata:
//!   ingrain:
//!     mode: shadow
//!     support: 6
//! ```

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// How a rule reacts to a hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Log the hit only.
    #[default]
    Shadow,
    /// Tell the agent, let the edit stand.
    Warn,
    /// Make the agent fix it; fail CI.
    Block,
}

impl Mode {
    pub fn next(self) -> Option<Mode> {
        match self {
            Mode::Shadow => Some(Mode::Warn),
            Mode::Warn => Some(Mode::Block),
            Mode::Block => None,
        }
    }
    pub fn prev(self) -> Option<Mode> {
        match self {
            Mode::Shadow => None,
            Mode::Warn => Some(Mode::Shadow),
            Mode::Block => Some(Mode::Warn),
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Mode::Shadow => "shadow",
            Mode::Warn => "warn",
            Mode::Block => "block",
        })
    }
}

impl FromStr for Mode {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "shadow" => Ok(Mode::Shadow),
            "warn" => Ok(Mode::Warn),
            "block" => Ok(Mode::Block),
            other => bail!("unknown mode `{other}` (expected shadow, warn or block)"),
        }
    }
}

/// Backtest numbers recorded when the rule was proposed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Backtest {
    /// Corrections in the cluster whose agent code the rule flags.
    pub before_hits: u32,
    pub befores: u32,
    /// Corrected versions the rule (wrongly) flags. Must be 0.
    pub after_hits: u32,
    pub afters: u32,
    /// Agent hunks that humans kept and the rule flags (likely false alarms).
    pub kept_hits: u32,
    pub kept: u32,
    /// Matches in the current tree when proposed (existing code; never blocks).
    #[serde(default)]
    pub head_hits: u32,
}

/// The `metadata.ingrain` block.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngrainMeta {
    #[serde(default)]
    pub mode: Mode,
    /// Number of corrections behind the rule.
    #[serde(default)]
    pub support: u32,
    /// `template` or `llm`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub origin: String,
    /// Human-readable pointers to the corrections (commit, PR comment, path).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backtest: Option<Backtest>,
    /// The fix humans applied, shown to the agent next to the message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub example: Option<Example>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Example {
    pub before: String,
    pub after: String,
}

/// A rule file on disk. The full YAML document is kept so unknown ast-grep
/// keys survive a round trip.
#[derive(Debug, Clone)]
pub struct RuleFile {
    pub path: PathBuf,
    pub id: String,
    pub language: String,
    pub message: String,
    pub meta: IngrainMeta,
    doc: Mapping,
}

impl RuleFile {
    /// Build a rule from an ast-grep YAML string plus ingrain metadata.
    pub fn from_parts(yaml: &str, meta: IngrainMeta) -> Result<Self> {
        let mut rule = Self::parse(yaml, PathBuf::new())?;
        rule.meta = meta;
        rule.sync_meta();
        Ok(rule)
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading rule {}", path.display()))?;
        Self::parse(&text, path.to_path_buf())
            .with_context(|| format!("parsing {}", path.display()))
    }

    pub fn parse(yaml: &str, path: PathBuf) -> Result<Self> {
        let doc: Mapping = serde_yaml::from_str(yaml).context("rule is not a YAML mapping")?;
        let get_str = |key: &str| {
            doc.get(Value::from(key))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        let id = get_str("id").context("rule has no `id`")?;
        let language = get_str("language").context("rule has no `language`")?;
        let message = get_str("message").unwrap_or_default();
        let meta = doc
            .get(Value::from("metadata"))
            .and_then(|m| m.get("ingrain"))
            .cloned()
            .map(serde_yaml::from_value)
            .transpose()
            .context("invalid metadata.ingrain block")?
            .unwrap_or_default();
        Ok(Self {
            path,
            id,
            language,
            message,
            meta,
            doc,
        })
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.meta.mode = mode;
        self.sync_meta();
    }

    /// Change the rule id (also renames the file on next save in the same dir).
    pub fn set_id(&mut self, id: &str) {
        self.id = id.to_owned();
        self.doc.insert(Value::from("id"), Value::from(id));
    }

    /// The YAML document, including metadata. Valid ast-grep rule syntax.
    pub fn to_yaml(&self) -> String {
        serde_yaml::to_string(&self.doc).expect("a mapping always serializes")
    }

    pub fn save(&mut self, path: &Path) -> Result<()> {
        self.sync_meta();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_yaml())
            .with_context(|| format!("writing {}", path.display()))?;
        self.path = path.to_path_buf();
        Ok(())
    }

    fn sync_meta(&mut self) {
        let meta = serde_yaml::to_value(&self.meta).expect("metadata serializes");
        let key = Value::from("metadata");
        let mut metadata = self
            .doc
            .get(&key)
            .and_then(Value::as_mapping)
            .cloned()
            .unwrap_or_default();
        metadata.insert(Value::from("ingrain"), meta);
        self.doc.insert(key, Value::Mapping(metadata));
    }
}

/// Load every `*.yml` / `*.yaml` rule in a directory, sorted by id.
pub fn load_dir(dir: &Path) -> Result<Vec<RuleFile>> {
    let mut rules = Vec::new();
    if !dir.exists() {
        return Ok(rules);
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let is_yaml = matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("yml" | "yaml")
        );
        if is_yaml {
            rules.push(RuleFile::load(&path)?);
        }
    }
    rules.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(rules)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULE: &str =
        "id: no-fetch\nlanguage: TypeScript\nrule:\n  pattern: fetch($$$A)\nmessage: use api\n";

    #[test]
    fn round_trip_keeps_rule_and_meta() {
        let dir = tempfile::tempdir().unwrap();
        let meta = IngrainMeta {
            support: 4,
            origin: "template".into(),
            ..Default::default()
        };
        let mut rule = RuleFile::from_parts(RULE, meta).unwrap();
        rule.set_mode(Mode::Warn);
        let path = dir.path().join("no-fetch.yml");
        rule.save(&path).unwrap();

        let loaded = RuleFile::load(&path).unwrap();
        assert_eq!(loaded.id, "no-fetch");
        assert_eq!(loaded.meta.mode, Mode::Warn);
        assert_eq!(loaded.meta.support, 4);
        assert!(loaded.to_yaml().contains("pattern: fetch($$$A)"));
    }

    #[test]
    fn missing_meta_defaults_to_shadow() {
        let rule = RuleFile::parse(RULE, PathBuf::new()).unwrap();
        assert_eq!(rule.meta.mode, Mode::Shadow);
    }

    #[test]
    fn mode_ladder() {
        assert_eq!(Mode::Shadow.next(), Some(Mode::Warn));
        assert_eq!(Mode::Block.next(), None);
        assert_eq!("block".parse::<Mode>().unwrap(), Mode::Block);
        assert!("loud".parse::<Mode>().is_err());
    }
}
