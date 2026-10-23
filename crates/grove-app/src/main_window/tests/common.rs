use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, Bounds, InputEvent, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels,
    Point, TestAppContext, WindowBounds, WindowOptions, point, px, size,
};
use grove_core::{
    Change, ChangedFile, GitError, Outcome, Presence, RepoSnapshot, RowId, RowKind, Settings,
    SettingsFile, SortMode, Upstream, WorktreeStatus,
};
use tempfile::TempDir;

use super::super::layout::SIDEBAR_W;
use super::super::window::open_with;
use super::super::*;
use crate::shell::Library;
use crate::test_support::{
    Launches, canned, fake_launcher, fake_presence, indexed_labels, set_library, settings_file,
};
use crate::{pull, removal};

/// Launches the main window over `file`'s settings and the canned snapshot.
pub(crate) fn launch(
    cx: &mut TestAppContext,
    file: &SettingsFile,
) -> (AnyWindowHandle, Entity<MainWindow>) {
    launch_repo(cx, file, canned())
}

/// `launch`, with a caller-supplied repo snapshot.
pub(crate) fn launch_repo(
    cx: &mut TestAppContext,
    file: &SettingsFile,
    repo: grove_core::RepoSnapshot,
) -> (AnyWindowHandle, Entity<MainWindow>) {
    launch_at(cx, file, repo, size(px(1180.), px(720.)))
}

/// `launch_repo`, with a caller-supplied window size.
pub(crate) fn launch_at(
    cx: &mut TestAppContext,
    file: &SettingsFile,
    repo: grove_core::RepoSnapshot,
    window: gpui_kit::Size<gpui_kit::Pixels>,
) -> (AnyWindowHandle, Entity<MainWindow>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::details_view::init(cx);
        set_library(file, vec![repo], cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: window,
            })),
            ..Default::default()
        };
        open_with(options, cx).unwrap()
    })
}

/// Whether the tree shows a collapsed folder row for `prefix`.
pub(crate) fn folder_collapsed(
    cx: &mut TestAppContext,
    main: &Entity<MainWindow>,
    prefix: &str,
) -> bool {
    main.read_with(cx, |m, cx| {
        m.tree.read(cx).tree().rows.iter().any(|r| {
            r.id == RowId::Folder(prefix.into())
                && matches!(
                    r.kind,
                    RowKind::Folder {
                        collapsed: true,
                        ..
                    }
                )
        })
    })
}

