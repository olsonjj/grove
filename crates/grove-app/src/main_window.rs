use std::path::Path;

use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyWindowHandle, App, AppContext, Bounds, Context, Entity, Focusable, InteractiveElement,
    IntoElement, ParentElement, Render, StatefulInteractiveElement, Styled, Subscription,
    TitlebarOptions, Window, WindowBounds, WindowKind, WindowOptions, div, px, size,
};
use grove_core::Presence;

use crate::details_view::{self, DetailsView};
use crate::tree_view::{CollapseChanged, OpenRequested, TreeView};
use crate::{launcher, shell, theme};

const SHORTCUTS: &str = "↑↓ Move   ←→ Collapse / expand   ↵ Editor   ⌘↵ Terminal";

/// The status bar's confirmation or failure line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub text: String,
    pub error: bool,
    /// Fixable in settings: show the settings link.
    pub settings: bool,
}

struct MainWindow {
    tree: Entity<TreeView>,
    details: Entity<DetailsView>,
    status: Option<Status>,
    /// Bumped per open or notice so a slow earlier launch can't overwrite a newer status.
    open_seq: u64,
    _subs: [Subscription; 6],
}

/// The saved repo if it still exists, else the first.
fn selected_repo(cx: &App) -> Option<String> {
    let library = shell::library(cx);
    let snapshot = library.store.snapshot();
    library
        .settings
        .last_selected_repo
        .as_deref()
        .and_then(|name| snapshot.repo(name))
        .or_else(|| snapshot.repos.first())
        .map(|r| r.name().to_string())
}

impl MainWindow {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let tree = cx.new(TreeView::new);
        let collapse = cx.subscribe(&tree, |_, _, ev: &CollapseChanged, cx| {
            shell::set_collapsed(&ev.repo, &ev.prefix, ev.collapsed, cx);
        });
        let open = cx.subscribe(&tree, |this, _, ev: &OpenRequested, cx| {
            this.open(ev, cx);
        });
        let details = cx.new(|cx| DetailsView::new(tree.clone(), cx));
        let details_open = cx.subscribe(&details, |this, _, ev: &OpenRequested, cx| {
            this.open(ev, cx);
        });
        let notice = cx.subscribe(&details, |this, _, ev: &details_view::Notice, cx| {
            this.open_seq += 1;
            this.set_status(ev.text.clone(), ev.error, false, cx);
        });
        let library = cx.observe_global::<shell::Library>(|this, cx| this.sync_repo(cx));
        let activation = cx.observe_window_activation(window, |_, window, cx| {
            if window.is_window_active() {
                shell::refresh_all(cx);
            }
        });
        window.focus(&tree.focus_handle(cx), cx);
        let mut this = Self {
            tree,
            details,
            status: None,
            open_seq: 0,
            _subs: [collapse, open, details_open, notice, library, activation],
        };
        this.show_repo(selected_repo(cx), cx);
        if std::env::var_os("GROVE_DEV_OPEN").is_some() {
            this.dev_open(cx);
        }
        this
    }

    /// Dev: opens the first worktree once editors are detected, so the inline button and
    /// status bar can be checked without synthetic input.
    fn dev_open(&self, cx: &mut Context<Self>) {
        let tree = self.tree.clone();
        cx.spawn(async move |_, cx| {
            while !cx.update(|cx| launcher::ready(cx)) {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;
            }
            tree.update(cx, |tree, cx| tree.dev_open_first(cx));
        })
        .detach();
    }

    /// Pushes fresher snapshot data for the shown repo into the tree.
    fn sync_repo(&mut self, cx: &mut Context<Self>) {
        if !cx.has_global::<shell::Library>() {
            return;
        }
        let name = self.tree.read(cx).repo_name().map(str::to_string);
        let snapshot = shell::library(cx).store.snapshot().clone();
        let repo = name.as_deref().and_then(|n| snapshot.repo(n));
        self.tree.update(cx, |tree, cx| tree.update_repo(repo, cx));
        cx.notify();
    }

    fn show_repo(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        let library = shell::library(cx);
        let repo = name
            .as_deref()
            .and_then(|n| library.store.snapshot().repo(n))
            .cloned();
        let collapsed = name
            .as_deref()
            .map(|n| library.settings.collapsed(n).to_vec())
            .unwrap_or_default();
        self.tree
            .update(cx, |tree, cx| tree.set_repo(repo, collapsed, cx));
    }
}

impl MainWindow {
    fn set_status(&mut self, text: String, error: bool, settings: bool, cx: &mut Context<Self>) {
        self.status = Some(Status {
            text,
            error,
            settings,
        });
        cx.notify();
    }

    fn open(&mut self, req: &OpenRequested, cx: &mut Context<Self>) {
        self.open_seq += 1;
        let seq = self.open_seq;
        // Presence was read at scan time; the folder may have gone or come back since.
        let presence = launcher::presence(&req.path, cx);
        if presence != req.presence {
            cx.global_mut::<shell::Library>()
                .store
                .set_presence(&req.path, presence);
            if presence == Presence::Present {
                shell::read_status(&req.path, cx);
            }
        }
        if let Some(reason) = presence.reason() {
            let text = format!("Can't open {}: {reason}", tilde(&req.path));
            return self.set_status(text, true, false, cx);
        }
        match launcher::open(req.role, req.target, &req.path, cx) {
            Err(e) => self.set_status(e.message, true, e.settings, cx),
            Ok(opening) => {
                self.set_status(opening.message, false, false, cx);
                let follow_up = opening.follow_up;
                cx.spawn(async move |this, cx| {
                    let (text, warn) = match opening.done.await {
                        Ok(outcome) => {
                            eprintln!("[grove] launch: {outcome:?}");
                            match follow_up(&outcome) {
                                Some(n) => (n.text, n.warn),
                                None => return,
                            }
                        }
                        Err(e) => {
                            eprintln!("[grove] launch failed: {e}");
                            (e.to_string(), true)
                        }
                    };
                    let _ = this.update(cx, |this, cx| {
                        if this.open_seq == seq {
                            this.set_status(text, warn, false, cx);
                        }
                    });
                })
                .detach();
            }
        }
    }

