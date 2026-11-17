//! Work off the UI thread: loading, git history, CLI runs and the file watcher. Workers push a
//! [`Msg`] and wake the window; `poll` drains them on the UI thread.

use crate::viewer::history::Commit;
use crate::viewer::model::{Source, Workspace};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

pub enum Msg {
    /// A fresh load; `generation` drops answers for a repository no longer open.
    Loaded {
        generation: u64,
        ws: Workspace,
    },
    History {
        path: PathBuf,
        commits: Vec<Commit>,
    },
    Patch {
        hash: String,
        patch: String,
    },
    /// A repository opened off the UI thread.
    Opened {
        source: Source,
        ws: Workspace,
    },
    /// The file "Applies to" asks about: its text (for symbol scopes) and whether it exists.
    AppliesFile {
        path: String,
        content: Option<String>,
        exists: bool,
    },
    /// One line of a running command's output.
    Line(String),
    Done {
        label: String,
        ok: bool,
        output: String,
    },
}

#[derive(Clone, Default)]
pub struct Bus {
    queue: Arc<Mutex<Vec<Msg>>>,
    waker: Arc<Mutex<Option<rune::Waker>>>,
    /// Bumped whenever another repository opens; the watcher of an older one stops.
    generation: Arc<AtomicU64>,
}

impl Bus {
    pub fn set_waker(&self, w: rune::Waker) {
        *self.waker.lock().unwrap() = Some(w);
    }

    pub fn send(&self, msg: Msg) {
        self.queue.lock().unwrap().push(msg);
        if let Some(w) = self.waker.lock().unwrap().as_ref() {
            w();
        }
    }

    pub fn drain(&self) -> Vec<Msg> {
        std::mem::take(&mut *self.queue.lock().unwrap())
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Start over on a new source: bump the generation and watch it.
    pub fn watch(&self, source: Source) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let bus = self.clone();
        std::thread::Builder::new()
            .name("ingrain-watch".into())
            .spawn(move || {
                let mut last = fingerprint(&source.watched());
                while bus.generation() == generation {
                    std::thread::sleep(Duration::from_millis(800));
                    let now = fingerprint(&source.watched());
                    if now != last {
                        last = now;
                        let ws = source.load();
                        bus.send(Msg::Loaded { generation, ws });
                    }
                }
            })
            .ok();
        generation
    }

    /// Run `f` on a worker thread and send what it returns.
    pub fn spawn(&self, f: impl FnOnce(&Bus) -> Option<Msg> + Send + 'static) {
        let bus = self.clone();
        std::thread::spawn(move || {
            if let Some(m) = f(&bus) {
                bus.send(m);
            }
        });
    }
}

/// Names, sizes and modification times of the watched files and of the files in the watched
/// directories: any write, add or delete changes it.
fn fingerprint(paths: &[PathBuf]) -> Vec<(PathBuf, u64, Option<SystemTime>)> {
    let mut out = Vec::new();
    let mut add = |p: &Path| {
        if let Ok(m) = std::fs::metadata(p) {
            out.push((p.to_path_buf(), m.len(), m.modified().ok()));
        }
    };
    for p in paths {
        add(p);
        if p.is_dir()
            && let Ok(entries) = std::fs::read_dir(p)
        {
            let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
            files.sort();
            for f in files {
                add(&f);
            }
        }
    }
    out
}
