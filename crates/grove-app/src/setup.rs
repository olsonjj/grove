//! Launch flow: git check → saved code folder → main window, or the setup window. The same
//! folder picker and preview change the code folder later (settings, sidebar).

use std::path::PathBuf;

use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyWindowHandle, App, AppContext, Bounds, ClickEvent, Context, Entity, FocusHandle, FontWeight,
    Global, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, PathPromptOptions,
    Render, StatefulInteractiveElement, Styled, TitlebarOptions, WeakEntity, Window, WindowBounds,
    WindowKind, WindowOptions, div, px, size,
};
use grove_core::git::{self, GitError};
use grove_core::{
    Settings, SettingsFile, Snapshot, SnapshotCache, SnapshotStore, resolved_env, scan,
};

use crate::main_window::tilde;
use crate::shell::{self, Library};
use crate::theme::{self, ButtonKind, button};

enum Boot {
    /// Whether the snapshot came from the cache.
    Ready(Library, bool),
    Setup(Setup),
}

/// Runs the blocking launch checks off the main thread, then starts the shell or opens setup.
pub fn launch(cx: &mut App) {
    cx.spawn(async move |cx| {
        let boot = cx.background_spawn(async { boot() }).await;
        cx.update(|cx| match boot {
            Boot::Ready(library, cached) => shell::start(library, cached, cx),
            Boot::Setup(setup) => open(setup, cx),
        });
    })
    .detach();
}

fn boot() -> Boot {
    let env = resolved_env();
    eprintln!(
        "[grove] PATH ({:?} via {}): {}",
        env.source, env.shell, env.path
    );
    let settings_file = SettingsFile::default_location();
    let settings = settings_file.load().unwrap_or_else(|e| {
        eprintln!("[grove] settings {}: {e}", settings_file.path().display());
        Settings::default()
    });
    let mut setup = Setup {
        stage: Stage::Choose,
        mode: Mode::FirstRun {
            settings_file,
            settings,
        },
        folder: None,
        scanned: None,
        focus: None,
    };
    if let Err(e) = check_git() {
        setup.stage = Stage::GitProblem(e);
        return Boot::Setup(setup);
    }

    // Dev/testing: behave as if this folder was just picked in setup.
    if let Some(dir) = std::env::var_os("GROVE_CODE_DIR") {
        let dir = PathBuf::from(dir);
        setup.scanned = Some(scan(&dir));
        setup.folder = Some(dir);
        return Boot::Setup(setup);
    }

    let Mode::FirstRun {
        settings_file,
        settings,
    } = &setup.mode
    else {
        unreachable!("boot is first run");
    };
    let (settings_file, settings) = (settings_file.clone(), settings.clone());
    let Some(dir) = settings.code_dir.clone() else {
        return Boot::Setup(setup);
    };
    // A folder that's gone falls through to the scan, which sends it back to setup.
    let cached = SnapshotCache::beside(&settings_file)
        .load(&dir)
        .filter(|s| !s.repos.is_empty() && dir.is_dir());
    if let Some(snapshot) = cached {
        eprintln!(
            "[grove] code dir {}: {} repos (cached)",
            dir.display(),
            snapshot.repos.len()
        );
        return Boot::Ready(
            Library {
                settings_file,
                settings,
                store: SnapshotStore::from_snapshot(snapshot),
                saves: Default::default(),
            },
            true,
        );
    }
    let snapshot = scan(&dir);
    eprintln!(
        "[grove] code dir {} (exists: {}): {} repos",
        dir.display(),
        snapshot.code_dir_exists,
        snapshot.repos.len()
    );
    if snapshot.repos.is_empty() {
        setup.folder = Some(dir);
        setup.scanned = Some(snapshot);
        return Boot::Setup(setup);
    }
    Boot::Ready(
        Library {
            settings_file,
            settings,
            store: SnapshotStore::from_snapshot(snapshot),
            saves: Default::default(),
        },
        false,
    )
}

fn check_git() -> Result<(), GitError> {
    let result = git::check();
    match &result {
        Ok(info) => eprintln!("[grove] git {} at {}", info.version, info.binary.display()),
        Err(e) => eprintln!("[grove] git check failed: {e}"),
    }
    result.map(|_| ())
}

