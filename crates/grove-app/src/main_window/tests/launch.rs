use std::path::PathBuf;

use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt;
use grove_core::{
    Fallback, LaunchError, Outcome, Permission, Presence, Settings, TerminalOpensIn, Upstream,
    WorktreeStatus,
};
use tempfile::TempDir;

use super::super::window::MIN_SIZE;
use super::super::*;
use super::common::*;
use crate::shell::Library;
use crate::test_support::{
    canned, code, fake_launcher, fake_presence, settings_file, tab_to, terminal_window,
};

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
fn custom_commands_open_from_enter_and_the_menu(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = with_commands(&tmp, "custom:Sublime");
    let launches = fake_launcher(cx, Ok(Outcome::Opened));
    let (w, main) = launch(cx, &file);

    keys(cx, w, &["enter"]);
    assert_eq!(
        *launches.lock().unwrap(),
        [vec!["/bin/sh", "-c", "subl -n '/c/api'"]]
    );
    assert_eq!(
        status(cx, &main).unwrap().text,
        "Opening /c/api in Sublime…"
    );

    // Detected editors first, then the commands.
    click(cx, w, "details-menu-editor");
    assert!(has(cx, w, "editor-choice", 3));
    click(cx, w, ("editor-choice", 2usize));
    let launch = launches.lock().unwrap()[1].clone();
    assert_eq!(launch[0], "osascript");
    assert!(
        launch[1].contains(r#"do script "cd '/c/api' && nvim '/c/api'""#),
        "{}",
        launch[1]
    );
    assert_eq!(status(cx, &main).unwrap().text, "Opening /c/api in Neovim…");
}

#[gpui_kit::test]
fn custom_command_failures_show_in_status_line(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = with_commands(&tmp, "custom:Sublime");
    let fail = LaunchError::Exit {
        program: "sh".into(),
        status: "exit status: 127".into(),
        stderr: "sh: subl: command not found".into(),
    };
    fake_launcher(cx, Err(fail));
    let (w, main) = launch(cx, &file);
    keys(cx, w, &["enter"]);
    let s = status(cx, &main).unwrap();
    assert_eq!(s.text, "Sublime failed: sh: subl: command not found");
    assert!(s.error);
}

#[gpui_kit::test]
fn custom_command_in_terminal_reports_fallback_and_warp(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = with_commands(&tmp, "custom:Neovim");
    let denied = Outcome::FellBack(Fallback::Denied(Permission::Automation));
    let launches = fake_launcher(cx, Ok(denied));
    let (w, main) = launch(cx, &file);
    keys(cx, w, &["enter"]);
    assert_eq!(
        status(cx, &main).unwrap().text,
        "Opened /c/api in a new Terminal window without running Neovim. For commands, \
         allow Grove in System Settings → Privacy & Security → Automation"
    );

    cx.update(|cx| cx.global_mut::<Library>().settings.default_terminal = Some("warp".into()));
    keys(cx, w, &["enter"]);
    assert_eq!(launches.lock().unwrap().len(), 1);
    let s = status(cx, &main).unwrap();
    assert!(
        s.text.starts_with("Warp can't run custom commands"),
        "{}",
        s.text
    );
    assert!(s.error && s.settings);
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
        "Can't open /c/wt/api-x: folder is missing (use Remove missing entry…)"
    );
    assert!(s.error);
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
        "Can't open /c/wt/api-x: folder is missing (use Remove missing entry…)"
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
        head: None,
        head_time: None,
        changes_time: None,
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
    let failed = Outcome::FellBack(Fallback::Failed("Terminal didn't come to the front".into()));
    let s = terminal_status(cx, Settings::default(), failed);
    assert_eq!(
        s.text,
        "Opened /c/api in a new Terminal window (new tab failed: Terminal didn't come to the front)"
    );
    assert!(s.error);
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
fn cmd_k_and_jump_to_open_the_switcher(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    let (w, _) = launch(cx, &file);
    assert!(!switcher_open(cx));
    keys(cx, w, &["cmd-k"]);
    assert!(switcher_open(cx));
    let switcher = cx.update(|cx| crate::switcher::window(cx).unwrap());
    keys(cx, switcher, &["escape"]);
    assert!(!switcher_open(cx));

    click(cx, w, "jump-to");
    assert!(switcher_open(cx));
    // Again closes it: one switcher, toggled.
    click(cx, w, "jump-to");
    assert!(!switcher_open(cx));
}

#[gpui_kit::test]
fn switcher_launch_outcomes_reach_the_status_bar(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    let fail = LaunchError::Exit {
        program: "code".into(),
        status: "exit status: 1".into(),
        stderr: "bad option".into(),
    };
    fake_launcher(cx, Err(fail));
    let (w, main) = launch(cx, &file);
    keys(cx, w, &["cmd-k"]);
    let switcher = cx.update(|cx| crate::switcher::window(cx).unwrap());
    keys(cx, switcher, &["enter"]);
    assert_eq!(
        status(cx, &main),
        Some(Status {
            text: "code failed: bad option".into(),
            error: true,
            settings: false,
        })
    );
}

#[gpui_kit::test]
fn tree_and_details_opens_are_recorded_to_recents_and_last_opened(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    fake_presence(cx, "/c/wt/api-y", Presence::Missing);
    let (w, _) = launch(cx, &file);
    let recents = || file.load().unwrap().recents;
    let opened = || {
        let last_opened = file.load().unwrap().last_opened;
        let since = grove_core::unix_now() - 60;
        assert!(last_opened.values().all(|&t| t >= since), "{last_opened:?}");
        last_opened.into_keys().collect::<Vec<_>>()
    };
    keys(cx, w, &["enter"]);
    assert_eq!(recents(), [PathBuf::from("/c/api")]);
    assert_eq!(opened(), [PathBuf::from("/c/api")]);

    keys(cx, w, &["down", "down", "down", "down"]);
    click(cx, w, "details-open-terminal");
    assert_eq!(
        recents(),
        [PathBuf::from("/c/wt/api-x"), PathBuf::from("/c/api")]
    );
    assert_eq!(
        opened(),
        [PathBuf::from("/c/api"), PathBuf::from("/c/wt/api-x")]
    );

    // A refused open isn't one.
    keys(cx, w, &["down", "down", "enter"]);
    assert_eq!(recents().len(), 2);
    assert_eq!(opened().len(), 2);
}

#[gpui_kit::test]
fn open_with_menu_works_from_the_keyboard(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    let launches = fake_launcher(cx, Ok(Outcome::Opened));
    let (w, main) = launch(cx, &file);
    let focused = |cx: &mut TestAppContext, id: gpui_kit::ElementId| {
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.find(id).focused() == Some(true)
        })
        .unwrap()
    };

    tab_to(cx, w, "details-menu-editor");
    keys(cx, w, &["space"]);
    assert!(found(cx, w, "editor-menu"));
    assert!(focused(cx, ("editor-choice", 0usize).into()));
    keys(cx, w, &["escape"]);
    assert!(!found(cx, w, "editor-menu"));
    assert!(focused(cx, "details-menu-editor".into()));

    keys(cx, w, &["enter", "down", "down", "up", "up", "down"]);
    assert!(focused(cx, ("editor-choice", 1usize).into()));
    keys(cx, w, &["enter"]);
    assert_eq!(
        *launches.lock().unwrap(),
        [vec!["/fake/bin/zed", "--new", "/c/api"]]
    );
    assert!(!found(cx, w, "editor-menu"));
    assert!(focused(cx, "details-menu-editor".into()));

    // Tab leaves the menu and closes it.
    keys(cx, w, &["enter", "tab"]);
    assert!(!found(cx, w, "editor-menu"));
    assert!(focused(cx, "details-open-terminal".into()));

    tab_to(cx, w, "details-copy-path");
    keys(cx, w, &["enter"]);
    assert_eq!(status(cx, &main).unwrap().text, "Copied path /c/api");
}

#[gpui_kit::test]
fn empty_fallback_menu_still_closes_on_escape(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    let launches = fake_launcher(cx, Ok(Outcome::Opened));
    cx.update(|cx| {
        let l = cx.global_mut::<crate::launcher::Launcher>();
        l.targets
            .as_mut()
            .unwrap()
            .retain(|t| !matches!(t.kind, grove_core::TargetKind::Editor(_)));
    });
    let (w, _) = launch(cx, &file);
    tab_to(cx, w, "details-open-editor");
    keys(cx, w, &["enter"]);
    assert!(launches.lock().unwrap().is_empty());
    assert!(found(cx, w, "editor-menu"));
    keys(cx, w, &["escape"]);
    assert!(!found(cx, w, "editor-menu"));
    tab_to(cx, w, "tree-view");
}
