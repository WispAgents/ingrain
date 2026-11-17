//! The window: a sidebar of lenses, the lens's list, and the selected item's page.

mod applies;
mod detail;
mod home;
mod inbox;
mod list;
mod sheets;
pub mod style;
mod timeline;

use crate::viewer::app::{App, Lens};
use ingrain_core::Store;
use rune::{Chrome, Cx, El, Icon, KeyEvent, Tx, col, el, row, text};
use rune_kit::shell::{self, Sidebar, SidebarRow, SidebarToggle, SplitView, Toolbar};
use rune_kit::{ControlSize, IconButton, IconButtonVariant, Kit, Toast, ToastRegion};
use rune_theme_opencode::icons;

pub const CHROME: Chrome = Chrome {
    full_size_content: true,
    traffic_lights: Some((16.0, 25.0)),
    drag_regions: true,
};

pub fn view(s: &App, cx: &mut Cx<App>) -> El {
    let k = rune_theme_opencode::kit(cx.system_dark());
    let mut layer = Vec::new();
    let body = if s.home {
        home::view(s, k, cx.titlebar_inset())
    } else {
        shell_view(s, k, cx)
    };
    sheets::build(s, k, &mut layer);
    if !s.toasts.is_empty() {
        let region = s.toasts.iter().fold(ToastRegion::new(), |r, t| {
            let id = t.id;
            let lead = style::dot(
                if t.ok {
                    style::success(k)
                } else {
                    style::danger(k)
                },
                8.0,
            );
            r.toast(
                Toast::new(format!("toast-{id}"), t.title.clone())
                    .description(t.body.clone())
                    .lead(lead)
                    .closable()
                    .on_close(move |s: &mut App| s.toasts.retain(|t| t.id != id))
                    .build(k),
            )
        });
        layer.push(region.build(k));
    }
    let base = el()
        .key("drop-root")
        .w_full()
        .h_full()
        .child(body)
        .on_drop_files(|s: &mut App, files: &[std::path::PathBuf], tx: &mut Tx| {
            s.drop_over = false;
            if let Some(f) = files.first() {
                dropped(s, f, tx);
            }
        })
        .on_drop_hover(|s: &mut App, over: bool, _tx: &mut Tx| s.drop_over = over);
    rune_kit::root(base, layer)
}

/// A folder opens that repository; a file inside the open one asks what applies to it.
fn dropped(s: &mut App, path: &std::path::Path, tx: &mut Tx) {
    // Resolved, so `/tmp` and `/private/tmp` (or any symlink) compare equal.
    let real = |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let path = &real(path);
    if let (Some(root), false) = (s.root().map(real), path.is_dir())
        && let Ok(rel) = path.strip_prefix(&root)
    {
        s.applies = rune::TextEdit::new(rel.to_string_lossy().replace('\\', "/"));
        s.read_applies();
        s.set_lens(Lens::Applies);
        return;
    }
    let dir = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().map(|p| p.to_path_buf()).unwrap_or_default()
    };
    s.open_dir(&dir, tx);
}

fn lens_icon(lens: Lens) -> &'static Icon {
    match lens {
        Lens::Inbox => &icons::REVIEW,
        Lens::Decisions => &icons::CHECKLIST,
        Lens::Rules => &icons::CODE,
        Lens::Timeline => &icons::FORK,
        Lens::Applies => &icons::FILE_TREE,
        Lens::Archive => &icons::ARCHIVE,
    }
}

fn lens_count(s: &App, lens: Lens) -> Option<usize> {
    let n = match lens {
        Lens::Inbox => s.ws.inbox_len(),
        Lens::Decisions => s.ws.live().count(),
        Lens::Rules => s.ws.accepted_rules().count(),
        Lens::Archive => s.ws.superseded().count(),
        Lens::Timeline | Lens::Applies => return None,
    };
    Some(n)
}

fn repo_avatar(k: &Kit, name: &str) -> El {
    let initial = name
        .chars()
        .next()
        .unwrap_or('·')
        .to_uppercase()
        .to_string();
    row()
        .size(20.0, 20.0)
        .radius(6.0)
        .center()
        .fill(rune_theme_opencode::brand(k.dark))
        .child(
            text(initial)
                .font(k.ty.small_medium)
                .color(rune::Color::WHITE),
        )
}

