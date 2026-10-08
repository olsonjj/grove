//! Canned snapshots and fakes shared by the windows' GPUI tests.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, App, AppContext, ElementId, TestAppContext};
use grove_core::{
    Editor, LaunchError, LaunchPlan, OpenTarget, Outcome, Presence, RepoRef, RepoSnapshot,
    Settings, SettingsFile, Snapshot, SnapshotStore, TargetKind, Terminal, Worktree,
};
use tempfile::TempDir;

use crate::launcher::Launcher;
use crate::shell::Library;

pub fn wt(path: &str, branch: Option<&str>, is_main: bool) -> Worktree {
    Worktree {
        path: PathBuf::from(path),
        branch: branch.map(str::to_string),
        head: Some("a1f3c9e5d2".into()),
        detached: branch.is_none(),
        is_main,
        locked: false,
        presence: Presence::Present,
        status: None,
    }
}

/// main, feature/{x,y}, fix/z, one detached.
pub fn canned() -> RepoSnapshot {
    RepoSnapshot {
        display_name: "api".into(),
        repo: RepoRef {
            name: "api".into(),
            path: "/c/api".into(),
            git_dir: "/c/api/.git".into(),
            bare: false,
        },
        worktrees: vec![
            wt("/c/api", Some("main"), true),
            wt("/c/wt/api-x", Some("feature/x"), false),
            wt("/c/wt/api-y", Some("feature/y"), false),
            wt("/c/wt/api-z", Some("fix/z"), false),
            wt("/c/wt/bisect", None, false),
        ],
        error: None,
        fetch_error: None,
    }
}

/// Sets the `Library` global: `file`'s settings over `repos` in code dir `/c`.
pub fn set_library(file: &SettingsFile, repos: Vec<RepoSnapshot>, cx: &mut App) {
    cx.set_global(Library {
        settings_file: file.clone(),
        settings: file.load().unwrap(),
        store: SnapshotStore::from_snapshot(Snapshot {
            code_dir: PathBuf::from("/c"),
            code_dir_exists: true,
            repos,
            last_fetch: None,
        }),
        saves: Default::default(),
    });
}

pub fn settings_file(tmp: &TempDir, settings: Settings) -> SettingsFile {
    let file = SettingsFile::new(tmp.path().join("settings.json"));
    file.save(&settings).unwrap();
    file
}

pub type Launches = Arc<Mutex<Vec<Vec<String>>>>;

/// Fake launcher with VS Code and Zed CLIs, Terminal, Warp and Ghostty; records each plan instead
/// of running it (a spawn as its argv, AppleScript as `["osascript", script]`) and
/// returns `result`.
pub fn fake_launcher(cx: &mut TestAppContext, result: Result<Outcome, LaunchError>) -> Launches {
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
pub fn fake_presence(cx: &mut TestAppContext, path: &'static str, presence: Presence) {
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

/// VS Code's argv opening `path`.
pub fn code(path: &str) -> Vec<String> {
    vec!["/fake/bin/code".into(), "--new-window".into(), path.into()]
}

/// The Terminal.app script (window by default) that `cd`s into `path`.
pub fn terminal_window(launch: &[String], path: &str) {
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

/// Presses Tab until `id` has keyboard focus; panics if it never gets it.
pub fn tab_to(cx: &mut TestAppContext, w: AnyWindowHandle, id: impl Into<ElementId>) {
    let id = id.into();
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        for _ in 0..60 {
            if window.try_find(id.clone()).and_then(|e| e.focused()) == Some(true) {
                return;
            }
            window.press("tab", cx);
        }
        panic!("Tab never reached {id:?}");
    })
    .unwrap();
    cx.run_until_parked();
}