enum Stage {
    GitProblem(GitError),
    CheckingGit,
    Choose,
}

/// Change…'s Continue, given the chosen folder's scan.
type ChangeDone = Box<dyn FnOnce(Snapshot, &mut App) + Send>;

/// What Continue does with the chosen folder.
#[expect(clippy::large_enum_variant, reason = "one per window")]
enum Mode {
    /// Saves it and starts Grove.
    FirstRun {
        settings_file: SettingsFile,
        settings: Settings,
    },
    /// Hands its scan to `done` (settings, sidebar) and closes. `None` once used.
    Change(Option<ChangeDone>),
}

pub struct Setup {
    stage: Stage,
    mode: Mode,
    folder: Option<PathBuf>,
    /// Scan of `folder`; `None` while scanning.
    scanned: Option<Snapshot>,
    /// Set when the window opens, so keys land here before anything is tabbed to.
    focus: Option<FocusHandle>,
}

impl Setup {
    fn view(mut self, window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            self.focus = Some(focus);
            self
        })
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, _: &mut Context<Self>) {
        if ev.keystroke.key == "escape" && matches!(self.mode, Mode::Change(_)) {
            window.remove_window();
        }
    }

    fn choose_folder(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.prompt(cx);
    }

    fn prompt(&mut self, cx: &mut Context<Self>) {
        cx.activate(true);
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(dir) = paths.into_iter().next()
            {
                let _ = this.update(cx, |this, cx| this.scan(dir, cx));
            }
        })
        .detach();
    }

    fn scan(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        self.folder = Some(dir.clone());
        self.scanned = None;
        cx.notify();
        let task = cx.background_spawn(async move { scan(&dir) });
        cx.spawn(async move |this, cx| {
            let snapshot = task.await;
            let _ = this.update(cx, |this, cx| {
                // Ignore a stale scan if the user picked another folder meanwhile.
                if this.folder.as_ref() == Some(&snapshot.code_dir) {
                    this.scanned = Some(snapshot);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn recheck_git(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.stage = Stage::CheckingGit;
        cx.notify();
        let task = cx.background_spawn(async { check_git() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.stage = match result {
                    Ok(()) => Stage::Choose,
                    Err(e) => Stage::GitProblem(e),
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn finish(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(snapshot) = self.scanned.take().filter(|s| !s.repos.is_empty()) else {
            return;
        };
        eprintln!(
            "[grove] code folder chosen: {} ({} repos)",
            snapshot.code_dir.display(),
            snapshot.repos.len()
        );
        match &mut self.mode {
            Mode::FirstRun {
                settings_file,
                settings,
            } => {
                settings.code_dir = Some(snapshot.code_dir.clone());
                if let Err(e) = settings_file.save(settings) {
                    eprintln!(
                        "[grove] saving settings to {}: {e}",
                        settings_file.path().display()
                    );
                }
                let library = Library {
                    settings_file: settings_file.clone(),
                    settings: settings.clone(),
                    store: SnapshotStore::from_snapshot(snapshot),
                    saves: Default::default(),
                };
                shell::start(library, false, cx);
            }
            Mode::Change(done) => {
                if let Some(done) = done.take() {
                    done(snapshot, cx);
                }
            }
        }
        window.remove_window();
    }

    fn render_choose(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let folder = self.folder.as_deref().map(tilde);
        let has_repos = self.scanned.as_ref().is_some_and(|s| !s.repos.is_empty());
        let scanning = self.folder.is_some() && self.scanned.is_none();

        let field = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(14.))
            .py(px(10.))
            .rounded(px(10.))
            .bg(theme::field())
            .border_1()
            .border_color(if has_repos {
                gpui_kit::rgb(0x2c4560)
            } else {
                theme::border_strong()
            })
            .child(
                theme::path_text(folder.clone().unwrap_or_else(|| "No folder chosen".into()))
                    .flex_1()
                    .text_size(px(13.))
                    .text_color(if folder.is_some() {
                        theme::text()
                    } else {
                        theme::faint()
                    }),
            )
            .child(button(
                "choose",
                if has_repos {
                    "Change…"
                } else {
                    "Choose folder…"
                },
                ButtonKind::Secondary,
                Some(cx.listener(Self::choose_folder)),
            ));

        let result = match (&self.scanned, &folder) {
            (_, None) => None,
            (None, Some(_)) => Some(
                div()
                    .text_size(px(13.))
                    .text_color(theme::muted())
                    .child("Looking for Git repos…")
                    .into_any_element(),
            ),
            (Some(s), Some(name)) if s.repos.is_empty() => Some(
                div()
                    .px(px(14.))
                    .py(px(12.))
                    .rounded(px(8.))
                    .bg(theme::warn_bg())
                    .text_color(theme::warn_text())
                    .text_size(px(13.))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .flex()
                            .gap(px(4.))
                            .child(div().flex_none().child("No Git repos found in"))
                            .child(theme::path_text(name.clone()).flex_1()),
                    )
                    .child(if s.code_dir_exists {
                        "Choose the folder that contains your repos."
                    } else {
                        "That folder no longer exists. Choose another."
                    })
                    .into_any_element(),
            ),
            (Some(s), Some(name)) => Some(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex()
                            .gap(px(4.))
                            .text_size(px(13.))
                            .text_color(theme::text_soft())
                            .child(div().flex_none().child(format!(
                                "Found {} repo{} in",
                                s.repos.len(),
                                if s.repos.len() == 1 { "" } else { "s" }
                            )))
                            .child(theme::path_text(name.clone()).flex_1()),
                    )
                    .child(
                        div()
                            .id("preview")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .child(div().grid().grid_cols(2).gap(px(8.)).children(
                                s.repos.iter().map(|r| {
                                    let n = r.worktree_count();
                                    div()
                                        .h(px(38.))
                                        .px(px(12.))
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .gap(px(8.))
                                        .rounded(px(8.))
                                        .bg(theme::card())
                                        .child(
                                            theme::mono(r.display_name.clone())
                                                .min_w_0()
                                                .truncate()
                                                .text_size(px(13.)),
                                        )
                                        .child(
                                            div()
                                                .flex_none()
                                                .text_size(px(12.))
                                                .text_color(theme::link())
                                                .child(format!(
                                                    "{n} worktree{}",
                                                    if n == 1 { "" } else { "s" }
                                                )),
                                        )
                                }),
                            )),
                    )
                    .into_any_element(),
            ),
        };

        let changing = matches!(self.mode, Mode::Change(_));
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(20.))
            .p(px(32.))
            .child(header(if changing {
                "Code folder"
            } else {
                "Setup · 1 step"
            }))
            .child(title("Where do you keep your code?"))
            .child(paragraph(if changing {
                "Pick the one folder that holds your Git repos. Grove reads only this folder — \
                 repos outside it disappear from Grove."
            } else {
                "Pick the one folder that holds your Git repos. Grove reads only this folder — \
                 nothing else on your Mac is scanned. You can change it later in settings."
            }))
            .child(field)
            .child(div().flex_1().min_h_0().flex().flex_col().children(result))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(theme::faint())
                            .child(if scanning {
                                "Scanning…"
                            } else {
                                "Grove never changes your repos."
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .when(changing, |d| {
                                d.child(button(
                                    "cancel",
                                    "Cancel",
                                    ButtonKind::Secondary,
                                    Some(|_: &ClickEvent, window: &mut Window, _: &mut App| {
                                        window.remove_window()
                                    }),
                                ))
                            })
                            .child(button(
                                "continue",
                                "Continue",
                                ButtonKind::Primary,
                                has_repos.then(|| cx.listener(Self::finish)),
                            )),
                    ),
            )
    }

    fn render_git_problem(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let (headline, detail) = match &self.stage {
            Stage::GitProblem(GitError::TooOld { found }) => (
                "Grove needs a newer Git".to_string(),
                format!(
                    "Grove found git {found}, but needs {} or later for worktree support.",
                    git::MIN_VERSION
                ),
            ),
            Stage::GitProblem(e) => (
                "Grove needs Git".to_string(),
                format!("Grove couldn't run git ({e})."),
            ),
            _ => ("Checking for Git…".to_string(), String::new()),
        };
        let checking = matches!(self.stage, Stage::CheckingGit);
        let command = |cmd: &'static str, note: &'static str| {
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(
                    theme::mono(cmd)
                        .px(px(12.))
                        .py(px(8.))
                        .rounded(px(6.))
                        .bg(theme::field())
                        .border_1()
                        .border_color(theme::border())
                        .text_size(px(13.)),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme::faint())
                        .child(note),
                )
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(18.))
            .p(px(32.))
            .child(header("Setup"))
            .child(title(headline))
            .child(paragraph(detail).text_color(theme::warn_text()))
            .child(paragraph(format!(
                "Install Git {} or later with one of these in Terminal, then check again:",
                git::MIN_VERSION
            )))
            .child(command(
                "xcode-select --install",
                "Apple's Command Line Tools (includes git).",
            ))
            .child(command("brew install git", "Homebrew's git."))
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(button(
                        "quit",
                        "Quit",
                        ButtonKind::Secondary,
                        Some(|_: &ClickEvent, _: &mut Window, cx: &mut App| shell::quit(cx)),
                    ))
                    .child(button(
                        "recheck",
                        if checking {
                            "Checking…"
                        } else {
                            "Check again"
                        },
                        ButtonKind::Primary,
                        (!checking).then(|| cx.listener(Self::recheck_git)),
                    )),
            )
    }
}

impl Render for Setup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.stage {
            Stage::Choose => self.render_choose(cx).into_any_element(),
            _ => self.render_git_problem(cx).into_any_element(),
        };
        div()
            .id("setup")
            .when_some(self.focus.as_ref(), |d, f| d.track_focus(f))
            .on_key_down(cx.listener(Self::on_key_down))
            .size_full()
            .bg(theme::panel())
            .text_color(theme::text())
            .child(body)
    }
}

fn header(step: &'static str) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .child(div().size(px(10.)).rounded(px(5.)).bg(theme::accent()))
        .child(
            div()
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .child("Grove"),
        )
        .child(
            div()
                .ml_auto()
                .text_size(px(12.))
                .text_color(theme::faint())
                .child(step),
        )
}

fn title(text: impl Into<gpui_kit::SharedString>) -> impl IntoElement {
    div()
        .text_size(px(24.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(text.into())
}

fn paragraph(text: impl Into<gpui_kit::SharedString>) -> gpui_kit::Div {
    div()
        .text_size(px(14.))
        .line_height(px(21.))
        .text_color(theme::muted())
        .child(text.into())
}

/// The open Change… window, so a second Change… brings it forward instead.
struct Changing(AnyWindowHandle, WeakEntity<Setup>);

impl Global for Changing {}

/// Change…: the first-run picker and preview for another code folder. `done` gets the
/// chosen folder's scan on Continue; Cancel or closing the window drops it. If the picker
/// is already open, the latest Change…'s `done` replaces the earlier one.
pub fn change_code_dir(done: impl FnOnce(Snapshot, &mut App) + Send + 'static, cx: &mut App) {
    let done: ChangeDone = Box::new(done);
    let open = cx
        .try_global::<Changing>()
        .filter(|c| cx.windows().contains(&c.0))
        .and_then(|c| Some((c.0, c.1.upgrade()?)));
    if let Some((w, setup)) = open {
        setup.update(cx, |setup, _| setup.mode = Mode::Change(Some(done)));
        cx.activate(true);
        let _ = w.update(cx, |_, window, _| window.activate_window());
        return;
    }
    let setup = Setup {
        stage: Stage::Choose,
        mode: Mode::Change(Some(done)),
        folder: None,
        scanned: None,
        focus: None,
    };
    let options = window_options("Choose Code Folder", cx);
    cx.activate(true);
    match gpui_kit::open_window(options, cx, |window, cx| {
        let setup = setup.view(window, cx);
        setup.update(cx, |setup, cx| setup.prompt(cx));
        setup
    }) {
        Ok((handle, setup)) => cx.set_global(Changing(handle, setup.downgrade())),
        Err(e) => eprintln!("[grove] code folder window: {e:#}"),
    }
}

/// The Change… window, if open.
#[cfg(test)]
pub fn changing(cx: &App) -> Option<AnyWindowHandle> {
    cx.try_global::<Changing>()
        .map(|c| c.0)
        .filter(|w| cx.windows().contains(w))
}

fn window_options(title: &'static str, cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(640.), px(600.)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some(title.into()),
            ..Default::default()
        }),
        focus: true,
        show: true,
        kind: WindowKind::Normal,
        is_resizable: false,
        ..Default::default()
    }
}