/// Resizes the details pane (panel 1) through the shared panel state, then lets the
/// frame -- and, via `Resized`, the delayed settings write -- land.
pub(crate) fn resize_pane(
    cx: &mut TestAppContext,
    main: &Entity<MainWindow>,
    w: AnyWindowHandle,
    width: f32,
) {
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        let panes = main.read(cx).panes.clone();
        panes.update(cx, |state, cx| state.resize_panel(1, px(width), window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(w, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

/// Drags the tree/pane divider by `dx` px (positive = rightwards) at mouse rate.
pub(crate) fn drag_divider(cx: &mut TestAppContext, w: AnyWindowHandle, dx: f32) {
    cx.update_window(w, |_, window, cx| {
        // The handle straddles the pane's left edge; the element itself is
        // not observed, so start the drag at that edge (inside the band).
        let pane = window.find("details").bounds();
        let from = pane.origin;
        let to = Point {
            x: from.x - px(dx),
            y: from.y + pane.size.height / 2.,
        };
        window.drag(from, to, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// The details pane's rendered width.
pub(crate) fn pane_width(cx: &mut TestAppContext, w: AnyWindowHandle) -> f32 {
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        let b = window.find("details").bounds();
        (b.right() - b.left()).as_f32()
    })
    .unwrap()
}

/// The tree panel's width: from the sidebar's edge (where the
/// group starts) to the pane divider.
pub(crate) fn tree_panel_width(cx: &mut TestAppContext, w: AnyWindowHandle) -> f32 {
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        let pane = window.find("details").bounds();
        pane.left().as_f32() - SIDEBAR_W
    })
    .unwrap()
}

/// Asserts `got` sits within 1 px of `want`.
pub(crate) fn assert_close(got: Pixels, want: f32, what: &str) {
    assert!(
        (got.as_f32() - want).abs() < 1.,
        "{what}: expected {want} px, got {} px",
        got.as_f32()
    );
}

/// Presses the divider, moves the pointer in steps, leaving the drag open
/// (`released: false`): the events a real drag delivers, split so the middle
/// can be observed. A press that never moved is no-op for the pane.
pub(crate) fn pressed_divider_drag(
    cx: &mut TestAppContext,
    w: AnyWindowHandle,
    from: Point<Pixels>,
    to: Point<Pixels>,
) {
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window.dispatch_event(
            MouseMoveEvent {
                position: from,
                pressed_button: None,
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(
            MouseDownEvent {
                button: MouseButton::Left,
                position: from,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        for step in 1..=8 {
            let fraction = step as f32 / 8.;
            window.dispatch_event(
                MouseMoveEvent {
                    position: point(
                        from.x + (to.x - from.x) * fraction,
                        from.y + (to.y - from.y) * fraction,
                    ),
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
        }
    })
    .unwrap();
}

/// Presses `keys` from the tree and parks the executor.
pub(crate) fn keys(cx: &mut TestAppContext, w: AnyWindowHandle, keys: &[&str]) {
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        for k in keys {
            window.press(k, cx);
        }
    })
    .unwrap();
    cx.run_until_parked();
}

/// Whether the (`id`, `ix`) element rendered.
pub(crate) fn has(
    cx: &mut TestAppContext,
    w: AnyWindowHandle,
    id: &'static str,
    ix: usize,
) -> bool {
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window.try_find((id, ix)).is_some()
    })
    .unwrap()
}

/// The main window's current status line.
pub(crate) fn status(cx: &mut TestAppContext, main: &Entity<MainWindow>) -> Option<Status> {
    main.read_with(cx, |m, _| m.status.clone())
}

/// Settings file with two custom commands and `default_editor`.
pub(crate) fn with_commands(tmp: &TempDir, default_editor: &str) -> SettingsFile {
    let cmd = |label: &str, template: &str, run_in_terminal| grove_core::CustomCommand {
        label: label.into(),
        template: template.into(),
        run_in_terminal,
    };
    settings_file(
        tmp,
        Settings {
            default_editor: Some(default_editor.into()),
            custom_commands: vec![
                cmd("Neovim", "nvim {path}", true),
                cmd("Sublime", "subl -n {path}", false),
            ],
            ..Default::default()
        },
    )
}

/// The badges on the worktree row at `path`.
pub(crate) fn row_badges(
    cx: &mut TestAppContext,
    main: &Entity<MainWindow>,
    path: &str,
) -> Vec<String> {
    main.read_with(cx, |m, cx| {
        m.tree
            .read(cx)
            .tree()
            .rows
            .iter()
            .find_map(|r| match &r.kind {
                RowKind::Worktree {
                    path: p, badges, ..
                } if p == Path::new(path) => Some(badges.iter().map(|b| b.label()).collect()),
                _ => None,
            })
            .unwrap()
    })
}

/// Status after ⌘Enter on `main` when the executor returns `outcome`.
pub(crate) fn terminal_status(
    cx: &mut TestAppContext,
    settings: Settings,
    outcome: Outcome,
) -> Status {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, settings);
    fake_launcher(cx, Ok(outcome));
    let (w, main) = launch(cx, &file);
    keys(cx, w, &["cmd-enter"]);
    status(cx, &main).unwrap()
}

/// `n` modified files, f0.txt through f{n-1}.txt.
pub(crate) fn changed_files(n: usize) -> Vec<ChangedFile> {
    (0..n)
        .map(|i| ChangedFile {
            change: Change::Modified,
            path: format!("f{i}.txt"),
            from: None,
        })
        .collect()
}

/// Clicks `id` and parks the executor.
pub(crate) fn click(
    cx: &mut TestAppContext,
    w: AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
) {
    let id = id.into();
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// Whether the element with `id` rendered.
pub(crate) fn found(cx: &mut TestAppContext, w: AnyWindowHandle, id: &'static str) -> bool {
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some()
    })
    .unwrap()
}

/// The details pane's title and kind line.
pub(crate) fn details(cx: &mut TestAppContext, main: &Entity<MainWindow>) -> (String, String) {
    main.read_with(cx, |m, cx| {
        let d = m.details.read(cx).details().cloned().unwrap();
        (d.title, d.kind)
    })
}

/// A (title, kind) pair.
pub(crate) fn pair(title: &str, kind: &str) -> (String, String) {
    (title.into(), kind.into())
}

/// Selects `/c/wt/api-x` with `presence` and tries every open control in the pane.
pub(crate) fn try_details_opens(cx: &mut TestAppContext, presence: Presence) -> (Launches, Status) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    let launches = fake_launcher(cx, Ok(Outcome::Opened));
    fake_presence(cx, "/c/wt/api-x", presence);
    let mut repo = canned();
    repo.worktrees[1].presence = presence;
    let (w, main) = launch_repo(cx, &file, repo);
    keys(cx, w, &["down", "down", "down", "down"]);
    for id in [
        "details-open-editor",
        "details-menu-editor",
        "details-open-terminal",
        "details-menu-terminal",
    ] {
        click(cx, w, id);
    }
    assert!(!found(cx, w, "editor-menu") && !found(cx, w, "terminal-menu"));
    click(cx, w, "details-reveal");
    (launches, status(cx, &main).unwrap())
}

/// Runs `git args` in `dir` with a throwaway identity, asserting success.
pub(crate) fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=T", "-c", "user.email=t@t.invalid"])
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Whether the switcher window is open.
pub(crate) fn switcher_open(cx: &mut TestAppContext) -> bool {
    cx.update(|cx| crate::switcher::window(cx).is_some_and(|w| cx.windows().contains(&w)))
}

/// Launches with the canned repo plus a one-worktree `web` repo.
pub(crate) fn two_repos(cx: &mut TestAppContext, file: &SettingsFile) -> AnyWindowHandle {
    let mut web = canned();
    web.display_name = "web".into();
    web.repo.name = "web".into();
    web.repo.path = "/c/web".into();
    web.repo.git_dir = "/c/web/.git".into();
    web.worktrees.truncate(1);
    web.worktrees[0].path = "/c/web".into();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::details_view::init(cx);
        set_library(file, vec![canned(), web], cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(1180.), px(720.)),
            })),
            ..Default::default()
        };
        open_with(options, cx).unwrap().0
    })
}

