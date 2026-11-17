//! The viewer's small visual vocabulary: category and mode colours, chips, section labels,
//! keycaps. Everything reads the kit, so light and dark both work.

use ingrain_core::{Mode, Store};
use ingrain_ledger::record::Category;
use rune::{Color, El, TextStyle, col, el, row, text};
use rune_kit::Kit;
use rune_theme_opencode::scale;

pub const DISPLAY: TextStyle = TextStyle::new(22.0, 600.0).lh(29.0).tracking(-0.3);
pub const TITLE: TextStyle = TextStyle::new(15.0, 600.0).lh(21.0);
pub const LEAD: TextStyle = TextStyle::new(14.0, 400.0).lh(21.0);
pub const ROW: TextStyle = TextStyle::new(13.0, 500.0).lh(18.0);
pub const LABEL: TextStyle = TextStyle::new(10.5, 600.0).tracking(0.6);
pub const MONO: TextStyle = TextStyle::new(11.5, 500.0).mono();
pub const MONO_SMALL: TextStyle = TextStyle::new(10.5, 500.0).mono();
pub const SMALL: TextStyle = TextStyle::new(11.5, 400.0).lh(15.0);
pub const BIG_NUMBER: TextStyle = TextStyle::new(20.0, 600.0).features("'tnum' 1");

pub fn dark(k: &Kit) -> bool {
    k.dark
}

fn hue(category: Category) -> &'static str {
    match category {
        Category::Architecture => "lilac",
        Category::Constraint => "coral",
        Category::Convention => "mint",
        Category::ApiChange => "amber",
        Category::Scope => "cobalt",
    }
}

/// The category's mark colour, and its chip's background and ink.
pub fn category_colors(k: &Kit, category: Category) -> (Color, Color, Color) {
    let d = dark(k);
    let h = hue(category);
    (scale(h, d, 9), scale(h, d, 3), scale(h, d, 11))
}

pub fn mode_colors(k: &Kit, mode: Mode) -> (Color, Color, Color) {
    let d = dark(k);
    let h = match mode {
        Mode::Shadow => "smoke",
        Mode::Warn => "amber",
        Mode::Block => "coral",
    };
    (scale(h, d, 9), scale(h, d, 3), scale(h, d, 11))
}

pub fn store_colors(k: &Kit, store: Store) -> (Color, Color) {
    let d = dark(k);
    let h = match store {
        Store::Project => "smoke",
        Store::Local => "yuzu",
        Store::Global => "cobalt",
    };
    (scale(h, d, 3), scale(h, d, 11))
}

pub fn success(k: &Kit) -> Color {
    scale("mint", dark(k), 9)
}

pub fn danger(k: &Kit) -> Color {
    scale("coral", dark(k), 9)
}

pub fn dot(color: Color, size: f32) -> El {
    el().size(size, size).radius(size / 2.0).fill(color)
}

pub fn ring(color: Color, size: f32) -> El {
    el().size(size, size).radius(size / 2.0).border(1.5, color)
}

pub fn chip(label: impl Into<String>, bg: Color, fg: Color) -> El {
    row()
        .px(7.0)
        .py(2.0)
        .radius(5.0)
        .fill(bg)
        .child(text(label.into()).font(SMALL.lh(14.0)).color(fg))
}

pub fn mono_chip(k: &Kit, label: impl Into<String>) -> El {
    row()
        .px(6.0)
        .py(2.0)
        .radius(5.0)
        .fill(k.c.layer_02)
        .border(1.0, k.c.border_weak)
        .child(text(label.into()).font(MONO_SMALL).color(k.c.text_muted))
}

pub fn category_chip(k: &Kit, category: Category) -> El {
    let (mark, bg, fg) = category_colors(k, category);
    row()
        .gap(5.0)
        .center_y()
        .px(7.0)
        .py(2.0)
        .radius(5.0)
        .fill(bg)
        .child(dot(mark, 6.0))
        .child(
            text(crate::viewer::model::category_name(category))
                .font(SMALL.lh(14.0))
                .color(fg),
        )
}

