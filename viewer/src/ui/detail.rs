//! The selected item's page: a decision read like a short document (what, why, what we didn't
//! choose, where it applies, how it is enforced, where it came from, how it changed), or a
//! learned rule with its mode, the fix it learned from and how often it fires.

use super::style;
use crate::actions;
use crate::app::{App, Sel};
use crate::model::{self, Decision, Rule, WEEKS};
use ingrain_core::Mode;
use ingrain_ledger::Status;
use rune::{Color, El, ScrollSpec, Tx, col, el, row, text};
use rune_kit::{
    Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Kit, SegmentedControl,
};
use rune_theme_opencode::icons;

pub fn view(s: &App, k: &Kit) -> El {
    let page = match &s.sel {
        Some(Sel::Decision(id)) => s.ws.decision(id).map(|d| decision(s, k, d)),
        Some(Sel::Rule(id)) => s.ws.rule(id).map(|r| rule(s, k, r)),
        None => None,
    };
    let page = page.unwrap_or_else(|| {
        col()
            .w_full()
            .h_full()
            .center()
            .gap(6.0)
            .child(rune::icon(&icons::CHECKLIST, 22.0).color(k.c.icon_faint))
            .child(
                text("Select something to read it")
                    .font(style::SMALL)
                    .color(k.c.text_faint),
            )
    });
    col()
        .key("detail-scroll")
        .w_full()
        .h_full()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .scroll_edge_fade(12.0)
        .child(page)
}

fn page(body: El) -> El {
    row().w_full().justify_center().child(
        col()
            .key("page")
            .w_full()
            .max_w(720.0)
            .px(36.0)
            .pt(28.0)
            .pb(48.0)
            .gap(26.0)
            .child(body),
    )
}

fn status_chip(k: &Kit, d: &Decision) -> El {
    let dark = k.dark;
    let (label, h) = if d.pending {
        ("proposed", "amber")
    } else {
        match d.rec.status {
            Status::Accepted => ("live", "mint"),
            Status::Superseded => ("superseded", "smoke"),
            Status::Proposed => ("proposed", "amber"),
        }
    };
    let (mark, bg, fg) = (
        rune_theme_opencode::scale(h, dark, 9),
        rune_theme_opencode::scale(h, dark, 3),
        rune_theme_opencode::scale(h, dark, 11),
    );
    row()
        .gap(5.0)
        .center_y()
        .px(7.0)
        .py(2.0)
        .radius(5.0)
        .fill(bg)
        .child(style::dot(mark, 6.0))
        .child(text(label).font(style::SMALL.lh(14.0)).color(fg))
}

fn icon_action(
    k: &Kit,
    key: &str,
    icon: &'static rune::Icon,
    label: &str,
    f: impl Fn(&mut App, &mut Tx) + 'static,
) -> El {
    IconButton::new(icon)
        .variant(IconButtonVariant::GhostMuted)
        .label(label)
        .help(label)
        .on_press_tx(f)
        .build(k)
        .key(key.to_owned())
}

fn decision_actions(s: &App, k: &Kit, d: &Decision) -> El {
    let line = d.agent_line();
    let id = d.rec.id.clone();
    let path = d.path.clone();
    let path2 = d.path.clone();
    let mut r = row()
        .gap(2.0)
        .center_y()
        .child(icon_action(
            k,
            "copy-line",
            &icons::COPY,
            "Copy the line agents get",
            move |s: &mut App, tx| {
                tx.copy(line.clone());
                s.toast(true, "Copied", "The decision as agents see it");
            },
        ))
        .child(icon_action(
            k,
            "open-file",
            &icons::OPEN_FILE,
            "Open the record file",
            move |_: &mut App, _| actions::open_file(&path),
        ))
        .child(icon_action(
            k,
            "reveal",
            &icons::FOLDER,
            "Show in folder",
            move |_: &mut App, _| actions::reveal(&path2),
        ));
    if d.is_live() && s.ws.root.is_some() {
        r = r.child(
            Button::new("Supersede…")
                .variant(ButtonVariant::Outline)
                .control_size(ControlSize::Small)
                .help("Propose a replacement; a person accepts it")
                .on_press(move |s: &mut App| s.start_supersede(&id))
                .build(k)
                .key("supersede"),
        );
    }
    r
}

