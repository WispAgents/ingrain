//! Dialogs over the window: supersede a decision, read a commit's change, watch a command run.

use super::style;
use crate::app::{App, Sheet};
use rune::{El, ScrollSpec, Tx, col, el, row, text};
use rune_kit::diff::DiffView;
use rune_kit::{Button, ButtonVariant, Dialog, Kit, TextArea};

pub fn build(s: &App, k: &Kit, layer: &mut Vec<El>) {
    let Some(sheet) = &s.sheet else { return };
    match sheet {
        Sheet::Supersede {
            id,
            decision,
            reason,
        } => {
            let old =
                s.ws.decision(id)
                    .map(|d| d.rec.decision.clone())
                    .unwrap_or_default();
            let n = style::shown_len(&old);
            let body = col()
                .w_full()
                .px(16.0)
                .pb(16.0)
                .gap(10.0)
                .child(
                    col()
                        .w_full()
                        .mb(6.0)
                        .gap(4.0)
                        .px(12.0)
                        .py(10.0)
                        .radius(8.0)
                        .fill(k.c.layer_01)
                        .child(text(format!("{id} now")).font(style::MONO_SMALL).color(k.c.text_faint))
                        .child(style::prose(k, &old, style::ROW, k.c.text_muted).strike(0, n, Some(k.c.text_faint)).wrap()),
                )
                .child(style::section_label(k, "New decision"))
                .child(
                    TextArea::new(decision)
                        .width(None)
                        .label("New decision")
                        .placeholder("One imperative sentence an agent can follow")
                        .auto_grow(2, 6)
                        .on_edit(|s: &mut App| match &mut s.sheet {
                            Some(Sheet::Supersede { decision, .. }) => decision,
                            // The form closed before this edit ran.
                            _ => &mut s.scratch,
                        })
                        .build(k)
                        .key("supersede-decision"),
                )
                .child(style::section_label(k, "Why it changes"))
                .child(
                    TextArea::new(reason)
                        .width(None)
                        .label("Why it changes")
                        .placeholder("What made the old decision wrong or outdated")
                        .auto_grow(2, 5)
                        .on_edit(|s: &mut App| match &mut s.sheet {
                            Some(Sheet::Supersede { reason, .. }) => reason,
                            _ => &mut s.scratch,
                        })
                        .build(k)
                        .key("supersede-reason"),
                )
                .child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .center_y()
                        .mt(6.0)
                        .child(text("This writes a proposal. It replaces the decision once someone accepts it.").font(style::SMALL).color(k.c.text_faint).wrap().grow(1.0).min_w(0.0))
                        .child(Button::new("Cancel").variant(ButtonVariant::Ghost).on_press(|s: &mut App| s.sheet = None).build(k).key("supersede-cancel"))
                        .child(Button::new("Propose").variant(ButtonVariant::Contrast).disabled(!s.supersede_ready()).on_press(|s: &mut App| s.submit_supersede()).build(k).key("supersede-submit")),
                );
            Dialog::new("supersede", format!("Supersede {id}"))
                .fit()
                .open(true)
                .width(560.0)
                .click_outside(true)
                .on_close(|s: &mut App| s.sheet = None)
                .child(body)
                .build(k, layer);
        }
        Sheet::Commit {
            title,
            diff,
            lines,
            loaded,
        } => {
            let body: El = if diff.files().is_empty() {
                text(if *loaded {
                    "No change to this file in that commit (a merge, or a rename)."
                } else {
                    "Loading the change…"
                })
                .font(style::SMALL)
                .color(k.c.text_faint)
            } else {
                col()
                    .w_full()
                    .h((*lines as f32 * 17.0 + 70.0).clamp(140.0, 480.0))
                    .min_h(0.0)
                    .radius(8.0)
                    .clip_rounded(8.0)
                    .border(1.0, k.c.border_weak)
                    .child(
                        DiffView::new(
                            |s: &App| match &s.sheet {
                                Some(Sheet::Commit { diff, .. }) => diff,
                                // The sheet closed before a queued event ran.
                                _ => &s.spare_diff,
                            },
                            |s: &mut App| match &mut s.sheet {
                                Some(Sheet::Commit { diff, .. }) => diff,
                                _ => &mut s.spare_diff,
                            },
                        )
                        .key("commit-diff")
                        .build(s, k),
                    )
            };
            let body = col().w_full().px(16.0).pb(16.0).child(body);
            Dialog::new("commit", title.clone())
                .fit()
                .open(true)
                .width(820.0)
                .click_outside(true)
                .on_close(|s: &mut App| s.sheet = None)
                .child(body)
                .build(k, layer);
        }
        Sheet::Log { title } => {
            let running = !s.running.is_empty();
            let mut lines = col().key("log-lines").w_full().gap(1.0);
            for (i, l) in s.log.iter().enumerate() {
                lines = lines.child(
                    text(l.clone())
                        .key(format!("log-{i}"))
                        .font(style::MONO_SMALL)
                        .color(k.c.text_muted)
                        .wrap(),
                );
            }
            if s.log.is_empty() {
                lines = lines.child(
                    text(if running { "Starting…" } else { "No output." })
                        .font(style::MONO_SMALL)
                        .color(k.c.text_faint),
                );
            }
            let body = col()
                .w_full()
                .px(16.0)
                .pb(16.0)
                .gap(12.0)
                .child(
                    col()
                        .key("log-scroll")
                        .w_full()
                        .h(320.0)
                        .px(12.0)
                        .py(10.0)
                        .radius(8.0)
                        .fill(k.c.layer_01)
                        .border(1.0, k.c.border_weak)
                        .scroll(ScrollSpec { stick_bottom: true })
                        .child(lines),
                )
                .child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .center_y()
                        .child(if running { rune_kit::Spinner::new().build(k) } else { el() })
                        .child(text(if running { "Running. You can close this; it keeps going and the inbox fills in when it's done." } else { "Done. New proposals are in the inbox." }).font(style::SMALL).color(k.c.text_faint).wrap().grow(1.0).min_w(0.0))
                        .child(
                            Button::new(if running { "Hide" } else { "Open inbox" })
                                .variant(ButtonVariant::Neutral)
                                .on_press_tx(move |s: &mut App, _tx: &mut Tx| {
                                    s.sheet = None;
                                    if !running {
                                        s.set_lens(crate::app::Lens::Inbox);
                                    }
                                })
                                .build(k)
                                .key("log-close"),
                        ),
                );
            Dialog::new("log", title.clone())
                .fit()
                .open(true)
                .width(640.0)
                .click_outside(true)
                .on_close(|s: &mut App| s.sheet = None)
                .child(body)
                .build(k, layer);
        }
    }
}