/// The element's label as rendered.
pub(crate) fn label_of(
    cx: &mut TestAppContext,
    w: AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
) -> String {
    let id = id.into();
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window.find(id).label().unwrap().to_string()
    })
    .unwrap()
}

/// The sidebar's header labels that rendered.
pub(crate) fn header_texts(cx: &mut TestAppContext, w: AnyWindowHandle) -> Vec<String> {
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        ["pinned-header", "repos-header"]
            .into_iter()
            .filter_map(|id| Some(window.try_find(id)?.label()?.to_string()))
            .collect()
    })
    .unwrap()
}

/// The manage popover's rows: every repo, in sidebar order, with its hidden state.
pub(crate) fn visibility_rows(cx: &mut TestAppContext, w: AnyWindowHandle) -> Vec<String> {
    indexed_labels(cx, w, "visibility-row")
}

/// Clicks the manage button, asserting the popover opened.
pub(crate) fn open_visibility(cx: &mut TestAppContext, w: AnyWindowHandle) {
    click(cx, w, "manage-visibility");
    assert!(found(cx, w, "visibility-menu"));
}

/// A one-worktree repo at `/c/<name>`.
pub(crate) fn repo_named(name: &str) -> grove_core::RepoSnapshot {
    let mut repo = canned();
    repo.display_name = name.into();
    repo.repo.name = name.into();
    repo.repo.path = format!("/c/{name}").into();
    repo.repo.git_dir = format!("/c/{name}/.git").into();
    repo.worktrees.truncate(1);
    repo.worktrees[0].path = format!("/c/{name}").into();
    repo
}

/// Launches the main window over caller-supplied repos and window size.
pub(crate) fn launch_repos(
    cx: &mut TestAppContext,
    file: &SettingsFile,
    repos: Vec<grove_core::RepoSnapshot>,
    window: gpui_kit::Size<gpui_kit::Pixels>,
) -> (AnyWindowHandle, Entity<MainWindow>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::details_view::init(cx);
        set_library(file, repos, cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: window,
            })),
            ..Default::default()
        };
        open_with(options, cx).unwrap()
    })
}

