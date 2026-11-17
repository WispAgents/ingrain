//! When things were decided: every decision on one rail, newest first, under month headers that
//! stick while their month scrolls. Superseded ones are hollow and point at what replaced them.

use super::{list, style};
use crate::app::{App, Sel, row_key};
use crate::model;
use rune::{Color, El, ScrollSpec, col, el, row, text};
use rune_kit::Kit;

pub fn view(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let items = s.visible();
    let mut months: Vec<(String, Vec<El>)> = Vec::new();
    for sel in &items {
        let Sel::Decision(id) = sel else { continue };
        let Some(d) = s.ws.decision(id) else { continue };
        let month = model::year_month(&d.rec.date).map_or("Undated".to_owned(), |(y, m)| {
            format!("{} {y}", model::month_name(m))
        });
        let day = d
            .rec
            .date
            .get(8..10)
            .unwrap_or("")
            .trim_start_matches('0')
            .to_owned();
        let live = d.is_live();
        let selected = s.sel.as_ref() == Some(sel);
        let (mark, _, _) = style::category_colors(k, d.rec.category);
        let node = if live {
            style::dot(mark, 11.0).border(2.0, c.bg_base)
        } else {
            style::ring(c.icon_faint, 11.0).fill(c.bg_base)
        };
        let n = style::shown_len(&d.rec.decision);
        let mut sentence = style::prose(
            k,
            &d.rec.decision,
            style::ROW,
            if live { c.text_base } else { c.text_faint },
        )
        .wrap()
        .max_lines(3);
        if !live {
            sentence = sentence.strike(0, n, None);
        }
        let mut meta = row().gap(6.0).center_y().child(
            text(d.rec.id.clone())
                .font(style::MONO_SMALL)
                .color(c.text_faint),
        );
        if let Some(by) = &d.rec.superseded_by {
            meta = meta.child(
                text(format!("→ {by}"))
                    .font(style::MONO_SMALL)
                    .color(c.text_muted),
            );
        }
        if let Some(prev) = &d.rec.supersedes {
            meta = meta.child(
                text(format!("replaces {prev}"))
                    .font(style::SMALL)
                    .color(c.text_faint),
            );
        }
        let sel2 = sel.clone();
        let entry = row()
            .key(row_key(sel))
            .w_full()
            .gap(0.0)
            .role("listitem")
            .on_press(move |s: &mut App| s.select(sel2.clone()))
            .child(
                text(day)
                    .font(style::MONO_SMALL)
                    .color(c.text_faint)
                    .w(26.0)
                    .pt(10.0)
                    .align(rune::Align::End),
            )
            // The rail: a line through every row, the node on it.
            .child(
                col()
                    .w(28.0)
                    .self_stretch()
                    .items_center()
                    .child(el().w(2.0).h(10.0).fill(c.border_weak))
                    .child(node)
                    .child(el().w(2.0).grow(1.0).fill(c.border_weak)),
            )
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .my(3.0)
                    .px(10.0)
                    .py(7.0)
                    .gap(3.0)
                    .radius(8.0)
                    .fill(if selected {
                        c.surface_raised
                    } else {
                        Color::TRANSPARENT
                    })
                    .border(
                        1.0,
                        if selected {
                            c.border_weak
                        } else {
                            Color::TRANSPARENT
                        },
                    )
                    .hover(if selected { c.surface_raised } else { c.hover })
                    .child(sentence)
                    .child(meta),
            );
        match months.last_mut() {
            Some((m, rows)) if *m == month => rows.push(entry),
            _ => months.push((month, vec![entry])),
        }
    }
    let mut body = col().key("timeline-rows").w_full().pb(24.0);
    for (month, rows) in months {
        let count = rows.len();
        let header = row()
            .w_full()
            .px(14.0)
            .py(8.0)
            .gap(8.0)
            .center_y()
            .fill(c.bg_base)
            .sticky_top(0.0)
            .child(
                text(month.clone())
                    .font(style::TITLE.lh(18.0))
                    .color(c.text_strong),
            )
            .child(
                text(format!("{count}"))
                    .font(style::SMALL)
                    .color(c.text_faint),
            );
        body = body.child(
            col()
                .key(format!("month-{month}"))
                .w_full()
                .child(header)
                .child(col().w_full().px(8.0).children(rows)),
        );
    }
    if items.is_empty() {
        body = body.child(
            text("No decisions yet.")
                .font(style::SMALL)
                .color(c.text_faint)
                .px(16.0)
                .py(24.0),
        );
    }
    col()
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(
            col()
                .w_full()
                .px(14.0)
                .pt(12.0)
                .pb(10.0)
                .child(list::search(s, k, "Filter the timeline")),
        )
        .child(
            col()
                .key("timeline-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .scroll_edge_fade(12.0)
                .role("list")
                .label("Timeline")
                .child(body),
        )
}