    fn status_bar(&self) -> impl IntoElement {
        let line = match &self.status {
            Some(s) => div()
                .flex()
                .min_w_0()
                .gap(px(10.))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(if s.error {
                            theme::warn_text()
                        } else {
                            theme::text_soft()
                        })
                        .child(s.text.clone()),
                )
                // Placeholder until the settings window exists (ticket 13).
                .when(s.settings, |d| {
                    d.child(
                        div()
                            .flex_none()
                            .text_color(theme::link())
                            .child("Settings…"),
                    )
                }),
            None => div(),
        };
        div()
            .id("status-bar")
            .flex_none()
            .h(px(28.))
            .px(px(14.))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(16.))
            .border_t_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .text_size(px(12.))
            .child(line)
            .child(
                div()
                    .flex_none()
                    .text_color(theme::faint())
                    .child(SHORTCUTS),
            )
    }
}

impl Render for MainWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let library = shell::library(cx);
        let snapshot = library.store.snapshot().clone();
        let selected = selected_repo(cx);

        let rows = snapshot.repos.iter().enumerate().map(|(i, repo)| {
            let is_selected = selected.as_deref() == Some(repo.name());
            let name = repo.name().to_string();
            div()
                .id(("repo", i))
                .h(px(34.))
                .px(px(10.))
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.))
                .rounded(px(6.))
                .text_size(px(13.))
                .cursor_pointer()
                .when_else(
                    is_selected,
                    |d| d.bg(theme::selected()).text_color(theme::text()),
                    |d| {
                        d.text_color(theme::text_soft())
                            .hover(|s| s.bg(theme::raised()))
                    },
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    shell::select_repo(&name, cx);
                    if this.tree.read(cx).repo_name() != Some(name.as_str()) {
                        this.show_repo(Some(name.clone()), cx);
                    }
                    window.focus(&this.tree.focus_handle(cx), cx);
                    cx.notify();
                }))
                .child(theme::mono(repo.display_name.clone()).truncate())
                .child(
                    div()
                        .flex_none()
                        .text_size(px(11.))
                        .text_color(theme::link())
                        .child(format!("{} wt", repo.worktree_count())),
                )
        });

        let sidebar = div()
            .w(px(240.))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .bg(theme::sidebar())
            .border_r_1()
            .border_color(theme::border())
            .child(
                div()
                    .px(px(18.))
                    .pt(px(14.))
                    .pb(px(8.))
                    .text_size(px(11.))
                    .text_color(theme::faint())
                    .child(format!("REPOSITORIES · {}", snapshot.repos.len())),
            )
            .child(
                div()
                    .id("repo-list")
                    .flex_1()
                    .overflow_y_scroll()
                    .px(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .children(rows),
            )
            .child(
                div()
                    .m(px(10.))
                    .p(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::bg())
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme::faint())
                            .child("CODE FOLDER"),
                    )
                    .child(
                        theme::path_text(tilde(&snapshot.code_dir))
                            .text_size(px(13.))
                            .text_color(theme::text()),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(theme::faint())
                            .child("Grove only looks inside this folder."),
                    ),
            );

        let content = if snapshot.repos.is_empty() {
            div()
                .p(px(16.))
                .text_color(theme::muted())
                .child("No repos")
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .child(self.tree.clone())
                .child(self.details.clone())
                .into_any_element()
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::bg())
            .text_color(theme::text())
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(sidebar)
                    .child(content),
            )
            .child(self.status_bar())
    }
}

/// `/Users/me/code` → `~/code`.
pub fn tilde(path: &Path) -> String {
    grove_core::tilde(path, grove_core::home_dir().as_deref())
}

const MIN_SIZE: gpui_kit::Size<gpui_kit::Pixels> = size(px(1000.), px(560.));

pub fn open(cx: &mut App) -> Option<AnyWindowHandle> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1180.), px(720.)),
            cx,
        ))),
        window_min_size: Some(MIN_SIZE),
        titlebar: Some(TitlebarOptions {
            title: Some("Grove".into()),
            ..Default::default()
        }),
        focus: true,
        show: true,
        kind: WindowKind::Normal,
        ..Default::default()
    };
    open_with(options, cx)
        .map(|(handle, _)| handle)
        .inspect_err(|e| eprintln!("[grove] main: {e:#}"))
        .ok()
}

