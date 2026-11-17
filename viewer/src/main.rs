//! `ingrain-viewer`: a native window onto a repository's decisions and learned rules.
//!
//! ```text
//! ingrain view [PATH]            # from the CLI, anywhere inside a repository
//! ingrain-viewer [PATH]          # directly; no repository → the home screen
//! ingrain-viewer --repo PATH --lens=timeline --select=D-0004
//! ```

mod actions;
mod app;
mod bus;
mod history;
mod model;
mod ui;

use app::App;

/// Tests that set `INGRAIN_HOME` hold this, so parallel tests don't see each other's value.
#[cfg(test)]
pub static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
use rune::AppSpec;

fn setup(s: &mut App, waker: rune::Waker) {
    s.bus.set_waker(waker);
}

fn poll(s: &mut App, now: f64, tx: &mut rune::Tx) -> bool {
    s.poll(tx) | s.expire_toasts(now, tx)
}

fn main() {
    rune::prewarm_gpu();
    let spec = AppSpec {
        fonts: rune_theme_opencode::FONTS,
        families: rune_theme_opencode::FAMILIES,
        setup: Some(setup),
        poll: Some(poll),
        on_key: Some(ui::on_key),
        min_size: Some((760.0, 480.0)),
        scenes: &[
            ("inbox", |s| s.set_lens(app::Lens::Inbox)),
            ("live", |s| s.set_lens(app::Lens::Decisions)),
            ("rules", |s| s.set_lens(app::Lens::Rules)),
            ("timeline", |s| s.set_lens(app::Lens::Timeline)),
            ("applies", |s| s.set_lens(app::Lens::Applies)),
            ("archive", |s| s.set_lens(app::Lens::Archive)),
            ("home", |s| s.go_home()),
        ],
        ..AppSpec::new("ingrain", (1180.0, 760.0), App::new, ui::view)
    };
    rune::App::new(spec).chrome(ui::CHROME).run();
}
