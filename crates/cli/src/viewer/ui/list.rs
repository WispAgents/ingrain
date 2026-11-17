//! The list beside the page: live decisions, superseded ones, or learned rules, under a search
//! field and category filters.

use super::style;
use crate::viewer::app::{App, Lens, Sel, row_key};
use crate::viewer::model::{self, Decision, Rule};
use rune::{El, ScrollSpec, Tx, col, el, row, text};
use rune_kit::{Kit, TextInput};
use rune_theme_opencode::icons;

pub fn search(s: &App, k: &Kit, placeholder: &str) -> El {
    TextInput::new(&s.filter)
        .width(None)
        .placeholder(placeholder)
        .leading_icon(&icons::MAGNIFYING_GLASS)
        .clear_button()
        .label("Filter")
        .on_edit(|s: &mut App| &mut s.filter)
        .on_change(|s: &mut App, _tx: &mut Tx| s.ensure_selection())
        .build(k)
        .key("search")
}

fn category_filters(s: &App, k: &Kit) -> El {
    let mut r = style::wrap_row().gap(4.0);
    for c in model::CATEGORIES {
        let n = s.ws.live().filter(|d| d.rec.category == c).count();
        if n == 0 {
            continue;
        }
        let on = s.category_filter == Some(c);
        let (mark, bg, fg) = style::category_colors(k, c);
        r = r.child(
            row()
                .key(format!("cat-{}", model::category_name(c)))
                .gap(5.0)
                .center_y()
                .px(7.0)
                .h(22.0)
                .radius(11.0)
                .fill(if on { bg } else { rune::Color::TRANSPARENT })
                .border(1.0, if on { bg } else { k.c.border_weak })
                .hover(k.c.hover)
                .role("button")
                .checked(on)
                .label(format!("{} decisions", model::category_name(c)))
                .on_press(move |s: &mut App| {
                    s.category_filter = if s.category_filter == Some(c) {
                        None
                    } else {
                        Some(c)
                    };
                    s.ensure_selection();
                })
                .child(style::dot(mark, 6.0))
                .child(
                    text(format!("{} {n}", model::category_name(c)))
                        .font(style::SMALL)
                        .color(if on { fg } else { k.c.text_muted }),
                ),
        );
    }
    r
}

pub fn view(s: &App, k: &Kit) -> El {
    let items = s.visible();
    let placeholder = match s.lens {
        Lens::Rules => "Filter rules",
        Lens::Archive => "Filter superseded decisions",
        _ => "Filter decisions",
    };
    let mut head = col()
        .w_full()
        .gap(10.0)
        .px(14.0)
        .pt(12.0)
        .pb(10.0)
        .child(search(s, k, placeholder));
    if s.lens == Lens::Decisions {
        head = head.child(category_filters(s, k));
    }
    let mut rows = col().key("rows").w_full().px(8.0).pb(16.0).gap(2.0);
    for sel in &items {
        rows = rows.child(match sel {
            Sel::Decision(id) => {
                s.ws.decision(id)
                    .map(|d| decision_row(s, k, d))
                    .unwrap_or_else(el)
            }
            Sel::Rule(id) => s.ws.rule(id).map(|r| rule_row(s, k, r)).unwrap_or_else(el),
        });
    }
    if items.is_empty() {
        rows = rows.child(empty(s, k));
    }
    col().w_full().h_full().min_h(0.0).child(head).child(
        col()
            .key("list-scroll")
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .scroll_edge_fade(12.0)
            .role("list")
            .label(s.lens.title())
            .child(rows),
    )
}

fn empty(s: &App, k: &Kit) -> El {
    let (title, hint) = if !s.filter.text.trim().is_empty() {
        ("Nothing matches", "Try fewer words, or clear the filter.")
    } else {
        match s.lens {
            Lens::Rules => (
                "No learned rules yet",
                "“Learn rules” mines your corrections and proposes rules for review.",
            ),
            Lens::Archive => (
                "Nothing superseded",
                "Replaced decisions stay here, linked to what replaced them.",
            ),
            _ => (
                "No decisions yet",
                "“Extract decisions” reads your docs, commits and reviews and proposes some.",
            ),
        }
    };
    col()
        .w_full()
        .py(40.0)
        .px(16.0)
        .gap(6.0)
        .items_center()
        .child(text(title).font(style::ROW).color(k.c.text_base))
        .child(
            text(hint)
                .font(style::SMALL)
                .color(k.c.text_faint)
                .wrap()
                .align(rune::Align::Center),
        )
}

