//! Review, one proposal at a time: read it, then accept (A), reject with a reason (R) or move on
//! (J / K). What's next waits in a queue underneath.

use super::{detail, style};
use crate::viewer::app::{App, Sel};
use crate::viewer::model::{self, Decision, Rule};
use rune::{El, ScrollSpec, Tx, col, el, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, TextInput};
use rune_theme_opencode::icons;

pub fn view(s: &App, k: &Kit) -> El {
    let items = s.inbox();
    let body = if items.is_empty() {
        zero(s, k)
    } else {
        let at = s.inbox_at.min(items.len() - 1);
        col()
            .w_full()
            .gap(18.0)
            .child(progress(k, at, items.len()))
            .child(card(s, k, &items[at]))
            .child(queue(s, k, &items, at))
    };
    col()
        .key("inbox-scroll")
        .w_full()
        .h_full()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            row().w_full().justify_center().child(
                col()
                    .w_full()
                    .max_w(760.0)
                    .px(32.0)
                    .pt(22.0)
                    .pb(40.0)
                    .child(body),
            ),
        )
}

fn progress(k: &Kit, at: usize, n: usize) -> El {
    let c = &k.c;
    let mut dots = row().gap(4.0).center_y();
    for i in 0..n.min(24) {
        let w = if i == at { 18.0 } else { 6.0 };
        dots = dots.child(
            el().key(format!("pip-{i}"))
                .w(w)
                .h(6.0)
                .radius(3.0)
                .fill(if i == at {
                    c.text_base
                } else if i < at {
                    c.border_strong
                } else {
                    c.border_weak
                }),
        );
    }
    row()
        .w_full()
        .center_y()
        .child(
            text(format!("{} of {n} waiting for you", at + 1))
                .font(style::SMALL)
                .color(c.text_muted),
        )
        .child(el().grow(1.0))
        .child(dots)
}

fn card(s: &App, k: &Kit, sel: &Sel) -> El {
    let c = &k.c;
    let inner = match sel {
        Sel::Decision(id) => s.ws.decision(id).map(|d| decision(s, k, d)),
        Sel::Rule(id) => s.ws.rule(id).map(|r| rule(k, r)),
    }
    .unwrap_or_else(el);
    col()
        .key(format!(
            "card-{}",
            match sel {
                Sel::Decision(i) | Sel::Rule(i) => i,
            }
        ))
        .w_full()
        .radius(14.0)
        .fill(c.surface_raised)
        .border(1.0, c.border_weak)
        .shadow(
            8.0,
            24.0,
            rune::Color::BLACK.alpha(if k.dark { 0.35 } else { 0.06 }),
        )
        .transition(rune::Transition::blur_replace())
        .child(
            col()
                .w_full()
                .px(28.0)
                .pt(24.0)
                .pb(22.0)
                .gap(20.0)
                .child(inner),
        )
        .child(style::hairline(k))
        .child(actions(s, k, sel))
}

fn kind_label(k: &Kit, label: &str, icon: &'static rune::Icon) -> El {
    row()
        .gap(6.0)
        .center_y()
        .child(rune::icon(icon, 13.0).color(k.c.icon_muted))
        .child(
            text(label.to_uppercase())
                .font(style::LABEL)
                .color(k.c.text_muted),
        )
}

fn decision(s: &App, k: &Kit, d: &Decision) -> El {
    let c = &k.c;
    let mut top = row()
        .w_full()
        .gap(8.0)
        .center_y()
        .child(kind_label(
            k,
            if d.rec.supersedes.is_some() {
                "Replacement"
            } else {
                "Proposed decision"
            },
            &icons::CHECKLIST,
        ))
        .child(el().grow(1.0))
        .child(style::category_chip(k, d.rec.category));
    if let Some(chip) = style::store_chip(k, d.store) {
        top = top.child(chip);
    }
    let mut body = col().w_full().gap(18.0).child(top);
    if let Some(old) = d.rec.supersedes.as_deref().and_then(|id| s.ws.decision(id)) {
        let n = style::shown_len(&old.rec.decision);
        body = body.child(
            col()
                .w_full()
                .gap(4.0)
                .px(12.0)
                .py(10.0)
                .radius(8.0)
                .fill(c.layer_01)
                .child(
                    text(format!("Retires {}", old.rec.id))
                        .font(style::MONO_SMALL)
                        .color(c.text_faint),
                )
                .child(
                    style::prose(k, &old.rec.decision, style::ROW, c.text_muted)
                        .strike(0, n, Some(c.text_faint))
                        .wrap(),
                ),
        );
    }
    body = body.child(
        style::prose(k, &d.rec.decision, style::DISPLAY, c.text_strong)
            .wrap()
            .selectable(true),
    );
    if !d.rec.reason.is_empty() {
        body = body.child(
            row()
                .w_full()
                .gap(12.0)
                .child(
                    el().w(3.0)
                        .radius(1.5)
                        .fill(style::category_colors(k, d.rec.category).0)
                        .self_stretch(),
                )
                .child(
                    style::prose(k, &d.rec.reason, style::LEAD, c.text_base)
                        .wrap()
                        .grow(1.0)
                        .min_w(0.0),
                ),
        );
    }
    let mut facts = style::wrap_row().gap(8.0);
    if d.rec.scope.is_empty() {
        facts = facts.child(style::chip("whole repository", c.layer_02, c.text_muted));
    }
    for sc in d.rec.scope.iter().take(6) {
        facts = facts.child(style::mono_chip(k, sc.clone()));
    }
    body = body.child(facts);
    if !d.rec.rejected.is_empty() {
        body = body.child(
            text(format!("Not chosen: {}", d.rec.rejected.join("; ")))
                .font(style::SMALL)
                .color(c.text_muted)
                .wrap(),
        );
    }
    if let Some(src) = d.rec.sources.first() {
        let more = d.rec.sources.len() - 1;
        body = body.child(
            row()
                .gap(6.0)
                .center_y()
                .child(rune::icon(&icons::LINK, 12.0).color(c.icon_faint))
                .child(
                    text(format!(
                        "Found in {}{}",
                        detail::pretty_source(src),
                        if more > 0 {
                            format!(" and {more} more")
                        } else {
                            String::new()
                        }
                    ))
                    .font(style::SMALL)
                    .color(c.text_faint)
                    .truncate(),
                ),
        );
    }
    let _ = model::age;
    body
}

