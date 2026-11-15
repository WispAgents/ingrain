//! Salient tokens: the identifiers and keywords that describe what a change did.

use crate::lang::Lang;
use crate::rules::Parsed;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub text: String,
    /// 1-based line.
    pub line: u32,
    /// The token is the callee of a call (`fetch` in `fetch(x)`, `log` in `console.log(x)`).
    pub is_callee: bool,
    /// The tree-sitter node kind, e.g. `identifier`, `predefined_type`.
    pub kind: String,
}

/// Comparison operators whose swap is a common correction (`==` → `===`).
const OPERATORS: &[&str] = &["==", "!=", "===", "!=="];

/// Identifier-like leaves, keywords and a few operators, in source order.
/// Comments and string contents are excluded.
pub fn salient(lang: Lang, source: &str) -> Vec<Token> {
    let parsed = Parsed::new(lang, source);
    let root = parsed.grep().root();
    root.dfs()
        .filter(|node| node.is_leaf())
        .filter_map(|node| {
            let kind = token_kind(&node);
            let text = node.text();
            let keep = if node.is_named() || kind == "predefined_type" {
                kind.contains("identifier") || kind == "predefined_type"
            } else {
                is_keyword(&text) || OPERATORS.contains(&text.as_ref())
            };
            if !keep || in_string_or_comment(&node) {
                return None;
            }
            Some(Token {
                text: text.into_owned(),
                line: node.start_pos().line() as u32 + 1,
                is_callee: is_callee(&node),
                kind,
            })
        })
        .collect()
}

/// The node's kind, or for an anonymous token that is the only child of a named
/// node (`any` inside `predefined_type`), the parent's kind.
fn token_kind(node: &Node<'_>) -> String {
    if !node.is_named()
        && let Some(parent) = node.parent()
        && parent.is_named()
        && parent.children().len() == 1
    {
        return parent.kind().into_owned();
    }
    node.kind().into_owned()
}

fn is_keyword(text: &str) -> bool {
    text.len() >= 2 && text.chars().all(|c| c.is_ascii_lowercase())
}

type Node<'a> = ast_grep_core::Node<'a, ast_grep_core::tree_sitter::StrDoc<Lang>>;

fn in_string_or_comment(node: &Node<'_>) -> bool {
    node.ancestors().any(|a| {
        let kind = a.kind();
        kind.contains("string") || kind.contains("comment") || kind.contains("template")
    })
}

/// The leaf names the function being called: either the whole callee
/// (`fetch(...)`) or the last member of it (`console.log(...)`).
fn is_callee(node: &Node<'_>) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    let is_call = |kind: &str| kind == "call_expression" || kind == "call";
    if is_call(&parent.kind()) {
        return parent.child(0).is_some_and(|c| c.range() == node.range());
    }
    let is_member = matches!(
        parent.kind().as_ref(),
        "member_expression" | "attribute" | "selector_expression" | "field_expression"
    );
    if !is_member {
        return false;
    }
    let last_child = parent.children().last();
    let is_last = last_child.is_some_and(|c| c.range() == node.range());
    is_last
        && parent.parent().is_some_and(|g| {
            is_call(&g.kind()) && g.child(0).is_some_and(|c| c.range() == parent.range())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ast_grep_language::SupportLang;

    fn texts(tokens: &[Token]) -> Vec<&str> {
        tokens.iter().map(|t| t.text.as_str()).collect()
    }

    #[test]
    fn typescript_tokens() {
        let src =
            "const r = await fetch(url); // fetch it\nlet x: any = 'fetch';\nconsole.log(r == x);";
        let tokens = salient(SupportLang::TypeScript, src);
        let t = texts(&tokens);
        assert!(t.contains(&"fetch"));
        assert!(t.contains(&"any"));
        assert!(t.contains(&"=="));
        assert!(t.contains(&"const"));
        // From the comment and the string: excluded.
        assert_eq!(t.iter().filter(|x| **x == "fetch").count(), 1);
        let fetch = tokens.iter().find(|t| t.text == "fetch").unwrap();
        assert!(fetch.is_callee);
        let log = tokens.iter().find(|t| t.text == "log").unwrap();
        assert!(log.is_callee);
        assert_eq!(log.line, 3);
        let console = tokens.iter().find(|t| t.text == "console").unwrap();
        assert!(!console.is_callee);
    }

    #[test]
    fn python_tokens() {
        let tokens = salient(SupportLang::Python, "import os\nprint(os.getenv('X'))\n");
        let getenv = tokens.iter().find(|t| t.text == "getenv").unwrap();
        assert!(getenv.is_callee);
        assert!(tokens.iter().find(|t| t.text == "print").unwrap().is_callee);
    }
}