fn sidebar(s: &App, k: &Kit) -> El {
    let subtitle = match (&s.ws.root, s.ws.branch.as_str()) {
        (Some(_), "") => "repository".to_owned(),
        (Some(_), b) => b.to_owned(),
        (None, _) => "global store".to_owned(),
    };
    let repo = SidebarRow::new(s.ws.name.clone())
        .avatar(repo_avatar(k, &s.ws.name))
        .subtitle(subtitle)
        .height(44.0)
        .trailing(rune::icon(&icons::CHEVRON_GRABBER_VERTICAL, 14.0).color(k.c.icon_faint))
        .on_select(|s: &mut App, _| s.go_home())
        .build(k)
        .key("repo-switcher");
    let mut sb = Sidebar::new()
        .label("Lenses")
        .inset(0.0)
        .header_gap(2.0)
        .header_padding(0.0, 14.0)
        .row_gap(2.0)
        .header(repo);
    for (i, lens) in Lens::ALL.into_iter().enumerate() {
        let mut r = SidebarRow::new(lens.title())
            .icon(lens_icon(lens))
            .height(32.0)
            .selected(s.lens == lens)
            .on_select(move |s: &mut App, _| s.set_lens(lens));
        match (lens, lens_count(s, lens)) {
            (Lens::Inbox, Some(n)) if n > 0 => r = r.badge(n.to_string()),
            (_, Some(n)) => r = r.detail(n.to_string()),
            _ => r = r.detail(format!("⌘{}", i + 1)),
        }
        sb = sb.child(r.build(k).key(lens.key()));
    }
    if let Some(stores) = store_rows(s, k) {
        sb = sb.child(stores);
    }
    let busy = !s.running.is_empty();
    let extract = SidebarRow::new(if busy {
        "Working…"
    } else {
        "Extract decisions"
    })
    .icon(&icons::BRAIN)
    .height(32.0)
    .on_select(|s: &mut App, _| s.extract())
    .build(k)
    .key("extract");
    let learn = SidebarRow::new("Learn rules")
        .icon(&icons::GLASSES)
        .height(32.0)
        .on_select(|s: &mut App, _| s.mine_rules())
        .build(k)
        .key("learn");
    let mut footer = col().w_full().gap(2.0);
    if !s.ws.problems.is_empty() {
        let n = s.ws.problems.len();
        let detail = s.ws.problems.join("\n");
        footer = footer.child(
            row()
                .key("problems")
                .w_full()
                .gap(8.0)
                .px(10.0)
                .py(8.0)
                .mb(6.0)
                .radius(8.0)
                .fill(rune_theme_opencode::scale("amber", k.dark, 3))
                .help(detail.clone())
                .role("button")
                .label("Files that didn't load")
                .on_press(move |s: &mut App| {
                    s.toast(false, "Files that didn't load", detail.clone())
                })
                .child(
                    rune::icon(&icons::WARNING, 14.0)
                        .color(rune_theme_opencode::scale("amber", k.dark, 11)),
                )
                .child(
                    text(format!(
                        "{n} file{} didn't load",
                        if n == 1 { "" } else { "s" }
                    ))
                    .font(style::SMALL)
                    .color(rune_theme_opencode::scale("amber", k.dark, 11)),
                ),
        );
    }
    if s.ws.root.is_some() {
        footer = footer.child(extract).child(learn);
    }
    sb.footer(footer).build(k)
}

fn store_title(st: Store) -> &'static str {
    match st {
        Store::Project => "Project",
        Store::Local => "Just me, this repo",
        Store::Global => "Just me, every repo",
    }
}

fn store_icon(st: Store) -> &'static Icon {
    match st {
        Store::Project => &icons::FOLDER,
        Store::Local => &icons::LOCK,
        Store::Global => &icons::GLOBE,
    }
}

/// Where decisions and rules live, as a filter: the team's (committed), this repository only
/// (`--local`), every repository (`--global`). Shown once anything personal exists.
fn store_rows(s: &App, k: &Kit) -> Option<El> {
    let count = |st: Store| {
        s.ws.decisions
            .iter()
            .filter(|d| d.store == st && !d.pending && d.is_live())
            .count()
            + s.ws
                .rules
                .iter()
                .filter(|r| r.store == st && !r.pending)
                .count()
    };
    let personal = s.ws.decisions.iter().any(|d| d.store != Store::Project)
        || s.ws.rules.iter().any(|r| r.store != Store::Project);
    if !personal {
        return None;
    }
    let mut c = col().w_full().gap(2.0).pt(14.0).child(
        row()
            .px(10.0)
            .pb(4.0)
            .child(style::section_label(k, "Stores")),
    );
    for st in Store::ALL {
        let help = match st {
            Store::Project => "Committed in decisions/ and .corrections/: the whole team and CI",
            Store::Local => {
                "Kept in .git/ingrain/local: only you, only this repository, never committed"
            }
            Store::Global => {
                "Kept in ~/.config/ingrain: only you, every repository on this machine"
            }
        };
        let row_ = SidebarRow::new(store_title(st))
            .icon(store_icon(st))
            .height(30.0)
            .detail(count(st).to_string())
            .selected(s.store_filter == Some(st))
            .on_select(move |s: &mut App, _| {
                s.store_filter = if s.store_filter == Some(st) {
                    None
                } else {
                    Some(st)
                };
                if s.lens == Lens::Inbox || s.lens == Lens::Applies {
                    s.set_lens(Lens::Decisions);
                }
                s.ensure_selection();
            })
            .build(k)
            .help(help)
            .key(format!("store-{}", st.name()));
        c = c.child(row_);
    }
    Some(c)
}