fn decision(s: &App, k: &Kit, d: &Decision) -> El {
    let c = &k.c;
    let mut meta = row()
        .w_full()
        .gap(6.0)
        .center_y()
        .child(text(d.rec.id.clone()).font(style::MONO).color(c.text_muted))
        .child(style::category_chip(k, d.rec.category))
        .child(status_chip(k, d));
    if let Some(chip) = style::store_chip(k, d.store) {
        meta = meta.child(chip);
    }
    meta = meta.child(el().grow(1.0)).child(decision_actions(s, k, d));

    let mut body = col().w_full().gap(26.0).child(
        col()
            .w_full()
            .gap(14.0)
            .child(meta)
            .child(
                style::prose(k, &d.rec.decision, style::DISPLAY, c.text_strong)
                    .key("decision-text")
                    .wrap()
                    .selectable(true),
            )
            .child(byline(s, k, d)),
    );

    if !d.rec.reason.is_empty() {
        let (mark, _, _) = style::category_colors(k, d.rec.category);
        body = body.child(style::section(
            k,
            "Why",
            row()
                .w_full()
                .gap(14.0)
                .child(el().w(3.0).radius(1.5).fill(mark).self_stretch())
                .child(
                    style::prose(k, &d.rec.reason, style::LEAD, c.text_base)
                        .wrap()
                        .selectable(true)
                        .grow(1.0)
                        .min_w(0.0),
                ),
        ));
    }
    if !d.rec.rejected.is_empty() {
        let mut list = col().w_full().gap(8.0);
        for alt in &d.rec.rejected {
            let n = style::shown_len(alt);
            list = list.child(
                row()
                    .w_full()
                    .gap(10.0)
                    .child(
                        col()
                            .pt(3.0)
                            .child(rune::icon(&icons::CIRCLE_BAN_SIGN, 13.0).color(c.icon_faint)),
                    )
                    .child(
                        style::prose(k, alt, style::LEAD, c.text_muted)
                            .strike(0, n, Some(c.text_faint))
                            .wrap()
                            .grow(1.0)
                            .min_w(0.0),
                    ),
            );
        }
        body = body.child(style::section(k, "Not chosen", list));
    }
    body = body.child(style::section(k, "Applies to", scope(k, d)));
    if d.enforced() {
        body = body.child(style::section(k, "Enforced by", checks(k, d)));
    }
    let chain = s.ws.lineage(&d.rec.id);
    if chain.len() > 1 {
        body = body.child(style::section(k, "Lineage", lineage(k, &chain, &d.rec.id)));
    }
    if !d.rec.sources.is_empty() {
        body = body.child(style::section(
            k,
            "Where it was decided",
            sources(s, k, &d.rec.sources),
        ));
    }
    if let Some(note) = personal_note(k, d.store) {
        body = body.child(style::section(k, "Where it lives", note));
    } else if s.ws.root.is_some() && !d.pending {
        body = body.child(style::section(k, "History", history(s, k, &d.path)));
    }
    page(body)
}

fn byline(s: &App, k: &Kit, d: &Decision) -> El {
    let mut parts = Vec::new();
    if !d.rec.date.is_empty() {
        parts.push(format!(
            "Decided {} ({})",
            d.rec.date,
            model::age(&d.rec.date, s.ws.loaded_at)
        ));
    }
    if !d.rec.author.is_empty() {
        parts.push(format!("by {}", d.rec.author));
    }
    if d.pending {
        parts.push("waiting for review".into());
    }
    text(parts.join(" "))
        .font(style::SMALL)
        .color(k.c.text_faint)
}

