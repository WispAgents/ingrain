//! "What applies to this file?": a path in, the decisions and rules an agent editing it gets.
//! Drop a file on the window to ask about it.

use super::style;
use crate::app::{App, Sel, row_key};
use ingrain_ledger::scope::Match;
use rune::{Color, El, ScrollSpec, Tx, col, el, row, text};
use rune_kit::{Kit, TextInput};
use rune_theme_opencode::icons;

pub fn view(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let input = TextInput::new(&s.applies)
        .width(None)
        .placeholder("A path in this repository")
        .leading_icon(&icons::FILE_TREE)
        .clear_button()
        .label("Path")
        .on_edit(|s: &mut App| &mut s.applies)
        .on_change(|s: &mut App, _tx: &mut Tx| {
            s.read_applies();
            s.ensure_selection();
        })
        .build(k)
        .key("applies-input");
    let head = col()
        .w_full()
        .gap(8.0)
        .px(14.0)
        .pt(12.0)
        .pb(10.0)
        .child(input)
        .child(
            text("Or drop a file from this repository on the window.")
                .font(style::SMALL)
                .color(c.text_faint),
        );
    let path = s.applies.text.trim();
    let body = if path.is_empty() {
        suggestions(s, k)
    } else {
        results(s, k)
    };
    col().w_full().h_full().min_h(0.0).child(head).child(
        col()
            .key("applies-scroll")
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(body),
    )
}

fn group(k: &Kit, title: &str, hint: &str, rows: Vec<El>) -> Option<El> {
    if rows.is_empty() {
        return None;
    }
    Some(
        col()
            .w_full()
            .gap(4.0)
            .px(8.0)
            .pb(12.0)
            .child(
                row()
                    .px(8.0)
                    .py(4.0)
                    .gap(8.0)
                    .center_y()
                    .child(style::section_label(k, title))
                    .child(
                        text(hint.to_owned())
                            .font(style::SMALL)
                            .color(k.c.text_faint),
                    ),
            )
            .children(rows),
    )
}

fn item(s: &App, k: &Kit, sel: Sel, mark: Color, id: &str, line: String) -> El {
    let c = &k.c;
    let selected = s.sel.as_ref() == Some(&sel);
    let sel2 = sel.clone();
    row()
        .key(row_key(&sel))
        .w_full()
        .gap(10.0)
        .px(10.0)
        .py(8.0)
        .radius(8.0)
        .fill(if selected {
            c.surface_raised
        } else {
            Color::TRANSPARENT
        })
        .hover(if selected { c.surface_raised } else { c.hover })
        .on_press(move |s: &mut App| s.select(sel2.clone()))
        .child(col().pt(5.0).child(style::dot(mark, 7.0)))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(3.0)
                .child(
                    text(id.to_owned())
                        .font(style::MONO_SMALL)
                        .color(c.text_faint),
                )
                .child(
                    style::prose(k, &line, style::ROW, c.text_base)
                        .wrap()
                        .max_lines(3),
                ),
        )
}

fn results(s: &App, k: &Kit) -> El {
    let (decisions, rules) = s.applies();
    let by = |m: Match| -> Vec<El> {
        decisions
            .iter()
            .filter(|(_, mm)| *mm == m)
            .map(|(d, _)| {
                item(
                    s,
                    k,
                    Sel::Decision(d.rec.id.clone()),
                    style::category_colors(k, d.rec.category).0,
                    &d.rec.id,
                    d.rec.decision.clone(),
                )
            })
            .collect()
    };
    let rule_rows: Vec<El> = rules
        .iter()
        .map(|r| {
            item(
                s,
                k,
                Sel::Rule(r.id().to_owned()),
                style::mode_colors(k, r.mode()).0,
                r.id(),
                r.file.message.clone(),
            )
        })
        .collect();
    let total = decisions.len() + rules.len();
    let exists = s.applies_exists();
    let mut out = col().w_full().gap(4.0).child(
        row()
            .w_full()
            .px(16.0)
            .pb(8.0)
            .gap(6.0)
            .center_y()
            .child(
                text(format!("{total} apply"))
                    .font(style::TITLE.lh(18.0))
                    .color(k.c.text_strong),
            )
            .child(el().grow(1.0))
            .child(
                text(match exists {
                    Some(true) => "file exists",
                    Some(false) => "no such file (globs still match)",
                    None => "",
                })
                .font(style::SMALL)
                .color(k.c.text_faint),
            ),
    );
    for g in [
        group(k, "Scoped here", "path globs that match", by(Match::Glob)),
        group(
            k,
            "Mentions a symbol",
            "names found in the file",
            by(Match::Symbol),
        ),
        group(k, "Learned rules", "same language", rule_rows),
        group(k, "Everywhere", "repo-wide decisions", by(Match::RepoWide)),
    ]
    .into_iter()
    .flatten()
    {
        out = out.child(g);
    }
    out
}

/// Paths from the decisions' own scopes, to try.
fn suggestions(s: &App, k: &Kit) -> El {
    let mut seen = std::collections::BTreeSet::new();
    for d in s.ws.live() {
        for sc in &d.rec.scope {
            let p = sc
                .trim_end_matches("/**")
                .trim_end_matches("**")
                .trim_end_matches('/');
            if !p.is_empty() && !p.contains('*') && p.contains('/') {
                seen.insert(p.to_owned());
            }
        }
    }
    let mut r = style::wrap_row();
    for p in seen.into_iter().take(14) {
        let p2 = p.clone();
        r = r.child(
            style::mono_chip(k, p)
                .hover(k.c.hover)
                .role("button")
                .on_press(move |s: &mut App| {
                    s.applies = rune::TextEdit::new(p2.clone());
                    s.read_applies();
                    s.ensure_selection();
                }),
        );
    }
    col()
        .w_full()
        .px(16.0)
        .pt(8.0)
        .gap(10.0)
        .child(style::section_label(k, "Try"))
        .child(r)
}