fn shell_view(s: &App, k: &Kit, cx: &mut Cx<App>) -> El {
    let main = match s.lens {
        Lens::Inbox => inbox::view(s, k),
        Lens::Timeline => split(s, k, timeline::view(s, k)),
        Lens::Applies => split(s, k, applies::view(s, k)),
        _ => split(s, k, list::view(s, k)),
    };
    let mut title = match s.lens {
        Lens::Inbox => format!("Inbox · {}", s.ws.inbox_len()),
        l => l.title().to_owned(),
    };
    if let Some(st) = s.store_filter {
        title = format!("{title} · {}", store_title(st).to_lowercase());
    }
    let toolbar = Toolbar::new().title(title).trailing(
        row().gap(6.0).center_y().child(
            IconButton::new(&icons::REFRESH)
                .variant(IconButtonVariant::GhostMuted)
                .label("Reload")
                .help("Reload from disk")
                .on_press(|s: &mut App| s.reload())
                .build(k)
                .key("reload"),
        ),
    );
    SplitView::new(main)
        .sidebar(&s.sidebar, |s: &mut App| &mut s.sidebar, sidebar(s, k))
        .toolbar(toolbar)
        .titlebar_inset(cx.titlebar_inset())
        .frame_inset(8.0)
        .toolbar_in_panel()
        .sidebar_padding(18.0, 10.0)
        .sidebar_toggle(
            SidebarToggle::new()
                .size(ControlSize::Large)
                .past_lights(16.0)
                .edge(18.0)
                .gap(4.0)
                .in_header(),
        )
        .reduced_motion(cx.is_reduced_motion())
        .build(k)
}

/// A list beside the selected item's page.
fn split(s: &App, k: &Kit, list: El) -> El {
    row()
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(
            col()
                .key("list-pane")
                .w(380.0)
                .h_full()
                .min_h(0.0)
                .child(list),
        )
        .child(el().w(1.0).h_full().fill(k.c.border_weak))
        .child(
            col()
                .key("detail-pane")
                .grow(1.0)
                .h_full()
                .min_h(0.0)
                .min_w(0.0)
                .child(detail::view(s, k)),
        )
}

pub fn on_key(s: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    use rune::Key;
    if shell::is_sidebar_toggle(ev) {
        s.sidebar.toggle();
        return true;
    }
    if ev.key == Key::Escape {
        if s.sheet.is_some() {
            s.sheet = None;
            return true;
        }
        if s.rejecting.is_some() {
            s.rejecting = None;
            return true;
        }
    }
    if s.sheet.is_some() {
        return false;
    }
    for (i, lens) in Lens::ALL.into_iter().enumerate() {
        if ev.is_command(char::from(b'1' + i as u8)) {
            s.set_lens(lens);
            return true;
        }
    }
    if ev.is_command('o') {
        open_folder(tx);
        return true;
    }
    if ev.is_command('f') {
        tx.focus(
            if s.lens == Lens::Applies {
                "applies-input"
            } else {
                "search"
            },
            false,
        );
        return true;
    }
    if ev.is_command('r') {
        s.reload();
        return true;
    }
    // A focused text field types letters itself (they never reach here), so single-key
    // shortcuts are safe whatever else has focus.
    if tx.focused().is_some_and(|f| {
        f.contains("input")
            || f.contains("search")
            || f.contains("reason")
            || f.contains("supersede")
    }) {
        return false;
    }
    match &ev.key {
        Key::Down => s.step(1, tx),
        Key::Up => s.step(-1, tx),
        Key::Text(t) if !ev.mods.cmd => match t.as_str() {
            "j" => s.step(1, tx),
            "k" => s.step(-1, tx),
            "/" => tx.focus("search", false),
            "a" if s.lens == Lens::Inbox => {
                if let Some(sel) = s.sel.clone() {
                    s.accept(&sel);
                }
            }
            "r" if s.lens == Lens::Inbox && s.sel.is_some() => {
                let mut edit = rune::TextEdit::new("");
                edit.focused = true;
                s.rejecting = Some(edit);
                tx.focus("reject-reason", false);
            }
            _ => return false,
        },
        _ => return false,
    }
    true
}

pub fn open_folder(tx: &mut Tx) {
    let options = rune::FileDialogOptions {
        title: "Open a repository".into(),
        ..Default::default()
    };
    tx.open_folder(options, |s: &mut App, result, tx| {
        if let Ok(Some(path)) = result {
            s.open_dir(&path, tx);
        }
    });
}