fn scope(k: &Kit, d: &Decision) -> El {
    if d.rec.scope.is_empty() || d.rec.scope.iter().all(|s| s == "**") {
        return row()
            .gap(8.0)
            .center_y()
            .child(rune::icon(&icons::GLOBE, 14.0).color(k.c.icon_muted))
            .child(
                text("The whole repository")
                    .font(style::LEAD)
                    .color(k.c.text_base),
            );
    }
    let mut r = style::wrap_row();
    for s in &d.rec.scope {
        let sel = s.clone();
        r = r.child(
            style::mono_chip(k, s.clone())
                .hover(k.c.hover)
                .role("button")
                .label(format!("What else applies to {s}"))
                .help("See everything that applies here")
                .on_press(move |s: &mut App| {
                    let path = sel
                        .trim_end_matches("/**")
                        .trim_end_matches("**")
                        .trim_end_matches('/')
                        .to_owned();
                    s.applies = rune::TextEdit::new(path);
                    s.read_applies();
                    s.set_lens(crate::app::Lens::Applies);
                }),
        );
    }
    r
}

fn checks(k: &Kit, d: &Decision) -> El {
    let c = &k.c;
    let mut list = col().w_full().gap(6.0);
    let line = |kind: &str, what: String| {
        row()
            .w_full()
            .gap(10.0)
            .center_y()
            .px(12.0)
            .py(8.0)
            .radius(8.0)
            .fill(c.layer_01)
            .border(1.0, c.border_weak)
            .child(
                text(kind.to_owned())
                    .font(style::SMALL)
                    .color(c.text_faint)
                    .w(112.0),
            )
            .child(
                text(what)
                    .font(style::MONO)
                    .color(c.text_base)
                    .wrap()
                    .grow(1.0)
                    .min_w(0.0)
                    .selectable(true),
            )
    };
    for dep in &d.rec.checks.forbid_deps {
        list = list.child(line("no dependency", dep.clone()));
    }
    for m in &d.rec.checks.forbid_imports {
        list = list.child(line("no import", m.clone()));
    }
    for p in &d.rec.checks.forbid_patterns {
        list = list.child(line(&format!("no {} code", p.language), p.pattern.clone()));
    }
    list.child(
        text("Checked on added lines at edit time, before commit and in CI.")
            .font(style::SMALL)
            .color(c.text_faint),
    )
}

fn lineage(k: &Kit, chain: &[&Decision], current: &str) -> El {
    let c = &k.c;
    let mut r = col().w_full().gap(0.0);
    for (i, d) in chain.iter().enumerate() {
        let here = d.rec.id == current;
        let live = d.is_live();
        let id = d.rec.id.clone();
        let rail = col()
            .w(14.0)
            .items_center()
            .child(el().w(2.0).h(10.0).fill(if i == 0 {
                Color::TRANSPARENT
            } else {
                c.border_base
            }))
            .child(if live {
                style::dot(style::success(k), 10.0)
            } else {
                style::ring(c.icon_faint, 10.0)
            })
            .child(
                el().w(2.0)
                    .grow(1.0)
                    .min_h(10.0)
                    .fill(if i + 1 == chain.len() {
                        Color::TRANSPARENT
                    } else {
                        c.border_base
                    }),
            );
        let n = style::shown_len(&d.rec.decision);
        let mut sentence = style::prose(
            k,
            &d.rec.decision,
            style::SMALL.lh(17.0),
            if live { c.text_base } else { c.text_muted },
        )
        .wrap();
        if !live {
            sentence = sentence.strike(0, n, Some(c.text_faint));
        }
        r = r.child(
            row()
                .key(format!("lineage-{id}"))
                .w_full()
                .gap(10.0)
                .px(6.0)
                .radius(8.0)
                .fill(if here {
                    c.surface_raised
                } else {
                    Color::TRANSPARENT
                })
                .hover(c.hover)
                .role("link")
                .label(format!("Open {id}"))
                .on_press(move |s: &mut App| s.select(Sel::Decision(id.clone())))
                .child(rail)
                .child(
                    col()
                        .py(8.0)
                        .gap(2.0)
                        .grow(1.0)
                        .min_w(0.0)
                        .child(
                            text(format!(
                                "{}  ·  {}",
                                d.rec.id,
                                if d.rec.date.is_empty() {
                                    "undated"
                                } else {
                                    &d.rec.date
                                }
                            ))
                            .font(style::MONO_SMALL)
                            .color(c.text_faint),
                        )
                        .child(sentence),
                ),
        );
    }
    r
}

