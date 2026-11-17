# ingrain viewer — plan

A native desktop app (Rune) for reading, reviewing and tracing a repository's decisions and
learned rules. The CLI stays the engine; the viewer is a window onto the same files.

## Who opens it, and why

| Person | Moment | What they need in under 10 seconds |
| --- | --- | --- |
| Newcomer | First week on the repo | "What are the rules here, and why?" |
| Reviewer / lead | A PR or a run of `extract` left proposals | "What is waiting for me? Accept or reject, fast." |
| Author of a change | About to edit `crates/cli/src/check.rs` | "What applies to this file?" |
| Archaeologist | "Why on earth do we shell out to `gh`?" | "When was this decided, by whom, what did it replace, what changed since?" |
| Maintainer | Rules drift | "Which rules fire, which get overridden, which are ready to promote?" |

## User stories

Opening

1. **O1** As a developer in a terminal, I run `ingrain viewer` in any folder of a repo and the viewer opens on that repo.
2. **O2** As a developer anywhere, I run `ingrain viewer ~/code/x` and get the repo, or a home screen of recent repos when I run it outside one.
3. **O3** As a Finder user, I drop a folder on the window, or press ⌘O, to open a repo.
4. **O4** As someone with only personal rules, I can browse my global store (`~/.config/ingrain`) without a repo.

Understanding

5. **U1** I see every live decision at a glance: id, sentence, category, scope, age, and whether it is enforced by a check or only told to agents.
6. **U2** I open a decision and read it like a page: the sentence, *why*, *what we didn't choose*, scope, checks, sources, author, date.
7. **U3** I see learned rules with their mode (shadow / warn / block), the before→after fix they came from, and how often they fire.
8. **U4** I filter by store (project / local / global), category and text, from one search field (`/`, ⌘F).

When and how it changed (history)

9. **H1** A timeline shows decisions by the month they were made, so "when did we decide that" is a glance.
10. **H2** A decision shows its lineage: what it superseded and what superseded it, as a chain I can walk.
11. **H3** A decision shows the git commits that touched its file, and each commit's diff, so I see how wording or scope changed and who changed it.
12. **H4** The archive lists superseded decisions with a link to their replacement.

Review (the inbox)

13. **R1** The inbox shows proposals (decisions and rules) with a count in the sidebar.
14. **R2** I accept with one key (`A`), reject with a reason (`R`), skip (`J`/`↓`). The next one slides in.
15. **R3** For a proposed rule I see the backtest (hits on agent code, false alarms on fixed code and kept code) and the example fix as a diff before I accept.
16. **R4** Superseding proposals show old vs new side by side, so I see what I am retiring.

Extract and act

17. **E1** One button runs `decisions extract` (and `rules mine` + `rules propose`) in the background; I watch its output, and new proposals land in the inbox when it finishes.
18. **E2** On a decision I can: supersede it (writes a proposal), copy its agent line, copy its id, open its file in my editor, reveal it in Finder, open its source links.
19. **E3** On a rule I can set its mode with a segmented control.

Scope

20. **S1** "Applies to": I type (or drop) a path and see exactly the decisions and rules whose scope covers it — the same answer agents get from `ingrain decisions for`.

Staying current

21. **C1** When files change on disk (an agent proposes, a teammate pulls), the window updates by itself, without a flash.

## How each is solved

**Shape.** The viewer is part of the `ingrain` binary (D-0001: one binary), in
`crates/cli/src/viewer/`, reading with `ingrain-core` and `ingrain-ledger` (same parsing, same
scope matching). Every write runs this same binary's commands (`decisions accept|reject|supersede`,
`rules accept|reject|mode`, `decisions extract`), one at a time, so there is one implementation of
each write and the viewer can never get the files subtly wrong. `ingrain viewer [PATH]` starts
itself again in the background with `--wait` and returns. It is the default `viewer` Cargo
feature: Rune needs Rust 1.99 (the CLI alone keeps 1.90, D-0033) and adds about 30 MB and 2.5 ms
of startup. On Linux, Rune links libxkbcommon and libwayland-client, which would stop `ingrain`
(hooks included) from starting on machines without them, so Linux release binaries are built
without the feature; Linux desktop users build from source.

**Window.** rune-kit's `SplitView`: sidebar (lenses), list, inspector (detail), on OpenCode's
theme (Inter + IBM Plex Mono, light and dark follow the system).

- Sidebar: repo switcher on top (name + branch; click → recents / open…), then lenses with
  counts: **Inbox**, **Decisions**, **Rules**, **Timeline**, **Applies to**, **Archive**. Footer:
  **Extract…**.
- List: one row per item. Ids in mono, category as a colored dot, scope as chips, relative date,
  a shield when a deterministic check enforces it. Store shown as a small tag (`local`, `global`).
- Inspector: the reading page (U2, H2, H3, E2), actions as a toolbar.

| Story | Solution |
| --- | --- |
| O1, O2 | `ingrain viewer [PATH]` → the window on that repo; outside one → Home with recents (`$INGRAIN_HOME/viewer.json`) |
| O3 | `on_drop_files` and `rune::file_dialog` folder picker, both resolve with `git rev-parse --show-toplevel` |
| O4 | Global store loaded even without a repo; Home shows it |
| U1, U4 | Decisions lens: filter field + store/category chips, list sorted by id |
| U2 | Inspector sections: Why · Not chosen · Scope · Checks · Sources · Meta |
| U3, E3 | Rules lens; inspector shows `SegmentedControl` for mode, example as a `DiffView` (before→after), hit stats from `.git/ingrain/hits.jsonl` |
| H1 | Timeline lens: month headers, a vertical rail with one node per decision, superseded ones dimmed and linked |
| H2, H4 | Lineage strip in the inspector built from `supersedes` / `superseded_by`; Archive lens |
| H3 | History section: `git log --follow` on the record file; click a commit → its patch in a `DiffView` |
| R1–R4 | Inbox lens: one card at a time, keyboard first; reject asks for a reason inline; superseding proposals show the old decision struck through above the new one |
| E1 | Extract sheet: runs the CLI on a worker, streams its output, refreshes on exit |
| E2 | Supersede opens an inline form (decision, reason) → `ingrain decisions supersede`; copy via `Tx::copy`; open via `open`/`xdg-open`/`start` |
| S1 | Applies-to lens: path field → `Scope` matching from `ingrain-ledger` + rule globs |
| C1 | A watcher thread compares directory mtimes every second and posts a reload through `AppHandle::update` (no file I/O on the UI thread) |

Keyboard: `⌘1…6` lenses, `/` search, `↑↓` move, `⌘K` command palette over everything,
`A`/`R` in the inbox, `⌘O` open repo, `⇧⌘B` sidebar.

## Out of scope for v1

Editing a decision's text in place (the CLI's `supersede` is the safe path), multi-repo
dashboards, Windows URL scheme registration, a signed `.app` bundle.

## Decisions this follows

- D-0001 (a single binary named `ingrain`): the viewer is a subcommand, not a second program.
- D-0008 (the review prompt is line-based, no TUI framework): `decisions review` and `rules review`
  stay as they are; the viewer is a separate, optional window.
