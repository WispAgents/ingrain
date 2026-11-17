//! Opened outside a repository (or from the repo switcher): open one, pick a recent one, drop
//! a folder, or browse the personal rules kept for every repository.

use super::style;
use crate::app::App;
use rune::{Color, El, ScrollSpec, Tx, col, el, row, text};
use rune_kit::{Button, ButtonVariant, Kit};
use rune_theme_opencode::icons;

pub fn view(s: &App, k: &Kit, inset: f32) -> El {
    let c = &k.c;
    let mut recents = col().w_full().gap(2.0);
    for (i, p) in s.recents.iter().enumerate() {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let home = std::env::var("HOME").unwrap_or_default();
        let shown = p.to_string_lossy().replacen(&home, "~", 1);
        let current = s.ws.root.as_deref() == Some(p.as_path());
        let path = p.clone();
        recents = recents.child(
            row()
                .key(format!("recent-{i}"))
                .w_full()
                .gap(12.0)
                .center_y()
                .px(12.0)
                .py(9.0)
                .radius(9.0)
                .hover(c.hover)
                .role("button")
                .label(format!("Open {name}"))
                .on_press_tx(move |s: &mut App, tx: &mut Tx| s.open_dir(&path, tx))
                .child(
                    row()
                        .size(26.0, 26.0)
                        .radius(7.0)
                        .center()
                        .fill(c.layer_02)
                        .child(
                            text(
                                name.chars()
                                    .next()
                                    .unwrap_or('·')
                                    .to_uppercase()
                                    .to_string(),
                            )
                            .font(style::ROW)
                            .color(c.text_muted),
                        ),
                )
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(1.0)
                        .child(text(name).font(style::ROW).color(c.text_base))
                        .child(
                            text(shown)
                                .font(style::SMALL)
                                .color(c.text_faint)
                                .truncate_head(),
                        ),
                )
                .child(if current {
                    style::chip("open", c.layer_02, c.text_muted)
                } else {
                    el()
                }),
        );
    }
    let drop = col()
        .key("drop-zone")
        .w_full()
        .py(22.0)
        .gap(6.0)
        .items_center()
        .radius(12.0)
        .border(
            1.5,
            if s.drop_over {
                c.border_focus
            } else {
                c.border_base
            },
        )
        .border_dash(5.0, 4.0)
        .fill(if s.drop_over {
            c.surface_raised
        } else {
            Color::TRANSPARENT
        })
        .child(rune::icon(&icons::FOLDER, 18.0).color(c.icon_muted))
        .child(
            text("Drop a repository folder here")
                .font(style::SMALL)
                .color(c.text_muted),
        );
    let personal =
        s.ws.decisions
            .iter()
            .filter(|d| d.store != ingrain_core::Store::Project)
            .count()
            + s.ws
                .rules
                .iter()
                .filter(|r| r.store != ingrain_core::Store::Project)
                .count();
    let mut buttons = row()
        .gap(8.0)
        .child(
            Button::new("Open repository…")
                .icon(&icons::FOLDER)
                .variant(ButtonVariant::Contrast)
                .shortcut("⌘O")
                .on_press_tx(|_: &mut App, tx: &mut Tx| super::open_folder(tx))
                .build(k)
                .key("open-repo"),
        )
        .child(
            Button::new("Personal rules")
                .icon(&icons::GLOBE)
                .variant(ButtonVariant::Ghost)
                .help("Decisions and rules kept for every repository (~/.config/ingrain)")
                .on_press(|s: &mut App| s.open_personal())
                .build(k)
                .key("open-personal"),
        );
    if s.ws.root.is_some() {
        buttons = buttons.child(el().grow(1.0)).child(
            Button::new(format!("Back to {}", s.ws.name))
                .variant(ButtonVariant::Ghost)
                .on_press(|s: &mut App| s.home = false)
                .build(k)
                .key("home-back"),
        );
    }
    let mut body = col()
        .w_full()
        .max_w(520.0)
        .gap(22.0)
        .child(
            col()
                .gap(6.0)
                .child(
                    row()
                        .gap(10.0)
                        .center_y()
                        .child(
                            row()
                                .size(30.0, 30.0)
                                .radius(8.0)
                                .center()
                                .fill(rune_theme_opencode::brand(k.dark))
                                .child(text("i").font(style::TITLE).color(Color::WHITE)),
                        )
                        .child(text("ingrain").font(style::DISPLAY).color(c.text_strong)),
                )
                .child(
                    text("What your team decided, and the rules learned from your corrections.")
                        .font(style::LEAD)
                        .color(c.text_muted)
                        .wrap(),
                ),
        )
        .child(buttons.w_full())
        .child(drop);
    if !s.recents.is_empty() {
        body = body.child(style::section(k, "Recent", recents));
    }
    if personal > 0 {
        body = body.child(
            text(format!(
                "{personal} personal decisions and rules apply to every repository."
            ))
            .font(style::SMALL)
            .color(c.text_faint),
        );
    }
    col()
        .w_full()
        .h_full()
        .fill(c.bg_base)
        .pt(inset.max(28.0) + 40.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            row()
                .w_full()
                .justify_center()
                .px(32.0)
                .pb(40.0)
                .child(body),
        )
}