/// A status read for `/c/<repo>` landing with HEAD committed at `head_time`.
pub(crate) fn commit_lands(cx: &mut TestAppContext, repo: &str, head_time: i64) {
    cx.update(|cx| {
        let store = &mut cx.global_mut::<Library>().store;
        let stamp = store.status_reads().stamp();
        store.apply_status(
            stamp,
            Path::new(&format!("/c/{repo}")),
            Ok(WorktreeStatus {
                files: vec![],
                upstream: Upstream::None,
                head: Some(format!("{head_time:x}")),
                head_time: Some(head_time),
                changes_time: None,
            }),
        );
    });
    cx.run_until_parked();
}

/// The first `n` repo rows' labels.
pub(crate) fn sidebar(cx: &mut TestAppContext, w: AnyWindowHandle, n: usize) -> Vec<String> {
    (0..n).map(|ix| label_of(cx, w, ("repo", ix))).collect()
}

/// Branch rows top to bottom.
pub(crate) fn tree_branches(cx: &mut TestAppContext, main: &Entity<MainWindow>) -> Vec<String> {
    main.read_with(cx, |m, cx| {
        m.tree
            .read(cx)
            .tree()
            .rows
            .iter()
            .filter_map(|r| match &r.kind {
                RowKind::Branch { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect()
    })
}

/// Index of the tree row for the worktree at `path`.
pub(crate) fn row_of(cx: &mut TestAppContext, main: &Entity<MainWindow>, path: &Path) -> usize {
    main.read_with(cx, |m, cx| {
        m.tree
            .read(cx)
            .tree()
            .index_of(&RowId::Worktree(path.into()))
            .unwrap_or_else(|| panic!("no row for {}", path.display()))
    })
}

/// Right-clicks `id` and parks the executor.
pub(crate) fn right_click(
    cx: &mut TestAppContext,
    w: AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
) {
    let id = id.into();
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window.right_click(id, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// Each removal's path and whether it was forced.
pub(crate) type Removals = Arc<Mutex<Vec<(PathBuf, bool)>>>;

/// Records each removal, then runs it for real (temp repos only) or fails with `fail`.
pub(crate) fn fake_remover(cx: &mut TestAppContext, fail: Option<&'static str>) -> Removals {
    fake_remover_with(cx, fail, |_| {})
}

/// `fake_remover`, calling `before` with the worktree first.
pub(crate) fn fake_remover_with(
    cx: &mut TestAppContext,
    fail: Option<&'static str>,
    before: impl Fn(&Path) + Send + Sync + 'static,
) -> Removals {
    let calls: Removals = Default::default();
    let sink = calls.clone();
    cx.update(|cx| {
        cx.set_global(removal::Remover {
            run: Arc::new(move |repo, path, force| {
                sink.lock().unwrap().push((path.to_path_buf(), force));
                before(path);
                match fail {
                    Some(stderr) => Err(GitError::Failed {
                        args: "worktree remove".into(),
                        stderr: stderr.into(),
                    }),
                    None => removal::run(repo, path, force),
                }
            }),
        })
    });
    calls
}

/// Real repo `api` in `tmp/code` with clean worktrees `a` and `c` and `b` holding one
/// modified and two untracked files, under `tmp/wt`. Trees sort by name.
pub(crate) fn removal_fixture(
    cx: &mut TestAppContext,
    tmp: &TempDir,
    settings: Settings,
) -> (AnyWindowHandle, Entity<MainWindow>, SettingsFile, PathBuf) {
    removal_fixture_with(cx, tmp, settings, |_| {})
}

/// `removal_fixture`, with `before_scan` given the worktrees' folder.
pub(crate) fn removal_fixture_with(
    cx: &mut TestAppContext,
    tmp: &TempDir,
    settings: Settings,
    before_scan: impl FnOnce(&Path),
) -> (AnyWindowHandle, Entity<MainWindow>, SettingsFile, PathBuf) {
    let file = settings_file(
        tmp,
        Settings {
            tree_sort: SortMode::Name,
            ..settings
        },
    );
    fake_launcher(cx, Ok(Outcome::Opened));
    let code = tmp.path().join("code");
    let api = code.join("api");
    std::fs::create_dir_all(&api).unwrap();
    git(&api, &["init", "-q"]);
    std::fs::write(api.join("readme"), "hi\n").unwrap();
    git(&api, &["add", "readme"]);
    git(&api, &["commit", "-q", "-m", "one"]);
    let wt = tmp.path().join("wt");
    for b in ["a", "b", "c"] {
        let dir = wt.join(b);
        git(
            &api,
            &["worktree", "add", "-q", "-b", b, dir.to_str().unwrap()],
        );
    }
    let b = wt.join("b");
    std::fs::write(b.join("readme"), "changed\n").unwrap();
    std::fs::create_dir(b.join("new")).unwrap();
    std::fs::write(b.join("new/one"), "1").unwrap();
    std::fs::write(b.join("new/two"), "2").unwrap();
    before_scan(&wt);
    let repo = grove_core::scan(&code).repos.remove(0);
    let wt = repo.worktrees[1].path.parent().unwrap().to_path_buf();
    let (w, main) = launch_repo(cx, &file, repo);
    // The confirm's slide-in otherwise moves its buttons between frames.
    cx.update(|cx| cx.set_reduce_motion(true));
    (w, main, file, wt)
}

/// The tree's selected row id.
pub(crate) fn selected_row(cx: &mut TestAppContext, main: &Entity<MainWindow>) -> Option<RowId> {
    main.read_with(cx, |m, cx| m.tree.read(cx).selected().cloned())
}

/// Whether the element with `id` holds focus.
pub(crate) fn focused(cx: &mut TestAppContext, w: AnyWindowHandle, id: &'static str) -> bool {
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window.find(id).focused() == Some(true)
    })
    .unwrap()
}

/// Each pull's worktree path.
pub(crate) type Pulls = Arc<Mutex<Vec<PathBuf>>>;

/// Records each pull, running git with `perform`.
pub(crate) fn puller_with(
    cx: &mut TestAppContext,
    perform: impl Fn(&Path) -> Result<(), GitError> + Send + Sync + 'static,
) -> Pulls {
    let calls: Pulls = Default::default();
    let sink = calls.clone();
    cx.update(|cx| {
        cx.set_global(pull::Puller {
            run: Arc::new(move |path| {
                sink.lock().unwrap().push(path.to_path_buf());
                perform(path)
            }),
        })
    });
    calls
}

/// Records each pull that fails with `fail`'s stderr. Canned paths have no git behind
/// them, so a pull that succeeds is a no-op; the fixture test installs the real one.
pub(crate) fn fake_puller(cx: &mut TestAppContext, fail: Option<&'static str>) -> Pulls {
    puller_with(cx, move |_| match fail {
        Some(stderr) => Err(GitError::Failed {
            args: "pull --ff-only".into(),
            stderr: stderr.into(),
        }),
        None => Ok(()),
    })
}

/// Clicks `id` without parking the executor, so background tasks can't finish yet.
pub(crate) fn click_now(
    cx: &mut TestAppContext,
    w: AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
) {
    let id = id.into();
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    })
    .unwrap();
}

/// `id`'s label as rendered, without parking the executor.
pub(crate) fn label_now(cx: &mut TestAppContext, w: AnyWindowHandle, id: &'static str) -> String {
    label_of(cx, w, id)
}

/// Clicks `id` `clicks` times and reads its label back, all before the background task
/// a parked executor would finish may run: the pane's synchronous state is asserted.
pub(crate) fn click_and_look(
    cx: &mut TestAppContext,
    w: AnyWindowHandle,
    id: &'static str,
    clicks: usize,
) -> String {
    for _ in 0..clicks {
        click_now(cx, w, id);
    }
    label_now(cx, w, id)
}

/// A successful status read carrying only `upstream`.
pub(crate) fn upstream_status(upstream: Upstream) -> Option<Result<WorktreeStatus, GitError>> {
    Some(Ok(WorktreeStatus {
        files: vec![],
        upstream,
        head: None,
        head_time: None,
        changes_time: None,
    }))
}

/// Canned repo with per-worktree upstreams: main behind 1, api-x up to date, api-y with
/// no upstream, api-z gone, bisect detached.
pub(crate) fn pull_repo() -> RepoSnapshot {
    let mut repo = canned();
    let set = |repo: &mut RepoSnapshot, ix: usize, upstream| {
        repo.worktrees[ix].status = upstream_status(upstream);
    };
    set(
        &mut repo,
        0,
        Upstream::Tracking {
            name: "origin/main".into(),
            ahead: 0,
            behind: 1,
        },
    );
    set(
        &mut repo,
        1,
        Upstream::Tracking {
            name: "origin/feature/x".into(),
            ahead: 0,
            behind: 0,
        },
    );
    set(&mut repo, 2, Upstream::None);
    set(
        &mut repo,
        3,
        Upstream::Gone {
            name: "origin/fix/z".into(),
        },
    );
    set(&mut repo, 4, Upstream::None);
    repo
}

/// The details pane's Sync line, as the status fields show it.
pub(crate) fn sync_line(cx: &mut TestAppContext, main: &Entity<MainWindow>) -> String {
    main.read_with(cx, |m, cx| {
        m.details
            .read(cx)
            .status_fields()
            .into_iter()
            .find_map(|(l, v)| (l == "Sync").then_some(v))
            .unwrap()
    })
}

/// Records each pull and runs git for real.
pub(crate) fn real_puller(cx: &mut TestAppContext) -> Pulls {
    puller_with(cx, pull::run)
}

// Branch switch trigger (ticket 25).

/// Each switch's (worktree path, target branch).
pub(crate) type Switches = Arc<Mutex<Vec<(PathBuf, String)>>>;

/// Records each switch, running git with `perform`.
pub(crate) fn switcher_with(
    cx: &mut TestAppContext,
    perform: impl Fn(&Path, &str) -> Result<(), GitError> + Send + Sync + 'static,
) -> Switches {
    let calls: Switches = Default::default();
    let sink = calls.clone();
    cx.update(|cx| {
        cx.set_global(crate::branch::SwitchRunner {
            run: Arc::new(move |path, target| {
                sink.lock()
                    .unwrap()
                    .push((path.to_path_buf(), target.to_string()));
                perform(path, target)
            }),
        })
    });
    calls
}

/// Records each switch that fails with `fail`'s stderr.
pub(crate) fn fake_switcher(cx: &mut TestAppContext, fail: Option<&'static str>) -> Switches {
    switcher_with(cx, move |_, _| match fail {
        Some(stderr) => Err(GitError::Failed {
            args: "switch --quiet".into(),
            stderr: stderr.into(),
        }),
        None => Ok(()),
    })
}

/// Fakes the branch-list read with `list`.
pub(crate) fn lister(cx: &mut TestAppContext, list: &'static [&'static str]) {
    cx.update(|cx| {
        cx.set_global(crate::branch::ListRunner {
            run: Arc::new(move |_| Ok(list.iter().map(|branch| branch.to_string()).collect())),
        })
    });
}