fn sources(s: &App, k: &Kit, list: &[String]) -> El {
    let mut col_ = col().w_full().gap(2.0);
    for src in list {
        let target = s.ws.targets.get(src).cloned().flatten();
        let is_url = target.as_deref().is_some_and(|t| t.starts_with("http"));
        let icon = if src.contains("github.com") {
            &icons::GITHUB
        } else if is_url {
            &icons::LINK
        } else {
            &icons::FILE_DOCUMENT
        };
        let label = pretty_source(src);
        let mut r = row()
            .w_full()
            .gap(10.0)
            .center_y()
            .px(8.0)
            .py(6.0)
            .radius(6.0)
            .child(rune::icon(icon, 14.0).color(k.c.icon_muted))
            .child(
                text(label)
                    .font(style::ROW)
                    .color(if target.is_some() {
                        k.c.text_base
                    } else {
                        k.c.text_muted
                    })
                    .truncate()
                    .grow(1.0)
                    .min_w(0.0),
            );
        if let Some(t) = target {
            r = r
                .hover(k.c.hover)
                .role("link")
                .label(format!("Open {src}"))
                .on_press_tx(move |_: &mut App, tx: &mut Tx| {
                    if t.starts_with("http") {
                        let _ = tx.open_url(&t);
                    } else {
                        actions::open_file(std::path::Path::new(&t));
                    }
                })
                .child(rune::icon(&icons::SQUARE_ARROW_TOP_RIGHT, 12.0).color(k.c.icon_faint));
        }
        col_ = col_.child(r);
    }
    col_
}

/// `https://github.com/o/r/pull/8` → `o/r #8 (pull request)`; commits shortened.
pub fn pretty_source(src: &str) -> String {
    if let Some(rest) = src.strip_prefix("https://github.com/") {
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() >= 4 {
            let repo = format!("{}/{}", parts[0], parts[1]);
            return match parts[2] {
                "pull" => format!(
                    "{repo} #{} · pull request",
                    parts[3].split('#').next().unwrap_or("")
                ),
                "issues" => format!("{repo} #{} · issue", parts[3]),
                "commit" => format!("{repo} @ {} · commit", &parts[3][..parts[3].len().min(8)]),
                _ => src.to_owned(),
            };
        }
    }
    src.to_owned()
}

fn history(s: &App, k: &Kit, path: &std::path::Path) -> El {
    let c = &k.c;
    let Some(commits) = s.history.get(path) else {
        return text("Reading git history…")
            .font(style::SMALL)
            .color(c.text_faint);
    };
    if commits.is_empty() {
        return text("Not committed yet.")
            .font(style::SMALL)
            .color(c.text_faint);
    }
    let mut list = col().w_full();
    let n = commits.len();
    for (i, cm) in commits.iter().enumerate() {
        let (hash, subject) = (cm.hash.clone(), cm.subject.clone());
        let first = i + 1 == n;
        let rail = col()
            .w(14.0)
            .items_center()
            .child(el().w(2.0).h(12.0).fill(if i == 0 {
                Color::TRANSPARENT
            } else {
                c.border_weak
            }))
            .child(if first {
                style::dot(c.icon_muted, 8.0)
            } else {
                style::ring(c.icon_faint, 8.0)
            })
            .child(el().w(2.0).grow(1.0).min_h(8.0).fill(if first {
                Color::TRANSPARENT
            } else {
                c.border_weak
            }));
        list = list.child(
            row()
                .key(format!("commit-{i}"))
                .w_full()
                .gap(10.0)
                .px(6.0)
                .radius(8.0)
                .hover(c.hover)
                .role("button")
                .label(format!(
                    "Show what {} changed",
                    &cm.hash[..cm.hash.len().min(8)]
                ))
                .on_press(move |s: &mut App| s.open_commit(hash.clone(), subject.clone()))
                .child(rail)
                .child(
                    col()
                        .py(7.0)
                        .gap(2.0)
                        .grow(1.0)
                        .min_w(0.0)
                        .child(
                            text(cm.subject.clone())
                                .font(style::ROW)
                                .color(c.text_base)
                                .truncate(),
                        )
                        .child(
                            text(format!(
                                "{}  ·  {}  ·  {}{}",
                                &cm.hash[..cm.hash.len().min(8)],
                                cm.author,
                                model::age_secs(cm.time, s.ws.loaded_at),
                                if first { "  ·  created" } else { "" }
                            ))
                            .font(style::SMALL)
                            .color(c.text_faint),
                        ),
                )
                .child(rune::icon(&icons::CHEVRON_RIGHT, 12.0).color(c.icon_faint)),
        );
    }
    list
}