fn rule(k: &Kit, r: &Rule) -> El {
    let c = &k.c;
    let mut body = col()
        .w_full()
        .gap(18.0)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(kind_label(k, "Proposed rule", &icons::CODE))
                .child(el().grow(1.0))
                .child(style::chip(
                    r.file.language.clone(),
                    c.layer_02,
                    c.text_muted,
                ))
                .child(
                    text(r.id().to_owned())
                        .font(style::MONO_SMALL)
                        .color(c.text_faint),
                ),
        )
        .child(
            text(if r.file.message.is_empty() {
                r.id().to_owned()
            } else {
                r.file.message.clone()
            })
            .font(style::DISPLAY)
            .color(c.text_strong)
            .wrap(),
        );
    let support = r.file.meta.support;
    body = body.child(text(format!("From {support} correction{}. Starts in shadow: it only logs until people confirm its hits.", if support == 1 { "" } else { "s" })).font(style::SMALL).color(c.text_muted).wrap());
    if let Some(ex) = &r.file.meta.example {
        body = body.child(detail::example(k, &ex.before, &ex.after));
    }
    if let Some(bt) = &r.file.meta.backtest {
        body = body.child(detail::backtest(k, bt));
    }
    body
}

type Handler = Box<dyn Fn(&mut App, &mut Tx)>;

fn actions(s: &App, k: &Kit, sel: &Sel) -> El {
    let c = &k.c;
    if let Some(reason) = &s.rejecting {
        let sel = sel.clone();
        let sel2 = sel.clone();
        return col()
            .w_full()
            .px(20.0)
            .py(14.0)
            .gap(10.0)
            .child(
                TextInput::new(reason)
                    .width(None)
                    .placeholder("Why not? Extraction won't propose it again.")
                    .label("Reason for rejecting")
                    .on_edit(|s: &mut App| match &mut s.rejecting {
                        Some(e) => e,
                        // The field closed before this edit ran.
                        None => &mut s.scratch,
                    })
                    .on_submit(move |s: &mut App, _tx: &mut Tx| {
                        let r = s
                            .rejecting
                            .as_ref()
                            .map(|e| e.text.clone())
                            .unwrap_or_default();
                        s.reject(&sel, r);
                    })
                    .build(k)
                    .key("reject-reason"),
            )
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(
                        text("Return to reject · Esc to cancel")
                            .font(style::SMALL)
                            .color(c.text_faint),
                    )
                    .child(el().grow(1.0))
                    .child(
                        Button::new("Cancel")
                            .variant(ButtonVariant::Ghost)
                            .control_size(ControlSize::Small)
                            .on_press(|s: &mut App| s.rejecting = None)
                            .build(k)
                            .key("reject-cancel"),
                    )
                    .child(
                        Button::new("Reject")
                            .variant(ButtonVariant::Danger)
                            .control_size(ControlSize::Small)
                            .on_press(move |s: &mut App| {
                                let r = s
                                    .rejecting
                                    .as_ref()
                                    .map(|e| e.text.clone())
                                    .unwrap_or_default();
                                s.reject(&sel2, r);
                            })
                            .build(k)
                            .key("reject-confirm"),
                    ),
            );
    }
    let accept_sel = sel.clone();
    let button = |label: &str, cap: &str, key: &str, variant: ButtonVariant, f: Handler| {
        let cap = cap.to_owned();
        let kk = k.clone();
        Button::new(label)
            .variant(variant)
            .label_with(move |font, ink| {
                row()
                    .gap(8.0)
                    .center_y()
                    .child(text(label_of(&cap).to_owned()).font(font).color(ink))
                    .child(style::keycap(&kk, &cap))
            })
            .on_press_tx(move |s: &mut App, tx: &mut Tx| f(s, tx))
            .build(k)
            .key(key.to_owned())
    };
    row()
        .w_full()
        .px(20.0)
        .py(14.0)
        .gap(8.0)
        .center_y()
        .child(button(
            "Skip",
            "J",
            "skip",
            ButtonVariant::Ghost,
            Box::new(|s: &mut App, tx: &mut Tx| s.step(1, tx)),
        ))
        .child(el().grow(1.0))
        .child(button(
            "Reject",
            "R",
            "reject",
            ButtonVariant::Outline,
            Box::new(|s: &mut App, tx: &mut Tx| {
                let mut e = rune::TextEdit::new("");
                e.focused = true;
                s.rejecting = Some(e);
                tx.focus("reject-reason", false);
            }),
        ))
        .child(button(
            "Accept",
            "A",
            "accept",
            ButtonVariant::Contrast,
            Box::new(move |s: &mut App, _tx: &mut Tx| s.accept(&accept_sel)),
        ))
}

