//! `ingrain viewer`: a native window onto a repository's decisions and learned rules, built
//! with Rune. It reads files with the same crates as the CLI and makes every change by running
//! this same binary's commands, one at a time.

mod actions;
mod app;
mod bus;
mod history;
mod model;
mod ui;

use app::App;
use rune::AppSpec;

fn setup(s: &mut App, waker: rune::Waker) {
    s.bus.set_waker(waker);
}

fn poll(s: &mut App, now: f64, tx: &mut rune::Tx) -> bool {
    s.poll(tx) | s.expire_toasts(now, tx)
}

fn first_state() -> App {
    App::with_args(&[])
}

/// Open the window and run until it closes. `app_args` choose what it opens (`--repo`,
/// `--personal`, `--select=`, `--applies=`); `rune_args` are Rune's own flags, for headless
/// runs (`--snap`, `--script`, `--scene`, `--tree`, ...).
pub fn run(app_args: Vec<String>, rune_args: Vec<String>) -> anyhow::Result<()> {
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
        ..AppSpec::new("ingrain", (1180.0, 760.0), first_state, ui::view)
    };
    rune::App::new(spec)
        .init(move || App::with_args(&app_args))
        // Rune's flags only, without a program name (Rune's `App::args`).
        .args(rune_args)
        .chrome(ui::CHROME)
        .try_run()
        .map_err(|e| anyhow::anyhow!("the viewer couldn't start: {e}"))
}
