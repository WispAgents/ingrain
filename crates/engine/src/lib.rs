//! Matching engine for ingrain, built on ast-grep: language detection,
//! rule compilation and scanning, and salient tokens for clustering.

pub mod lang;
pub mod rules;
pub mod tokens;

pub use lang::Lang;
pub use rules::{CompiledRule, Hit, Parsed};
pub use tokens::Token;