fn open(setup: Setup, cx: &mut App) {
    let options = window_options("Grove Setup", cx);
    cx.activate(true);
    if let Err(e) = gpui_kit::open_window(options, cx, |window, cx| setup.view(window, cx)) {
        eprintln!("[grove] setup window: {e:#}");
        shell::quit(cx);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use gpui_kit::TestAppContext;
    use gpui_kit::test::TestWindowExt;
    use grove_core::Snapshot;

    use super::*;
    use crate::test_support::{canned, tab_to};

    fn change(cx: &mut TestAppContext, done: ChangeDone) -> AnyWindowHandle {
        cx.update(|cx| {
            gpui_kit::init(cx);
            let setup = Setup {
                stage: Stage::Choose,
                mode: Mode::Change(Some(done)),
                folder: Some("/c".into()),
                scanned: Some(Snapshot {
                    code_dir: "/c".into(),
                    code_dir_exists: true,
                    repos: vec![canned()],
                    last_fetch: None,
                }),
                focus: None,
            };
            gpui_kit::open_window(
                window_options("Choose Code Folder", cx),
                cx,
                |window, cx| setup.view(window, cx),
            )
            .unwrap()
            .0
        })
    }

    fn first_run(cx: &mut TestAppContext, stage: Stage) -> AnyWindowHandle {
        let tmp = tempfile::TempDir::new().unwrap();
        let settings_file = SettingsFile::new(tmp.path().join("settings.json"));
        cx.update(|cx| {
            gpui_kit::init(cx);
            let setup = Setup {
                stage,
                mode: Mode::FirstRun {
                    settings_file,
                    settings: Settings::default(),
                },
                folder: Some("/c".into()),
                scanned: Some(Snapshot {
                    code_dir: "/c".into(),
                    code_dir_exists: true,
                    repos: vec![canned()],
                    last_fetch: None,
                }),
                focus: None,
            };
            gpui_kit::open_window(window_options("Grove Setup", cx), cx, |window, cx| {
                setup.view(window, cx)
            })
            .unwrap()
            .0
        })
    }

    #[gpui_kit::test]
    fn first_run_buttons_are_reachable_by_tab(cx: &mut TestAppContext) {
        let w = first_run(cx, Stage::Choose);
        tab_to(cx, w, "choose");
        tab_to(cx, w, "continue");
        let w = first_run(
            cx,
            Stage::GitProblem(GitError::NotFound {
                detail: "no git".into(),
            }),
        );
        tab_to(cx, w, "quit");
        tab_to(cx, w, "recheck");
    }

    #[gpui_kit::test]
    fn change_folder_works_from_the_keyboard(cx: &mut TestAppContext) {
        let chosen = Arc::new(Mutex::new(None));
        let sink = chosen.clone();
        let w = change(
            cx,
            Box::new(move |s, _| *sink.lock().unwrap() = Some(s.code_dir)),
        );
        tab_to(cx, w, "choose");
        tab_to(cx, w, "cancel");
        tab_to(cx, w, "continue");
        cx.update_window(w, |_, window, cx| window.press("enter", cx))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(*chosen.lock().unwrap(), Some("/c".into()));
        assert!(!cx.update(|cx| cx.windows().contains(&w)));
    }

    #[gpui_kit::test]
    fn escape_cancels_change_folder(cx: &mut TestAppContext) {
        let w = change(cx, Box::new(|_, _| panic!("cancelled")));
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.press("escape", cx)
        })
        .unwrap();
        cx.run_until_parked();
        assert!(!cx.update(|cx| cx.windows().contains(&w)));
    }
}