// ---- rules ----

fn mode_text(m: Mode) -> &'static str {
    match m {
        Mode::Shadow => {
            "Shadow: hits are logged, nobody is told. Rules start here until hits are confirmed."
        }
        Mode::Warn => "Warn: the agent is told when it writes this; the edit stands.",
        Mode::Block => "Block: the agent must fix it, and CI fails on added lines that match.",
    }
}

pub fn rule(s: &App, k: &Kit, r: &Rule) -> El {
    let c = &k.c;
    let mut meta = row()
        .w_full()
        .gap(6.0)
        .center_y()
        .child(
            text(r.id().to_owned())
                .font(style::MONO)
                .color(c.text_muted)
                .truncate()
                .min_w(0.0),
        )
        .child(style::chip(
            r.file.language.clone(),
            c.layer_02,
            c.text_muted,
        ));
    if r.pending {
        meta = meta.child(style::chip(
            "proposed",
            rune_theme_opencode::scale("amber", k.dark, 3),
            rune_theme_opencode::scale("amber", k.dark, 11),
        ));
    } else {
        meta = meta.child(style::mode_chip(k, r.mode()));
    }
    if let Some(chip) = style::store_chip(k, r.store) {
        meta = meta.child(chip);
    }
    let path = r.file.path.clone();
    let path2 = r.file.path.clone();
    meta = meta
        .child(el().grow(1.0))
        .child(icon_action(
            k,
            "open-file",
            &icons::OPEN_FILE,
            "Open the rule file",
            move |_: &mut App, _| actions::open_file(&path),
        ))
        .child(icon_action(
            k,
            "reveal",
            &icons::FOLDER,
            "Show in folder",
            move |_: &mut App, _| actions::reveal(&path2),
        ));

    let support = r.file.meta.support;
    let origin = match r.file.meta.origin.as_str() {
        "llm" => "written by a model",
        "template" => "from a template",
        _ => "learned",
    };
    let mut body = col().w_full().gap(26.0).child(
        col()
            .w_full()
            .gap(14.0)
            .child(meta)
            .child(
                text(if r.file.message.is_empty() {
                    r.id().to_owned()
                } else {
                    r.file.message.clone()
                })
                .font(style::DISPLAY)
                .color(c.text_strong)
                .wrap()
                .selectable(true),
            )
            .child(
                text(format!(
                    "Learned from {support} correction{} · {origin}",
                    if support == 1 { "" } else { "s" }
                ))
                .font(style::SMALL)
                .color(c.text_faint),
            ),
    );

    if !r.pending {
        let id = r.id().to_owned();
        let at = match r.mode() {
            Mode::Shadow => 0,
            Mode::Warn => 1,
            Mode::Block => 2,
        };
        body = body.child(style::section(
            k,
            "Mode",
            col()
                .gap(8.0)
                .child(
                    SegmentedControl::new(&["Shadow", "Warn", "Block"], Some(at))
                        .label("Rule mode")
                        .on_pick(move |s: &mut App, i: usize, _tx: &mut Tx| {
                            let m = [Mode::Shadow, Mode::Warn, Mode::Block][i.min(2)];
                            s.set_mode(&id, m);
                        })
                        .build(k)
                        .key("mode"),
                )
                .child(
                    text(mode_text(r.mode()))
                        .font(style::SMALL)
                        .color(c.text_muted)
                        .wrap(),
                ),
        ));
        body = body.child(style::section(k, "In use", usage(s, k, r)));
    }
    if let Some(ex) = &r.file.meta.example {
        body = body.child(style::section(
            k,
            "The fix it learned from",
            example(k, &ex.before, &ex.after),
        ));
    }
    if let Some(bt) = &r.file.meta.backtest {
        body = body.child(style::section(k, "Backtest", backtest(k, bt)));
    }
    if !r.file.meta.evidence.is_empty() {
        body = body.child(style::section(
            k,
            "Evidence",
            sources(s, k, &r.file.meta.evidence),
        ));
    }
    body = body.child(style::section(
        k,
        "Rule file",
        col()
            .w_full()
            .px(14.0)
            .py(12.0)
            .radius(8.0)
            .fill(c.layer_01)
            .border(1.0, c.border_weak)
            .child(
                text(r.yaml.trim_end().to_owned())
                    .font(style::MONO)
                    .color(c.text_muted)
                    .wrap()
                    .selectable(true),
            ),
    ));
    if let Some(note) = personal_note(k, r.store) {
        body = body.child(style::section(k, "Where it lives", note));
    } else if s.ws.root.is_some() && !r.pending {
        body = body.child(style::section(k, "History", history(s, k, &r.file.path)));
    }
    page(body)
}

