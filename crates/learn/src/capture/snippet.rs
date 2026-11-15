//! Cutting snippets with context out of file contents.

use ingrain_core::{LineRange, Snippet};

/// Lines of context kept around changed lines.
pub const CONTEXT: u32 = 8;

/// Snippet around `changed` (1-based, file coordinates). For an empty range,
/// `anchor` is the line after which the change happened.
pub fn cut(content: &str, changed: LineRange, anchor: u32, context: u32) -> Snippet {
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len() as u32;
    let (lo, hi) = if changed.is_empty() {
        (anchor.max(1), anchor)
    } else {
        (changed.start, changed.end.min(total.max(1)))
    };
    let start = lo.saturating_sub(context).max(1);
    let end = (hi + context).min(total);
    let text = if total == 0 || start > end {
        String::new()
    } else {
        lines[(start - 1) as usize..end as usize].join("\n")
    };
    let rel = if changed.is_empty() {
        LineRange::empty()
    } else {
        LineRange::new(changed.start - start + 1, hi - start + 1)
    };
    Snippet {
        text,
        changed: rel,
        first_line: start,
    }
}

/// A snippet that is entirely changed (no surrounding file known).
pub fn whole(text: &str) -> Snippet {
    let n = text.lines().count() as u32;
    Snippet {
        text: text.to_owned(),
        changed: if n == 0 {
            LineRange::empty()
        } else {
            LineRange::new(1, n)
        },
        first_line: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuts_with_context() {
        let content = (1..=20)
            .map(|i| format!("l{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let s = cut(&content, LineRange::new(10, 11), 0, 2);
        assert_eq!(s.first_line, 8);
        assert_eq!(s.changed, LineRange::new(3, 4));
        assert_eq!(s.changed_text(), "l10\nl11");
        let edge = cut(&content, LineRange::new(1, 1), 0, 3);
        assert_eq!(edge.first_line, 1);
        assert_eq!(edge.changed, LineRange::new(1, 1));
        let empty = cut(&content, LineRange::empty(), 5, 1);
        assert!(empty.changed.is_empty());
        assert_eq!(empty.text, "l4\nl5\nl6");
    }
}