fn label_of(cap: &str) -> &'static str {
    match cap {
        "J" => "Skip",
        "R" => "Reject",
        _ => "Accept",
    }
}

fn queue(s: &App, k: &Kit, items: &[Sel], at: usize) -> El {
    let c = &k.c;
    let rest: Vec<(usize, &Sel)> = items.iter().enumerate().filter(|(i, _)| *i != at).collect();
    if rest.is_empty() {
        return el();
    }
    let mut list = col().w_full().gap(2.0);
    for (i, sel) in rest.into_iter().take(12) {
        let (label, mark) = match sel {
            Sel::Decision(id) => {
                s.ws.decision(id)
                    .map(|d| {
                        (
                            d.rec.decision.clone(),
                            style::category_colors(k, d.rec.category).0,
                        )
                    })
                    .unwrap_or_default()
            }
            Sel::Rule(id) => {
                s.ws.rule(id)
                    .map(|r| (r.file.message.clone(), style::mode_colors(k, r.mode()).0))
                    .unwrap_or_default()
            }
        };
        let kind = if matches!(sel, Sel::Rule(_)) {
            "rule"
        } else {
            "decision"
        };
        list = list.child(
            row()
                .key(format!("queue-{i}"))
                .w_full()
                .gap(10.0)
                .center_y()
                .px(10.0)
                .py(7.0)
                .radius(8.0)
                .hover(c.hover)
                .role("button")
                .on_press(move |s: &mut App| {
                    s.inbox_at = i;
                    s.rejecting = None;
                    s.ensure_selection();
                })
                .child(style::dot(mark, 6.0))
                .child(
                    style::prose(
                        k,
                        &label,
                        style::ROW,
                        if i < at { c.text_faint } else { c.text_muted },
                    )
                    .truncate()
                    .grow(1.0)
                    .min_w(0.0),
                )
                .child(text(kind).font(style::SMALL).color(c.text_faint)),
        );
    }
    style::section(k, "Up next", list)
}

fn zero(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let mut actions = row().gap(8.0);
    if s.ws.root.is_some() {
        actions = actions
            .child(
                Button::new("Extract decisions")
                    .icon(&icons::BRAIN)
                    .variant(ButtonVariant::Neutral)
                    .on_press(|s: &mut App| s.extract())
                    .build(k)
                    .key("zero-extract"),
            )
            .child(
                Button::new("Learn rules")
                    .icon(&icons::GLASSES)
                    .variant(ButtonVariant::Ghost)
                    .on_press(|s: &mut App| s.mine_rules())
                    .build(k)
                    .key("zero-learn"),
            );
    }
    col()
        .w_full()
        .pt(90.0)
        .gap(12.0)
        .items_center()
        .child(row().size(52.0, 52.0).radius(26.0).center().fill(rune_theme_opencode::scale("mint", k.dark, 3)).child(rune::icon(&icons::CHECK, 24.0).color(style::success(k))))
        .child(text("Inbox zero").font(style::TITLE).color(c.text_strong))
        .child(text("Nothing is waiting for review. New proposals from agents and extraction land here.").font(style::SMALL).color(c.text_muted).wrap().align(rune::Align::Center).max_w(340.0))
        .child(el().h(6.0))
        .child(actions)
        .child(el().h(4.0))
        .child(text(format!("{} live decisions · {} rules", s.ws.live().count(), s.ws.accepted_rules().count())).font(style::SMALL).color(c.text_faint))
}