/// Fakes the branch-list read with a failed read carrying `stderr`.
pub(crate) fn failing_lister(cx: &mut TestAppContext, stderr: &'static str) {
    cx.update(|cx| {
        cx.set_global(crate::branch::ListRunner {
            run: Arc::new(move |_| {
                Err(GitError::Failed {
                    args: "for-each-ref refs/heads".into(),
                    stderr: stderr.into(),
                })
            }),
        })
    });
}

/// Selects the worktree row at `path` and parks the executor.
pub(crate) fn select_worktree(
    cx: &mut TestAppContext,
    w: AnyWindowHandle,
    main: &Entity<MainWindow>,
    path: &str,
) {
    let ix = row_of(cx, main, Path::new(path));
    click(cx, w, ("tree-row", ix));
    cx.run_until_parked();
}

/// The switch write in flight: which worktree, to what branch.
pub(crate) fn switching(
    cx: &mut TestAppContext,
    main: &Entity<MainWindow>,
) -> Option<(PathBuf, String)> {
    main.read_with(cx, |m, cx| {
        m.details
            .read(cx)
            .switching()
            .map(|(p, t)| (p.to_path_buf(), t.to_string()))
    })
}

/// What the trigger reads: the committed branch, else the placeholder.
pub(crate) fn trigger_text(cx: &mut TestAppContext, main: &Entity<MainWindow>) -> Option<String> {
    main.read_with(cx, |m, cx| m.details.read(cx).branch_trigger_text(cx))
}

/// The dropdown's rows in order, as their accessible labels.
pub(crate) fn dropdown(cx: &mut TestAppContext, w: AnyWindowHandle) -> Vec<String> {
    indexed_labels(cx, w, "branch-item")
}

/// The branches shown as tree rows for the pane's repo.
pub(crate) fn tree_rows_branches(
    cx: &mut TestAppContext,
    main: &Entity<MainWindow>,
) -> Vec<String> {
    main.read_with(cx, |m, cx| {
        m.tree
            .read(cx)
            .tree()
            .rows
            .iter()
            .filter_map(|r| match &r.id {
                RowId::Branch(name) => Some(name.clone()),
                _ => None,
            })
            .collect()
    })
}
