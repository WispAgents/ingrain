//! Parser for `git diff -U0` output.

use crate::model::LineRange;

/// One file in a diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    /// Path before the change; `None` for added files.
    pub old_path: Option<String>,
    /// Path after the change; `None` for deleted files.
    pub new_path: Option<String>,
    pub hunks: Vec<Hunk>,
}

/// One hunk. Line numbers are 1-based. A side with `len == 0` is an anchor only:
/// git reports the line *before* the insertion/deletion point as `start`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_len: u32,
    pub new_start: u32,
    pub new_len: u32,
    pub removed: Vec<String>,
    pub added: Vec<String>,
}

impl Hunk {
    pub fn old_range(&self) -> LineRange {
        range(self.old_start, self.old_len)
    }
    pub fn new_range(&self) -> LineRange {
        range(self.new_start, self.new_len)
    }
}

fn range(start: u32, len: u32) -> LineRange {
    if len == 0 {
        LineRange::empty()
    } else {
        LineRange::new(start, start + len - 1)
    }
}

/// Parse unified diff text (any context size works, `-U0` is what we request).
pub fn parse(text: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut current: Option<FileDiff> = None;
    let mut hunk: Option<Hunk> = None;

    let flush_hunk = |file: &mut Option<FileDiff>, hunk: &mut Option<Hunk>| {
        if let (Some(f), Some(h)) = (file.as_mut(), hunk.take()) {
            f.hunks.push(h);
        }
    };

    for line in text.lines() {
        if line.starts_with("diff --git ") {
            flush_hunk(&mut current, &mut hunk);
            if let Some(f) = current.take() {
                files.push(f);
            }
            current = Some(FileDiff {
                old_path: None,
                new_path: None,
                hunks: Vec::new(),
            });
        } else if line.starts_with("@@") {
            flush_hunk(&mut current, &mut hunk);
            hunk = parse_header(line);
        } else if hunk.is_none() {
            // File header lines. Checked only outside hunks, because a removed
            // line such as `-- comment` also starts with "--- ".
            if let Some(f) = current.as_mut() {
                if let Some(rest) = line.strip_prefix("--- ") {
                    f.old_path = strip_side(rest, "a/");
                } else if let Some(rest) = line.strip_prefix("+++ ") {
                    f.new_path = strip_side(rest, "b/");
                }
            }
        } else if let Some(h) = hunk.as_mut() {
            if let Some(rest) = line.strip_prefix('+') {
                h.added.push(rest.to_owned());
            } else if let Some(rest) = line.strip_prefix('-') {
                h.removed.push(rest.to_owned());
            }
            // Context lines and "\ No newline at end of file" carry no change.
        }
    }
    flush_hunk(&mut current, &mut hunk);
    if let Some(f) = current.take() {
        files.push(f);
    }
    files
}

fn strip_side(path: &str, prefix: &str) -> Option<String> {
    let path = path.trim_end();
    if path == "/dev/null" {
        return None;
    }
    let path = unquote(path);
    Some(path.strip_prefix(prefix).unwrap_or(&path).to_owned())
}

/// Undo git's C-style quoting (`"a\tb\303\251"`), which it applies to paths with
/// special characters even when `core.quotePath` is off.
fn unquote(path: &str) -> String {
    let Some(inner) = path.strip_prefix('"').and_then(|p| p.strip_suffix('"')) else {
        return path.to_owned();
    };
    let mut bytes = Vec::with_capacity(inner.len());
    let mut chars = inner.bytes().peekable();
    while let Some(b) = chars.next() {
        if b != b'\\' {
            bytes.push(b);
            continue;
        }
        match chars.next() {
            Some(b'n') => bytes.push(b'\n'),
            Some(b't') => bytes.push(b'\t'),
            Some(b'r') => bytes.push(b'\r'),
            Some(d @ b'0'..=b'7') => {
                let mut value = u32::from(d - b'0');
                for _ in 0..2 {
                    match chars.peek() {
                        Some(&o @ b'0'..=b'7') => {
                            value = value * 8 + u32::from(o - b'0');
                            chars.next();
                        }
                        _ => break,
                    }
                }
                bytes.push(value as u8);
            }
            Some(other) => bytes.push(other),
            None => bytes.push(b'\\'),
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Parse `@@ -a,b +c,d @@ ...`.
fn parse_header(line: &str) -> Option<Hunk> {
    let inner = line.strip_prefix("@@ ")?;
    let end = inner.find(" @@")?;
    let mut parts = inner[..end].split(' ');
    let (old_start, old_len) = parse_side(parts.next()?.strip_prefix('-')?)?;
    let (new_start, new_len) = parse_side(parts.next()?.strip_prefix('+')?)?;
    Some(Hunk {
        old_start,
        old_len,
        new_start,
        new_len,
        removed: Vec::new(),
        added: Vec::new(),
    })
}

fn parse_side(s: &str) -> Option<(u32, u32)> {
    match s.split_once(',') {
        Some((start, len)) => Some((start.parse().ok()?, len.parse().ok()?)),
        None => Some((s.parse().ok()?, 1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "\
diff --git a/src/a.ts b/src/a.ts
index 1..2 100644
--- a/src/a.ts
+++ b/src/a.ts
@@ -3,2 +3 @@ function x() {
-  const r = await fetch(url);
-  return r.json();
+  return apiClient.get(url);
@@ -10,0 +10,2 @@
+// added
+// lines
diff --git a/new.py b/new.py
new file mode 100644
--- /dev/null
+++ b/new.py
@@ -0,0 +1 @@
+print('hi')
";

    #[test]
    fn unquotes_c_style_paths() {
        assert_eq!(unquote("\"b/\\303\\251/y.ts\""), "b/é/y.ts");
        assert_eq!(unquote("\"a/tab\\there\""), "a/tab\there");
        assert_eq!(unquote("b/plain.ts"), "b/plain.ts");
    }

    #[test]
    fn parses_files_and_hunks() {
        let files = parse(DIFF);
        assert_eq!(files.len(), 2);
        let a = &files[0];
        assert_eq!(a.new_path.as_deref(), Some("src/a.ts"));
        assert_eq!(a.hunks.len(), 2);
        assert_eq!(a.hunks[0].old_range(), LineRange::new(3, 4));
        assert_eq!(a.hunks[0].new_range(), LineRange::new(3, 3));
        assert_eq!(a.hunks[0].removed.len(), 2);
        assert!(a.hunks[1].old_range().is_empty());
        assert_eq!(a.hunks[1].new_range(), LineRange::new(10, 11));
        assert_eq!(files[1].old_path, None);
        assert_eq!(files[1].hunks[0].added, vec!["print('hi')"]);
    }
}