fn usage(s: &App, k: &Kit, r: &Rule) -> El {
    let c = &k.c;
    let st = &r.stats;
    let max = st.weekly.iter().copied().max().unwrap_or(0).max(1) as f32;
    let (mark, _, _) = style::mode_colors(k, r.mode());
    let mut bars = row().h(40.0).gap(3.0).items_end();
    let weekly = if st.weekly.is_empty() {
        vec![0; WEEKS]
    } else {
        st.weekly.clone()
    };
    for (i, n) in weekly.iter().enumerate() {
        let h = if *n == 0 {
            2.0
        } else {
            4.0 + 36.0 * (*n as f32 / max)
        };
        bars = bars.child(
            el().key(format!("week-{i}"))
                .w(10.0)
                .h(h)
                .radius(2.0)
                .fill(if *n == 0 { c.border_weak } else { mark })
                .help(format!("{n} hits, {} weeks ago", WEEKS - 1 - i)),
        );
    }
    let rate = (st.overrides * 100)
        .checked_div(st.hits + st.overrides)
        .unwrap_or(0);
    col()
        .w_full()
        .gap(16.0)
        .child(
            row()
                .w_full()
                .gap(28.0)
                .child(style::stat(
                    k,
                    st.hits.to_string(),
                    "distinct hits",
                    c.text_strong,
                ))
                .child(style::stat(
                    k,
                    st.overrides.to_string(),
                    &format!("overrides ({rate}%)"),
                    if rate > 30 {
                        style::danger(k)
                    } else {
                        c.text_strong
                    },
                ))
                .child(style::stat(
                    k,
                    model::age_secs(st.last_seen, s.ws.loaded_at),
                    "last hit",
                    c.text_strong,
                )),
        )
        .child(
            row().gap(12.0).items_end().child(bars).child(
                text("hits a week, last 12 weeks")
                    .font(style::SMALL)
                    .color(c.text_faint),
            ),
        )
}