fn list_row(s: &App, k: &Kit, sel: Sel, accent: rune::Color, body: El) -> El {
    let selected = s.sel.as_ref() == Some(&sel);
    let key = row_key(&sel);
    row()
        .key(key)
        .w_full()
        .px(10.0)
        .py(9.0)
        .gap(10.0)
        .radius(8.0)
        .fill(if selected {
            k.c.surface_raised
        } else {
            rune::Color::TRANSPARENT
        })
        .border(
            1.0,
            if selected {
                k.c.border_weak
            } else {
                rune::Color::TRANSPARENT
            },
        )
        .hover(if selected {
            k.c.surface_raised
        } else {
            k.c.hover
        })
        .role("listitem")
        .on_press(move |s: &mut App| s.select(sel.clone()))
        .child(col().pt(5.0).child(style::dot(accent, 7.0)))
        .child(body.grow(1.0).min_w(0.0))
}

fn decision_row(s: &App, k: &Kit, d: &Decision) -> El {
    let (mark, _, _) = style::category_colors(k, d.rec.category);
    let mut meta = row().gap(6.0).center_y().child(
        text(d.rec.id.clone())
            .font(style::MONO_SMALL)
            .color(k.c.text_faint),
    );
    if let Some(chip) = style::store_chip(k, d.store) {
        meta = meta.child(chip);
    }
    if d.enforced() {
        meta = meta.child(
            rune::icon(&icons::SHIELD, 12.0)
                .color(k.c.icon_faint)
                .help("Enforced by a check"),
        );
    }
    meta = meta.child(el().grow(1.0));
    if !d.rec.date.is_empty() {
        meta = meta.child(
            text(model::age(&d.rec.date, s.ws.loaded_at))
                .font(style::SMALL)
                .color(k.c.text_faint),
        );
    }
    let mut body = col().gap(4.0).child(meta).child(
        style::prose(
            k,
            &d.rec.decision,
            style::ROW,
            if d.is_live() {
                k.c.text_base
            } else {
                k.c.text_muted
            },
        )
        .wrap()
        .max_lines(3),
    );
    if let Some(by) = &d.rec.superseded_by {
        body = body.child(
            text(format!("→ replaced by {by}"))
                .font(style::SMALL)
                .color(k.c.text_faint),
        );
    } else if !d.rec.scope.is_empty() && d.rec.scope != ["**"] {
        let shown: Vec<_> = d.rec.scope.iter().take(2).cloned().collect();
        let more = d.rec.scope.len().saturating_sub(2);
        let mut line = shown.join("  ");
        if more > 0 {
            line.push_str(&format!("  +{more}"));
        }
        body = body.child(
            text(line)
                .font(style::MONO_SMALL)
                .color(k.c.text_faint)
                .truncate(),
        );
    }
    list_row(s, k, Sel::Decision(d.rec.id.clone()), mark, body)
}

fn rule_row(s: &App, k: &Kit, r: &Rule) -> El {
    let (mark, _, _) = style::mode_colors(k, r.mode());
    let mut meta = row().gap(6.0).center_y().child(
        text(r.id().to_owned())
            .font(style::MONO_SMALL)
            .color(k.c.text_faint)
            .truncate()
            .min_w(0.0),
    );
    if let Some(chip) = style::store_chip(k, r.store) {
        meta = meta.child(chip);
    }
    meta = meta.child(el().grow(1.0)).child(
        text(r.file.language.clone())
            .font(style::SMALL)
            .color(k.c.text_faint),
    );
    let hits = r.stats.hits;
    let foot = text(if hits == 0 {
        format!("{} · no hits yet", r.mode())
    } else {
        format!(
            "{} · {hits} hits · last {}",
            r.mode(),
            model::age_secs(r.stats.last_seen, s.ws.loaded_at)
        )
    })
    .font(style::SMALL)
    .color(k.c.text_faint);
    let body = col()
        .gap(4.0)
        .child(meta)
        .child(
            text(if r.file.message.is_empty() {
                r.id().to_owned()
            } else {
                r.file.message.clone()
            })
            .font(style::ROW)
            .color(k.c.text_base)
            .wrap()
            .max_lines(3),
        )
        .child(foot);
    list_row(s, k, Sel::Rule(r.id().to_owned()), mark, body)
}