pub fn store_chip(k: &Kit, store: Store) -> Option<El> {
    if store == Store::Project {
        return None;
    }
    let (bg, fg) = store_colors(k, store);
    Some(chip(store.name(), bg, fg))
}

pub fn mode_chip(k: &Kit, mode: Mode) -> El {
    let (mark, bg, fg) = mode_colors(k, mode);
    row()
        .gap(5.0)
        .center_y()
        .px(7.0)
        .py(2.0)
        .radius(5.0)
        .fill(bg)
        .child(dot(mark, 6.0))
        .child(text(mode.to_string()).font(SMALL.lh(14.0)).color(fg))
}

/// An uppercase label over a section.
pub fn section_label(k: &Kit, title: &str) -> El {
    text(title.to_uppercase()).font(LABEL).color(k.c.text_faint)
}

pub fn section(k: &Kit, title: &str, body: El) -> El {
    col()
        .w_full()
        .gap(8.0)
        .child(section_label(k, title))
        .child(body)
}

pub fn keycap(k: &Kit, key: &str) -> El {
    row()
        .px(5.0)
        .min_w(18.0)
        .h(18.0)
        .center()
        .radius(4.0)
        .fill(k.c.layer_02)
        .border(1.0, k.c.border_base)
        .child(text(key.to_owned()).font(MONO_SMALL).color(k.c.text_muted))
}

pub fn hairline(k: &Kit) -> El {
    el().w_full().h(1.0).fill(k.c.border_weak)
}

/// A labelled number, for stats rows.
pub fn stat(k: &Kit, value: String, label: &str, ink: Color) -> El {
    col()
        .gap(2.0)
        .child(text(value).font(BIG_NUMBER).color(ink))
        .child(text(label.to_owned()).font(SMALL).color(k.c.text_faint))
}

pub fn wrap_row() -> El {
    row().w_full().flex_wrap().gap(6.0)
}

/// Text with `inline code` drawn as code: backticks dropped, the code in mono on a soft
/// background. Returns the text shown (for strike ranges) and the element.
pub fn prose(k: &Kit, src: &str, font: TextStyle, ink: Color) -> El {
    let (shown, spans) = code_spans(src);
    let size = (font.size * 0.88).round();
    let spans = spans
        .into_iter()
        .map(|(a, b)| {
            rune::Span::new(a, b)
                .mono()
                .size(size)
                .bg(k.c.layer_02, 2.0)
        })
        .collect();
    rune::rich(shown, spans).font(font).color(ink)
}

/// `a `b` c` → ("a b c", [(2, 3)]): byte ranges of the code in the text without backticks.
/// An unmatched backtick stays as written.
pub fn code_spans(src: &str) -> (String, Vec<(usize, usize)>) {
    let mut out = String::with_capacity(src.len());
    let mut spans = Vec::new();
    let mut rest = src;
    while let Some(open) = rest.find('`') {
        let Some(len) = rest[open + 1..].find('`') else {
            break;
        };
        out.push_str(&rest[..open]);
        let start = out.len();
        out.push_str(&rest[open + 1..open + 1 + len]);
        if len > 0 {
            spans.push((start, out.len()));
        }
        rest = &rest[open + 2 + len..];
    }
    out.push_str(rest);
    (out, spans)
}

/// The length of `src` once its backticks are dropped, for strike ranges.
pub fn shown_len(src: &str) -> usize {
    code_spans(src).0.len()
}

#[cfg(test)]
mod tests {
    #[test]
    fn code_spans_drop_backticks() {
        assert_eq!(
            super::code_spans("use `gh api` here"),
            ("use gh api here".to_owned(), vec![(4, 10)])
        );
        assert_eq!(
            super::code_spans("odd ` tick"),
            ("odd ` tick".to_owned(), vec![])
        );
    }
}