fn open_with(
    options: WindowOptions,
    cx: &mut App,
) -> gpui_kit::Result<(AnyWindowHandle, Entity<MainWindow>)> {
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| MainWindow::new(window, cx))
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use std::sync::Mutex;

    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{Point, TestAppContext};
    use grove_core::{
        Change, ChangedFile, Editor, Fallback, LaunchError, LaunchPlan, OpenTarget, Outcome,
        Permission, Presence, RowId, RowKind, Settings, SettingsFile, Snapshot, SnapshotStore,
        TargetKind, Terminal, TerminalOpensIn, Upstream, WorktreeStatus,
    };
    use tempfile::TempDir;

    use super::*;
    use crate::launcher::Launcher;
    use crate::shell::Library;
    use crate::tree_view::tests::canned;

    fn launch(
        cx: &mut TestAppContext,
        file: &SettingsFile,
    ) -> (AnyWindowHandle, Entity<MainWindow>) {
        launch_repo(cx, file, canned())
    }

    fn launch_repo(
        cx: &mut TestAppContext,
        file: &SettingsFile,
        repo: grove_core::RepoSnapshot,
    ) -> (AnyWindowHandle, Entity<MainWindow>) {
        launch_at(cx, file, repo, size(px(1180.), px(720.)))
    }

    fn launch_at(
        cx: &mut TestAppContext,
        file: &SettingsFile,
        repo: grove_core::RepoSnapshot,
        window: gpui_kit::Size<gpui_kit::Pixels>,
    ) -> (AnyWindowHandle, Entity<MainWindow>) {
        let settings = file.load().unwrap();
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Library {
                settings_file: file.clone(),
                settings,
                store: SnapshotStore::from_snapshot(Snapshot {
                    code_dir: PathBuf::from("/c"),
                    code_dir_exists: true,
                    repos: vec![repo],
                }),
                saves: Default::default(),
            });
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

    fn folder_collapsed(cx: &mut TestAppContext, main: &Entity<MainWindow>, prefix: &str) -> bool {
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

    #[gpui_kit::test]
    fn collapse_by_key_is_saved_and_restored(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = SettingsFile::new(tmp.path().join("settings.json"));
        file.save(&Settings {
            last_selected_repo: Some("api".into()),
            ..Default::default()
        })
        .unwrap();

        let (w, main) = launch(cx, &file);
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            // main → main worktree → feature/, then collapse it.
            for k in ["down", "down", "left"] {
                window.press(k, cx);
            }
        })
        .unwrap();
        cx.run_until_parked();
        assert!(folder_collapsed(cx, &main, "feature/"));
        assert_eq!(file.load().unwrap().collapsed("api"), ["feature/"]);

        cx.update_window(w, |_, window, cx| {
            window.press("right", cx);
            window.press("left", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(file.load().unwrap().collapsed("api"), ["feature/"]);
        cx.update_window(w, |_, window, cx| window.press("right", cx))
            .unwrap();
        cx.run_until_parked();
        assert!(file.load().unwrap().collapsed("api").is_empty());
        cx.update_window(w, |_, window, cx| window.press("left", cx))
            .unwrap();
        cx.run_until_parked();
        cx.update(|cx| cx.remove_global::<Library>());

        // Fresh load from the same file restores it.
        let (_, main) = launch(cx, &file);
        assert!(folder_collapsed(cx, &main, "feature/"));
        assert!(!folder_collapsed(cx, &main, "fix/"));
    }

    type Launches = Arc<Mutex<Vec<Vec<String>>>>;

    /// Fake launcher with VS Code and Zed CLIs, Terminal, Warp and Ghostty; records each plan instead
    /// of running it (a spawn as its argv, AppleScript as `["osascript", script]`) and
    /// returns `result`.
    fn fake_launcher(cx: &mut TestAppContext, result: Result<Outcome, LaunchError>) -> Launches {
        let launches: Launches = Default::default();
        let sink = launches.clone();
        let editor = |e: Editor| OpenTarget {
            kind: TargetKind::Editor(e),
            app: None,
            cli: Some(format!("/fake/bin/{}", e.cli_name()).into()),
        };
        let terminal = |t: Terminal, app: &str| OpenTarget {
            kind: TargetKind::Terminal(t),
            app: Some(app.into()),
            cli: None,
        };
        cx.update(|cx| {
            cx.set_global(Launcher {
                targets: Some(vec![
                    editor(Editor::VsCode),
                    editor(Editor::Zed),
                    terminal(
                        Terminal::Terminal,
                        "/System/Applications/Utilities/Terminal.app",
                    ),
                    terminal(Terminal::Warp, "/Applications/Warp.app"),
                    terminal(Terminal::Ghostty, "/Applications/Ghostty.app"),
                ]),
                executor: Arc::new(move |plan| {
                    let recorded = match plan {
                        LaunchPlan::Spawn { program, args, .. } => {
                            std::iter::once(program.display().to_string())
                                .chain(args.iter().map(|a| a.to_string_lossy().into_owned()))
                                .collect()
                        }
                        LaunchPlan::AppleScript { script, .. } => vec!["osascript".into(), script],
                    };
                    sink.lock().unwrap().push(recorded);
                    crate::launcher::finished(result.clone())
                }),
                presence: Arc::new(|_| Presence::Present),
                reveal: Arc::new(|path, _| eprintln!("[test] reveal {}", path.display())),
            })
        });
        launches
    }

    /// Makes the fake presence check report `presence` for `path`, Present elsewhere.
    fn fake_presence(cx: &mut TestAppContext, path: &'static str, presence: Presence) {
        cx.update(|cx| {
            cx.global_mut::<Launcher>().presence = Arc::new(move |p| {
                if p == Path::new(path) {
                    presence
                } else {
                    Presence::Present
                }
            })
        });
    }

    fn keys(cx: &mut TestAppContext, w: AnyWindowHandle, keys: &[&str]) {
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            for k in keys {
                window.press(k, cx);
            }
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn has(cx: &mut TestAppContext, w: AnyWindowHandle, id: &'static str, ix: usize) -> bool {
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.try_find((id, ix)).is_some()
        })
        .unwrap()
    }

    fn status(cx: &mut TestAppContext, main: &Entity<MainWindow>) -> Option<Status> {
        main.read_with(cx, |m, _| m.status.clone())
    }

    fn settings_file(tmp: &TempDir, settings: Settings) -> SettingsFile {
        let file = SettingsFile::new(tmp.path().join("settings.json"));
        file.save(&settings).unwrap();
        file
    }

    fn code(path: &str) -> Vec<String> {
        vec!["/fake/bin/code".into(), "--new-window".into(), path.into()]
    }

    #[gpui_kit::test]
    fn enter_opens_worktree_and_branch_rows_in_default_editor(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, main) = launch(cx, &file);

        // Branch row `main` opens its worktree.
        keys(cx, w, &["enter"]);
        assert_eq!(*launches.lock().unwrap(), [code("/c/api")]);
        let s = status(cx, &main).unwrap();
        assert_eq!(s.text, "Opening /c/api in VS Code…");
        assert!(!s.error);

        // main → wt → feature/ → feature/x → its worktree.
        keys(cx, w, &["down", "down", "down", "enter", "down", "enter"]);
        assert_eq!(
            launches.lock().unwrap()[1..],
            [code("/c/wt/api-x"), code("/c/wt/api-x")]
        );
        assert_eq!(
            status(cx, &main).unwrap().text,
            "Opening /c/wt/api-x in VS Code…"
        );

        // A folder row opens nothing; every open is a new launch, never deduped.
        keys(cx, w, &["up", "up", "enter"]);
        assert_eq!(launches.lock().unwrap().len(), 3);
    }

    #[gpui_kit::test]
    fn configured_editor_is_used(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(
            &tmp,
            Settings {
                default_editor: Some("zed".into()),
                ..Default::default()
            },
        );
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, main) = launch(cx, &file);
        keys(cx, w, &["enter"]);
        assert_eq!(
            *launches.lock().unwrap(),
            [vec!["/fake/bin/zed", "--new", "/c/api"]]
        );
        assert_eq!(status(cx, &main).unwrap().text, "Opening /c/api in Zed…");
    }

    #[gpui_kit::test]
    fn launch_failure_shows_reason(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let fail = LaunchError::Exit {
            program: "code".into(),
            status: "exit status: 1".into(),
            stderr: "bad option".into(),
        };
        let launches = fake_launcher(cx, Err(fail));
        let (w, main) = launch(cx, &file);
        keys(cx, w, &["enter"]);
        assert_eq!(launches.lock().unwrap().len(), 1);
        let s = status(cx, &main).unwrap();
        assert_eq!(s.text, "code failed: bad option");
        assert!(s.error);
    }

    #[gpui_kit::test]
    fn uninstalled_default_editor_is_an_error(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(
            &tmp,
            Settings {
                default_editor: Some("cursor".into()),
                ..Default::default()
            },
        );
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, main) = launch(cx, &file);
        keys(cx, w, &["enter"]);
        assert!(launches.lock().unwrap().is_empty());
        assert_eq!(
            status(cx, &main).unwrap(),
            Status {
                text: "Your default editor, Cursor, isn't installed anymore".into(),
                error: true,
                settings: true,
            }
        );
    }

    #[gpui_kit::test]
    fn inline_button_on_selected_worktree_only(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, main) = launch(cx, &file);
        let has_button = |cx: &mut TestAppContext, ix: usize| has(cx, w, "open-editor", ix);
        // Branch row selected: no button anywhere.
        assert!(!(0..10).any(|ix| has_button(cx, ix)));
        keys(cx, w, &["down"]);
        assert!(has_button(cx, 1));
        assert!(!has_button(cx, 4));

        cx.update_window(w, |_, window, cx| window.click(("open-editor", 1usize), cx))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(*launches.lock().unwrap(), [code("/c/api")]);
        assert_eq!(
            status(cx, &main).unwrap().text,
            "Opening /c/api in VS Code…"
        );
    }

    fn row_badges(cx: &mut TestAppContext, main: &Entity<MainWindow>, path: &str) -> Vec<String> {
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

    #[gpui_kit::test]
    fn status_reads_landing_update_badges_and_keep_selection(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let (w, main) = launch(cx, &file);
        keys(cx, w, &["down", "down", "down", "down"]);
        let before = main.read_with(cx, |m, cx| m.tree.read(cx).selected().cloned());
        assert_eq!(before, Some(RowId::Worktree("/c/wt/api-x".into())));
        assert!(row_badges(cx, &main, "/c/wt/api-x").is_empty());

        cx.update(|cx| {
            let store = &mut cx.global_mut::<Library>().store;
            let stamp = store.status_reads().stamp();
            store.apply_status(
                stamp,
                Path::new("/c/wt/api-x"),
                Ok(WorktreeStatus {
                    files: changed_files(3),
                    upstream: Upstream::Tracking {
                        name: "origin/feature/x".into(),
                        ahead: 2,
                        behind: 1,
                    },
                }),
            );
        });
        cx.run_until_parked();
        assert_eq!(row_badges(cx, &main, "/c/wt/api-x"), ["3 changed", "↑2 ↓1"]);
        let after = main.read_with(cx, |m, cx| m.tree.read(cx).selected().cloned());
        assert_eq!(after, before);
    }

    #[gpui_kit::test]
    fn unavailable_worktree_cannot_be_opened(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        fake_presence(cx, "/c/wt/api-x", Presence::Unavailable);
        let mut repo = canned();
        repo.worktrees[1].presence = Presence::Unavailable;
        let (w, main) = launch_repo(cx, &file, repo);
        assert_eq!(row_badges(cx, &main, "/c/wt/api-x"), ["unavailable"]);
        keys(cx, w, &["down", "down", "down", "down", "enter"]);
        assert!(launches.lock().unwrap().is_empty());
        assert_eq!(
            status(cx, &main).unwrap().text,
            "Can't open /c/wt/api-x: its drive isn't mounted"
        );
    }

    #[gpui_kit::test]
    fn missing_worktree_cannot_be_opened(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        fake_presence(cx, "/c/wt/api-x", Presence::Missing);
        let mut repo = canned();
        repo.worktrees[1].presence = Presence::Missing; // /c/wt/api-x
        let (w, main) = launch_repo(cx, &file, repo);

        // feature/x branch, then its worktree; Enter and the button both refuse.
        keys(cx, w, &["down", "down", "down", "enter", "down", "enter"]);
        cx.update_window(w, |_, window, cx| window.click(("open-editor", 4usize), cx))
            .unwrap();
        cx.run_until_parked();
        assert!(launches.lock().unwrap().is_empty());
        let s = status(cx, &main).unwrap();
        assert_eq!(
            s.text,
            "Can't open /c/wt/api-x: folder is missing (run git worktree prune)"
        );
        assert!(s.error);
    }

    #[gpui_kit::test]
    fn folder_deleted_after_scan_is_refused_and_badged(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, main) = launch(cx, &file);
        assert!(row_badges(cx, &main, "/c/wt/api-x").is_empty());
        fake_presence(cx, "/c/wt/api-x", Presence::Missing);

        keys(cx, w, &["down", "down", "down", "down", "enter"]);
        assert!(launches.lock().unwrap().is_empty());
        assert_eq!(
            status(cx, &main).unwrap().text,
            "Can't open /c/wt/api-x: folder is missing (run git worktree prune)"
        );
        assert_eq!(row_badges(cx, &main, "/c/wt/api-x"), ["missing"]);
        keys(cx, w, &["cmd-enter"]);
        assert!(launches.lock().unwrap().is_empty());
    }

    #[gpui_kit::test]
    fn remounted_worktree_opens_and_rereads_status(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let mut repo = canned();
        repo.worktrees[1].presence = Presence::Unavailable; // /c/wt/api-x
        let (w, main) = launch_repo(cx, &file, repo);
        assert_eq!(row_badges(cx, &main, "/c/wt/api-x"), ["unavailable"]);

        keys(cx, w, &["down", "down", "down", "down", "enter"]);
        assert_eq!(*launches.lock().unwrap(), [code("/c/wt/api-x")]);
        let wt = cx.update(|cx| {
            shell::library(cx).store.snapshot().repos[0]
                .worktrees
                .iter()
                .find(|w| w.path == Path::new("/c/wt/api-x"))
                .cloned()
                .unwrap()
        });
        assert_eq!(wt.presence, Presence::Present);
        // The fake path isn't a repo, so the reread lands as a failure, not unread.
        assert!(matches!(wt.status, Some(Err(_))));
    }

    /// The Terminal.app script (window by default) that `cd`s into `path`.
    fn terminal_window(launch: &[String], path: &str) {
        assert_eq!(launch[0], "osascript");
        assert!(
            launch[1].contains(&format!("do script \"cd '{path}'\"")),
            "{}",
            launch[1]
        );
        assert!(
            !launch[1].contains("keystroke"),
            "Terminal.app defaults to windows"
        );
    }

    #[gpui_kit::test]
    fn cmd_enter_opens_worktree_and_branch_rows_in_default_terminal(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, main) = launch(cx, &file);

        // Branch row `main` opens its worktree in a new Terminal tab.
        keys(cx, w, &["cmd-enter"]);
        let got = launches.lock().unwrap().clone();
        assert_eq!(got.len(), 1);
        terminal_window(&got[0], "/c/api");
        let s = status(cx, &main).unwrap();
        assert_eq!(s.text, "Opening /c/api in Terminal…");
        assert!(!s.error);

        // feature/x branch, then its worktree row; each open is a new launch.
        keys(
            cx,
            w,
            &["down", "down", "down", "cmd-enter", "down", "cmd-enter"],
        );
        let got = launches.lock().unwrap().clone();
        assert_eq!(got.len(), 3);
        terminal_window(&got[1], "/c/wt/api-x");
        terminal_window(&got[2], "/c/wt/api-x");
        assert_eq!(
            status(cx, &main).unwrap().text,
            "Opening /c/wt/api-x in Terminal…"
        );

        // Folder row: nothing. Plain Enter still goes to the editor.
        keys(cx, w, &["up", "up", "cmd-enter", "down", "enter"]);
        let got = launches.lock().unwrap().clone();
        assert_eq!(got.len(), 4);
        assert_eq!(got[3], code("/c/wt/api-x"));
    }

    #[gpui_kit::test]
    fn configured_terminal_and_window_setting(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(
            &tmp,
            Settings {
                default_terminal: Some("warp".into()),
                terminal_opens_in: Some(TerminalOpensIn::Window),
                ..Default::default()
            },
        );
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, main) = launch(cx, &file);
        keys(cx, w, &["cmd-enter"]);
        assert_eq!(
            *launches.lock().unwrap(),
            [vec![
                "/usr/bin/open",
                "warp://action/new_window?path=/c/api"
            ]]
        );
        assert_eq!(status(cx, &main).unwrap().text, "Opening /c/api in Warp…");
    }

    #[gpui_kit::test]
    fn automation_denied_shows_permission_hint(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let denied = Outcome::FellBack(Fallback::Denied(Permission::Automation));
        let launches = fake_launcher(cx, Ok(denied));
        let (w, main) = launch(cx, &file);
        keys(cx, w, &["cmd-enter"]);
        assert_eq!(launches.lock().unwrap().len(), 1);
        let s = status(cx, &main).unwrap();
        assert_eq!(
            s.text,
            "Opened /c/api in a new Terminal window. For tabs, allow Grove in System Settings → Privacy & Security → Automation"
        );
        assert!(s.error);
    }

    #[gpui_kit::test]
    fn terminal_launch_failure_shows_reason(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let fail = LaunchError::Exit {
            program: "open".into(),
            status: "exit status: 1".into(),
            stderr: "Unable to find application named 'Terminal'".into(),
        };
        fake_launcher(cx, Err(fail));
        let (w, main) = launch(cx, &file);
        keys(cx, w, &["cmd-enter"]);
        let s = status(cx, &main).unwrap();
        assert_eq!(
            s.text,
            "open failed: Unable to find application named 'Terminal'"
        );
        assert!(s.error);
    }

    #[gpui_kit::test]
    fn uninstalled_default_terminal_is_an_error(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(
            &tmp,
            Settings {
                default_terminal: Some("iterm2".into()),
                ..Default::default()
            },
        );
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, main) = launch(cx, &file);
        keys(cx, w, &["cmd-enter"]);
        assert!(launches.lock().unwrap().is_empty());
        assert_eq!(
            status(cx, &main).unwrap(),
            Status {
                text: "Your default terminal, iTerm2, isn't installed anymore".into(),
                error: true,
                settings: true,
            }
        );
    }

    #[gpui_kit::test]
    fn inline_terminal_button_opens_and_refuses_missing(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        fake_presence(cx, "/c/wt/api-x", Presence::Missing);
        let mut repo = canned();
        repo.worktrees[1].presence = Presence::Missing; // /c/wt/api-x
        let (w, main) = launch_repo(cx, &file, repo);
        let has_button = |cx: &mut TestAppContext, ix: usize| has(cx, w, "open-terminal", ix);
        assert!(!(0..10).any(|ix| has_button(cx, ix)));
        keys(cx, w, &["down"]);
        assert!(has_button(cx, 1));
        assert!(!has_button(cx, 4));

        cx.update_window(w, |_, window, cx| {
            window.click(("open-terminal", 1usize), cx)
        })
        .unwrap();
        cx.run_until_parked();
        let got = launches.lock().unwrap().clone();
        assert_eq!(got.len(), 1);
        terminal_window(&got[0], "/c/api");
        assert_eq!(
            status(cx, &main).unwrap().text,
            "Opening /c/api in Terminal…"
        );

        // Missing worktree: the button is inert and ⌘Enter refuses.
        keys(cx, w, &["down", "down", "down"]);
        assert!(has_button(cx, 4));
        cx.update_window(w, |_, window, cx| {
            window.click(("open-terminal", 4usize), cx)
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(launches.lock().unwrap().len(), 1);
        keys(cx, w, &["cmd-enter"]);
        assert_eq!(launches.lock().unwrap().len(), 1);
        assert_eq!(
            status(cx, &main).unwrap().text,
            "Can't open /c/wt/api-x: folder is missing (run git worktree prune)"
        );
    }

    #[gpui_kit::test]
    fn inline_buttons_fit_at_min_window_size(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let mut repo = canned();
        repo.worktrees[1].status = Some(Ok(WorktreeStatus {
            files: changed_files(12),
            upstream: Upstream::Tracking {
                name: "origin/feature/x".into(),
                ahead: 12,
                behind: 30,
            },
        }));
        let (w, _) = launch_at(cx, &file, repo, MIN_SIZE);
        keys(cx, w, &["down"; 4]);
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            let row = window.find(("tree-row", 4usize)).bounds();
            let details = window.find("details").bounds();
            let editor = window.find(("open-editor", 4usize)).bounds();
            let terminal = window.find(("open-terminal", 4usize)).bounds();
            assert!(editor.left() >= row.left());
            assert!(terminal.right() <= row.right());
            assert!(terminal.right() <= details.left());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn default_app_names_follow_detection_order(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        launch(cx, &file);
        cx.update(|cx| {
            assert_eq!(
                launcher::target_name(grove_core::Role::Terminal, cx).as_deref(),
                Some("Terminal")
            );
            assert_eq!(
                launcher::target_name(grove_core::Role::Editor, cx).as_deref(),
                Some("VS Code")
            );
        });
    }

    /// Status after ⌘Enter on `main` when the executor returns `outcome`.
    fn terminal_status(cx: &mut TestAppContext, settings: Settings, outcome: Outcome) -> Status {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, settings);
        fake_launcher(cx, Ok(outcome));
        let (w, main) = launch(cx, &file);
        keys(cx, w, &["cmd-enter"]);
        status(cx, &main).unwrap()
    }

    #[gpui_kit::test]
    fn permission_timeout_is_reported(cx: &mut TestAppContext) {
        let s = terminal_status(cx, Settings::default(), Outcome::TimedOut);
        assert_eq!(
            s.text,
            "Still waiting for permission to control Terminal. To open it, allow Grove in System Settings → Privacy & Security → Automation"
        );
        assert!(s.error);
    }

    #[gpui_kit::test]
    fn old_ghostty_tab_falls_back_softly(cx: &mut TestAppContext) {
        let settings = Settings {
            default_terminal: Some("ghostty".into()),
            ..Default::default()
        };
        let unsupported = Outcome::FellBack(Fallback::Unsupported(
            "Expected end of line but found identifier. (-2741)".into(),
        ));
        let s = terminal_status(cx, settings, unsupported);
        assert_eq!(
            s.text,
            "Opened /c/api in a new Ghostty window (tabs need Ghostty 1.3 or later)"
        );
        assert!(!s.error);
    }

    #[gpui_kit::test]
    fn cold_launch_fallback_keeps_confirmation(cx: &mut TestAppContext) {
        let settings = Settings {
            default_terminal: Some("ghostty".into()),
            ..Default::default()
        };
        let s = terminal_status(cx, settings, Outcome::FellBack(Fallback::NotRunning));
        assert_eq!(s.text, "Opening /c/api in Ghostty…");
        assert!(!s.error);
    }

    #[gpui_kit::test]
    fn tab_script_failure_names_the_reason(cx: &mut TestAppContext) {
        let failed =
            Outcome::FellBack(Fallback::Failed("Terminal didn't come to the front".into()));
        let s = terminal_status(cx, Settings::default(), failed);
        assert_eq!(
            s.text,
            "Opened /c/api in a new Terminal window (new tab failed: Terminal didn't come to the front)"
        );
        assert!(s.error);
    }

    fn changed_files(n: usize) -> Vec<ChangedFile> {
        (0..n)
            .map(|i| ChangedFile {
                change: Change::Modified,
                path: format!("f{i}.txt"),
                from: None,
            })
            .collect()
    }

    fn click(cx: &mut TestAppContext, w: AnyWindowHandle, id: impl Into<gpui_kit::ElementId>) {
        let id = id.into();
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn found(cx: &mut TestAppContext, w: AnyWindowHandle, id: &'static str) -> bool {
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some()
        })
        .unwrap()
    }

    /// The details pane's title and kind line.
    fn details(cx: &mut TestAppContext, main: &Entity<MainWindow>) -> (String, String) {
        main.read_with(cx, |m, cx| {
            let d = m.details.read(cx).details().cloned().unwrap();
            (d.title, d.kind)
        })
    }

    fn pair(title: &str, kind: &str) -> (String, String) {
        (title.into(), kind.into())
    }

    #[gpui_kit::test]
    fn details_show_name_and_kind_for_branch_and_worktree(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let (w, main) = launch(cx, &file);
        assert!(found(cx, w, "details-kind"));
        assert_eq!(details(cx, &main), pair("main", "Branch · has worktree"));
        keys(cx, w, &["down"]);
        assert_eq!(details(cx, &main), pair("api", "Worktree · main worktree"));
        keys(cx, w, &["down", "down"]);
        assert_eq!(
            details(cx, &main),
            pair("feature/x", "Branch · has worktree")
        );
        keys(cx, w, &["down"]);
        assert_eq!(
            details(cx, &main),
            pair("api-x", "Worktree · linked worktree")
        );
        keys(cx, w, &["down"; 9]);
        assert_eq!(
            details(cx, &main),
            pair("bisect", "Worktree · detached HEAD")
        );
    }

    #[gpui_kit::test]
    fn details_show_upstream_sync_and_changed_files(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let mut repo = canned();
        repo.worktrees[0].status = Some(Ok(WorktreeStatus {
            files: changed_files(2),
            upstream: Upstream::Tracking {
                name: "origin/main".into(),
                ahead: 2,
                behind: 1,
            },
        }));
        let (w, main) = launch_repo(cx, &file, repo);
        let fields = main.read_with(cx, |m, cx| m.details.read(cx).status_fields());
        assert_eq!(
            fields,
            [
                ("Branch", "main".to_string()),
                ("Upstream", "origin/main".into()),
                ("Sync", "2 ahead, 1 behind".into()),
                ("Working tree", "2 changed files".into()),
            ]
        );
        assert!(has(cx, w, "changed-file", 0));
        assert!(has(cx, w, "changed-file", 1));
        assert!(!has(cx, w, "changed-file", 2));
        keys(cx, w, &["down", "down", "down", "down"]);
        assert!(!has(cx, w, "changed-file", 0));
    }

    #[gpui_kit::test]
    fn menu_open_uses_chosen_app_once_without_changing_default(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, main) = launch(cx, &file);

        click(cx, w, "details-menu-editor");
        assert!(found(cx, w, "editor-menu"));
        click(cx, w, ("editor-choice", 1usize));
        assert_eq!(
            *launches.lock().unwrap(),
            [vec!["/fake/bin/zed", "--new", "/c/api"]]
        );
        assert_eq!(status(cx, &main).unwrap().text, "Opening /c/api in Zed…");
        assert!(!found(cx, w, "editor-menu"));

        // The default is untouched: the primary button and Enter still use VS Code.
        click(cx, w, "details-open-editor");
        keys(cx, w, &["enter"]);
        assert_eq!(
            launches.lock().unwrap()[1..],
            [code("/c/api"), code("/c/api")]
        );
        assert_eq!(
            cx.update(|cx| shell::library(cx).settings.default_editor.clone()),
            None
        );
        assert_eq!(file.load().unwrap().default_editor, None);

        click(cx, w, "details-menu-terminal");
        click(cx, w, ("terminal-choice", 1usize));
        assert_eq!(
            launches.lock().unwrap()[3],
            ["/usr/bin/open", "warp://action/new_tab?path=/c/api"]
        );
        click(cx, w, "details-open-terminal");
        terminal_window(&launches.lock().unwrap()[4], "/c/api");
        assert_eq!(file.load().unwrap().default_terminal, None);
    }

    #[gpui_kit::test]
    fn uninstalled_default_falls_back_to_menu(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(
            &tmp,
            Settings {
                default_editor: Some("cursor".into()),
                ..Default::default()
            },
        );
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, main) = launch(cx, &file);
        click(cx, w, "details-open-editor");
        assert!(launches.lock().unwrap().is_empty());
        assert!(status(cx, &main).unwrap().settings);
        assert!(found(cx, w, "editor-menu"));
        click(cx, w, ("editor-choice", 0usize));
        assert_eq!(*launches.lock().unwrap(), [code("/c/api")]);
    }

    #[gpui_kit::test]
    fn copy_cd_and_path_copy_shell_quoted_command_and_raw_path(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let mut repo = canned();
        repo.worktrees[1].path = "/c/wt/it's here".into();
        let (w, main) = launch_repo(cx, &file, repo);
        keys(cx, w, &["down", "down", "down", "down"]);
        let clipboard = |cx: &mut TestAppContext| cx.read_from_clipboard().and_then(|c| c.text());

        click(cx, w, "details-copy-cd");
        assert_eq!(clipboard(cx).as_deref(), Some(r"cd '/c/wt/it'\''s here'"));
        assert_eq!(
            status(cx, &main).unwrap().text,
            "Copied cd command for /c/wt/it's here"
        );

        click(cx, w, "details-copy-path");
        assert_eq!(clipboard(cx).as_deref(), Some("/c/wt/it's here"));
        assert_eq!(
            status(cx, &main).unwrap().text,
            "Copied path /c/wt/it's here"
        );

        click(cx, w, "details-reveal");
        assert_eq!(
            status(cx, &main).unwrap().text,
            "Revealed /c/wt/it's here in Finder"
        );
    }

    /// Selects `/c/wt/api-x` with `presence` and tries every open control in the pane.
    fn try_details_opens(cx: &mut TestAppContext, presence: Presence) -> (Launches, Status) {
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

    #[gpui_kit::test]
    fn details_open_disabled_for_missing_worktree(cx: &mut TestAppContext) {
        let (launches, s) = try_details_opens(cx, Presence::Missing);
        assert!(launches.lock().unwrap().is_empty());
        assert_eq!(
            s.text,
            "Can't reveal /c/wt/api-x: folder is missing (run git worktree prune)"
        );
        assert!(s.error);
    }

    #[gpui_kit::test]
    fn details_open_disabled_for_unavailable_worktree(cx: &mut TestAppContext) {
        let (launches, s) = try_details_opens(cx, Presence::Unavailable);
        assert!(launches.lock().unwrap().is_empty());
        assert_eq!(s.text, "Can't reveal /c/wt/api-x: its drive isn't mounted");
    }

    #[gpui_kit::test]
    fn details_open_refused_when_folder_vanished_after_scan(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, main) = launch(cx, &file);
        keys(cx, w, &["down", "down", "down", "down"]);
        fake_presence(cx, "/c/wt/api-x", Presence::Missing);
        click(cx, w, "details-menu-editor");
        click(cx, w, ("editor-choice", 1usize));
        assert!(launches.lock().unwrap().is_empty());
        assert_eq!(
            status(cx, &main).unwrap().text,
            "Can't open /c/wt/api-x: folder is missing (run git worktree prune)"
        );
        // The refusal rechecked presence, so the pane now shows it disabled.
        click(cx, w, "details-open-editor");
        assert!(launches.lock().unwrap().is_empty());
        assert_eq!(row_badges(cx, &main, "/c/wt/api-x"), ["missing"]);
    }

    fn git(dir: &Path, args: &[&str]) {
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

    #[gpui_kit::test]
    fn details_list_last_three_commits_in_background(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let repo_dir = tmp.path().join("api");
        std::fs::create_dir(&repo_dir).unwrap();
        git(&repo_dir, &["init", "-q"]);
        for m in ["one", "two", "three", "four"] {
            git(&repo_dir, &["commit", "-q", "--allow-empty", "-m", m]);
        }
        let mut repo = canned();
        repo.worktrees[0].path = repo_dir;
        let (w, main) = launch_repo(cx, &file, repo);
        cx.run_until_parked();
        assert!((0..3).all(|i| has(cx, w, "commit", i)));
        assert!(!has(cx, w, "commit", 3));
        // A linked worktree that isn't a repo shows the error instead of stale commits.
        keys(cx, w, &["down", "down", "down", "down"]);
        let err = main.read_with(cx, |m, cx| match m.details.read(cx).commits() {
            Some(Err(e)) => e.to_string(),
            other => panic!("expected an error, got {other:?}"),
        });
        assert!(
            err.starts_with("git log -3") && err.contains("failed:"),
            "{err}"
        );
        assert!(!has(cx, w, "commit", 0));
    }
}