/// Before → after, as removed and added lines.
pub fn example(k: &Kit, before: &str, after: &str) -> El {
    let c = &k.c;
    let dark = k.dark;
    let side = |lines: &str, sign: &str, h: &str| {
        let mut col_ = col().w_full();
        for l in lines.trim_end().lines() {
            col_ = col_.child(
                row()
                    .w_full()
                    .px(10.0)
                    .py(1.0)
                    .fill(rune_theme_opencode::scale(h, dark, 3).alpha(0.7))
                    .child(
                        text(sign.to_owned())
                            .font(style::MONO)
                            .color(rune_theme_opencode::scale(h, dark, 11))
                            .w(16.0),
                    )
                    .child(
                        text(l.to_owned())
                            .font(style::MONO)
                            .color(c.text_base)
                            .grow(1.0)
                            .min_w(0.0)
                            .wrap(),
                    ),
            );
        }
        col_
    };
    col()
        .w_full()
        .py(6.0)
        .radius(8.0)
        .clip_rounded(8.0)
        .fill(c.layer_01)
        .border(1.0, c.border_weak)
        .child(side(before, "−", "coral"))
        .child(side(after, "+", "mint"))
}

pub fn backtest(k: &Kit, bt: &ingrain_core::Backtest) -> El {
    let c = &k.c;
    let bar = |label: &str, hit: u32, of: u32, good_when_high: bool| {
        let frac = if of == 0 { 0.0 } else { hit as f32 / of as f32 };
        let good = if good_when_high {
            frac >= 0.5
        } else {
            hit == 0
        };
        let ink = if good {
            style::success(k)
        } else {
            style::danger(k)
        };
        row()
            .w_full()
            .gap(12.0)
            .center_y()
            .child(
                text(label.to_owned())
                    .font(style::SMALL)
                    .color(c.text_muted)
                    .w(210.0),
            )
            .child(
                el().grow(1.0).h(6.0).radius(3.0).fill(c.layer_02).child(
                    el().w_frac(frac.clamp(0.0, 1.0))
                        .h(6.0)
                        .radius(3.0)
                        .fill(ink),
                ),
            )
            .child(
                text(format!("{hit} / {of}"))
                    .font(style::MONO_SMALL)
                    .color(c.text_muted)
                    .w(64.0)
                    .align(rune::Align::End),
            )
    };
    col()
        .w_full()
        .gap(10.0)
        .child(bar(
            "Flags the agent's code it learned from",
            bt.before_hits,
            bt.befores,
            true,
        ))
        .child(bar(
            "Flags the corrected code (should be 0)",
            bt.after_hits,
            bt.afters,
            false,
        ))
        .child(bar(
            "Flags agent code people kept",
            bt.kept_hits,
            bt.kept,
            false,
        ))
        .child(
            text(format!(
                "{} matches in the tree when proposed (existing code never blocks).",
                bt.head_hits
            ))
            .font(style::SMALL)
            .color(c.text_faint),
        )
}

/// Personal items have no git history: say where they are and who they reach instead.
fn personal_note(k: &Kit, store: ingrain_core::Store) -> Option<El> {
    let (icon, what, detail) = match store {
        ingrain_core::Store::Project => return None,
        ingrain_core::Store::Local => (
            &icons::LOCK,
            "Just you, in this repository.",
            "Kept in .git/ingrain/local/, so it's never committed. Your agent follows it here; teammates and CI don't see it.",
        ),
        ingrain_core::Store::Global => (
            &icons::GLOBE,
            "Just you, in every repository on this machine.",
            "Kept in ~/.config/ingrain/ ($INGRAIN_HOME). Your agent follows it in any repository set up for ingrain; nothing is committed.",
        ),
    };
    let (bg, fg) = style::store_colors(k, store);
    Some(
        row()
            .w_full()
            .gap(12.0)
            .px(14.0)
            .py(12.0)
            .radius(10.0)
            .fill(bg)
            .child(col().pt(2.0).child(rune::icon(icon, 15.0).color(fg)))
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(3.0)
                    .child(text(what).font(style::ROW).color(fg))
                    .child(
                        text(detail)
                            .font(style::SMALL.lh(17.0))
                            .color(k.c.text_muted)
                            .wrap(),
                    ),
            ),
    )
}
