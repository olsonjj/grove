use std::path::{Path, PathBuf};

use gpui_kit::test::TestWindowExt;
use gpui_kit::{TestAppContext, px, size};
use grove_core::{Outcome, RowId, Settings, SettingsFile, SortMode};
use tempfile::TempDir;

use super::super::*;
use super::common::sidebar;
use super::common::*;
use crate::shell::Library;
use crate::test_support::{canned, code, fake_launcher, settings_file, tab_to};

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

/// The header row (count, chip, manage button, sort menu) must fit inside the sidebar
/// even with a two-digit hidden count — widening the sidebar came from this review.
#[gpui_kit::test]
fn the_header_and_chip_fit_inside_the_sidebar(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    let names: Vec<String> = (0..12).map(|i| format!("r{i:02}")).collect();
    let repos = names.iter().map(|n| repo_named(n)).collect();
    let (w, _) = launch_repos(cx, &file, repos, size(px(1280.), px(720.)));
    open_visibility(cx, w);
    // Hide ten repos, so the chip reads "· 10 hidden".
    for i in 0..10usize {
        click(cx, w, ("visibility-eye", i));
    }
    assert_eq!(label_of(cx, w, "hidden-chip"), "10 hidden");
    keys(cx, w, &["escape"]);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        let sidebar_right = window.find("repo-list").bounds().right();
        for id in [
            "repos-header",
            "hidden-chip",
            "manage-visibility",
            "repo-sort",
        ] {
            assert!(
                window.find(id).bounds().right() <= sidebar_right,
                "{id} overflows the sidebar"
            );
        }
        // The manage button keeps its breathing room before the sort menu.
        let manage = window.find("manage-visibility").bounds().right();
        let sort_left = window.find("repo-sort").bounds().left();
        assert!(
            manage < sort_left,
            "manage button crowded against the sort menu"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn tab_reaches_every_control_and_comes_back_to_the_tree(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    let w = two_repos(cx, &file);
    let ids: [gpui_kit::ElementId; 15] = [
        "jump-to".into(),
        "fetch-all".into(),
        "open-settings".into(),
        "repo-sort".into(),
        "tree-sort".into(),
        ("repo", 0usize).into(),
        ("repo", 1usize).into(),
        "change-code-folder".into(),
        "details-copy-path".into(),
        "details-copy-cd".into(),
        "details-reveal".into(),
        "details-open-editor".into(),
        "details-menu-editor".into(),
        "details-open-terminal".into(),
        "details-menu-terminal".into(),
    ];
    for id in ids {
        tab_to(cx, w, id);
        tab_to(cx, w, "tree-view");
    }
}

#[gpui_kit::test]
fn keyboard_switches_repo_and_lands_in_its_tree(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    let launches = fake_launcher(cx, Ok(Outcome::Opened));
    let w = two_repos(cx, &file);
    tab_to(cx, w, ("repo", 1usize));
    keys(cx, w, &["enter"]);
    let label = |cx: &mut TestAppContext, ix: usize| {
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.find(("repo", ix)).label().unwrap().to_string()
        })
        .unwrap()
    };
    assert_eq!(label(cx, 1), "web, 1 worktree, selected");
    assert_eq!(label(cx, 0), "api, 5 worktrees");
    // Focus moved into the tree: Enter opens the new repo's worktree.
    keys(cx, w, &["enter"]);
    assert_eq!(*launches.lock().unwrap(), [code("/c/web")]);
    assert_eq!(
        file.load().unwrap().last_selected_repo.as_deref(),
        Some("web")
    );
}

#[gpui_kit::test]
fn tree_rows_and_icon_buttons_have_accessible_labels(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    let (w, _) = launch(cx, &file);
    keys(cx, w, &["down"]);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        let row = window.find(("tree-row", 1usize));
        assert_eq!(
            row.label(),
            Some("/c/api, worktree, The repo's original clone")
        );
        assert_eq!(row.selected(), Some(true));
        assert_eq!(
            window.find(("open-terminal", 1usize)).label(),
            Some("Open in Terminal")
        );
        assert_eq!(
            window.find("open-settings").label(),
            Some("Open-with settings")
        );
        assert_eq!(
            window.find("details-menu-editor").label(),
            Some("Open in another editor")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn pin_button_and_context_menu_pin_persist_and_keep_selection(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    let w = two_repos(cx, &file);
    let pins = || file.load().unwrap().pinned_repos;
    assert!(!has(cx, w, "repo-pin", 1), "only on hover or selection");
    assert_eq!(label_of(cx, w, ("repo-pin", 0usize)), "Pin api");

    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window.hover(("repo", 1usize), cx);
    })
    .unwrap();
    click(cx, w, ("repo-pin", 1usize));
    assert_eq!(pins(), [PathBuf::from("/c/web")]);
    assert_eq!(label_of(cx, w, ("repo", 0usize)), "web, 1 worktree, pinned");
    assert_eq!(label_of(cx, w, ("repo-pin", 0usize)), "Unpin web");
    assert_eq!(
        label_of(cx, w, ("repo", 1usize)),
        "api, 5 worktrees, selected"
    );

    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window.right_click(("repo", 0usize), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(label_of(cx, w, "repo-menu-pin"), "Unpin");
    click(cx, w, "repo-menu-pin");
    assert!(pins().is_empty());
    assert!(!found(cx, w, "repo-menu"));
    assert_eq!(
        label_of(cx, w, ("repo", 0usize)),
        "api, 5 worktrees, selected"
    );

    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        window.right_click(("repo", 0usize), cx);
    })
    .unwrap();
    click(cx, w, "repo-menu-pin");
    assert_eq!(pins(), [PathBuf::from("/c/api")]);
    assert_eq!(
        label_of(cx, w, ("repo", 0usize)),
        "api, 5 worktrees, pinned, selected"
    );
}

#[gpui_kit::test]
fn repo_menu_closes_on_escape_tab_and_outside_click(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    let w = two_repos(cx, &file);
    let open = |cx: &mut TestAppContext| {
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.right_click(("repo", 1usize), cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(found(cx, w, "repo-menu"));
    };
    let focused = |cx: &mut TestAppContext, id: gpui_kit::ElementId| {
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.find(id).focused() == Some(true)
        })
        .unwrap()
    };

    open(cx);
    assert!(focused(cx, "repo-menu-pin".into()));
    keys(cx, w, &["escape"]);
    assert!(!found(cx, w, "repo-menu"));
    assert!(focused(cx, ("repo", 1usize).into()));

    open(cx);
    keys(cx, w, &["tab"]);
    assert!(!found(cx, w, "repo-menu"));

    open(cx);
    click(cx, w, "repos-header");
    assert!(!found(cx, w, "repo-menu"));
    assert!(file.load().unwrap().pinned_repos.is_empty());
}

#[gpui_kit::test]
fn arrow_keys_cross_from_pinned_into_the_rest(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(
        &tmp,
        Settings {
            pinned_repos: vec!["/c/web".into()],
            ..Default::default()
        },
    );
    fake_launcher(cx, Ok(Outcome::Opened));
    let w = two_repos(cx, &file);
    tab_to(cx, w, ("repo", 0usize));
    keys(cx, w, &["down"]);
    assert_eq!(
        label_of(cx, w, ("repo", 1usize)),
        "api, 5 worktrees, selected"
    );
    keys(cx, w, &["up"]);
    assert_eq!(
        label_of(cx, w, ("repo", 0usize)),
        "web, 1 worktree, pinned, selected"
    );
    assert_eq!(
        file.load().unwrap().last_selected_repo.as_deref(),
        Some("web")
    );
    // Focus stayed in the sidebar, on the selected row.
    keys(cx, w, &["down", "down"]);
    assert_eq!(
        label_of(cx, w, ("repo", 1usize)),
        "api, 5 worktrees, selected"
    );
}

#[gpui_kit::test]
fn all_pinned_still_counts_every_repo_and_selects_the_first_pin(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(
        &tmp,
        Settings {
            pinned_repos: vec!["/c/web".into(), "/c/api".into()],
            ..Default::default()
        },
    );
    fake_launcher(cx, Ok(Outcome::Opened));
    let w = two_repos(cx, &file);
    assert_eq!(header_texts(cx, w), ["PINNED · 2", "REPOSITORIES · 2"]);
    assert_eq!(
        label_of(cx, w, ("repo", 0usize)),
        "api, 5 worktrees, pinned, selected"
    );
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        let list: gpui_kit::ElementId = "repo-list".into();
        let in_list = |id: gpui_kit::ElementId| window.find(id).path().contains(&list);
        // REPOSITORIES and PINNED stay fixed above the scrolling list.
        assert!(!in_list("repos-header".into()));
        assert!(!in_list("pinned-header".into()));
        assert!(!in_list(("repo", 0usize).into()));
        assert!(
            window.find("repos-header").bounds().origin.y
                < window.find("pinned-header").bounds().origin.y
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn unpinning_from_the_keyboard_keeps_focus_on_the_moved_button(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(
        &tmp,
        Settings {
            pinned_repos: vec!["/c/web".into()],
            last_selected_repo: Some("api".into()),
            ..Default::default()
        },
    );
    fake_launcher(cx, Ok(Outcome::Opened));
    let w = two_repos(cx, &file);
    assert_eq!(header_texts(cx, w), ["PINNED · 1", "REPOSITORIES · 2"]);
    tab_to(cx, w, ("repo-pin", 0usize));
    keys(cx, w, &["enter"]);
    assert!(file.load().unwrap().pinned_repos.is_empty());
    assert_eq!(header_texts(cx, w), ["REPOSITORIES · 2"]);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        let pin = window.find(("repo-pin", 1usize));
        assert_eq!(pin.label(), Some("Pin web"));
        assert_eq!(pin.focused(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn pinning_keeps_a_saved_selection_of_an_absent_repo(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(
        &tmp,
        Settings {
            last_selected_repo: Some("gone".into()),
            ..Default::default()
        },
    );
    fake_launcher(cx, Ok(Outcome::Opened));
    let w = two_repos(cx, &file);
    click(cx, w, ("repo-pin", 0usize));
    let s = file.load().unwrap();
    assert_eq!(s.pinned_repos, [PathBuf::from("/c/api")]);
    assert_eq!(s.last_selected_repo.as_deref(), Some("gone"));
}

#[gpui_kit::test]
fn opening_a_worktree_lifts_its_repo_in_recent_order(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(
        &tmp,
        Settings {
            last_selected_repo: Some("api".into()),
            ..Default::default()
        },
    );
    fake_launcher(cx, Ok(Outcome::Opened));
    let mut web = repo_named("web");
    web.worktrees[0].head_time = Some(1_700_000_000);
    let (w, _) = launch_repos(
        cx,
        &file,
        vec![repo_named("api"), web],
        size(px(1180.), px(720.)),
    );
    assert_eq!(
        sidebar(cx, w, 2),
        ["web, 1 worktree", "api, 1 worktree, selected"]
    );

    keys(cx, w, &["enter"]);
    assert!(
        file.load()
            .unwrap()
            .last_opened
            .contains_key(Path::new("/c/api"))
    );
    assert_eq!(
        sidebar(cx, w, 2),
        ["api, 1 worktree, selected", "web, 1 worktree"]
    );
}

#[gpui_kit::test]
fn sort_menu_changes_order_persists_and_keeps_selection(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(
        &tmp,
        Settings {
            last_selected_repo: Some("api".into()),
            ..Default::default()
        },
    );
    fake_launcher(cx, Ok(Outcome::Opened));
    let mut web = repo_named("web");
    web.worktrees[0].head_time = Some(1_700_000_000);
    let (w, main) = launch_repos(
        cx,
        &file,
        vec![repo_named("api"), web],
        size(px(1180.), px(720.)),
    );
    let recent = ["web, 1 worktree", "api, 1 worktree, selected"];
    let by_name = ["api, 1 worktree, selected", "web, 1 worktree"];
    assert_eq!(sidebar(cx, w, 2), recent);
    assert_eq!(label_of(cx, w, "repo-sort"), "Sort repositories: Recent");

    click(cx, w, "repo-sort");
    assert!(found(cx, w, "repo-sort-menu"));
    click(cx, w, ("repo-sort-choice", 1usize));
    assert!(!found(cx, w, "repo-sort-menu"));
    assert_eq!(sidebar(cx, w, 2), by_name);
    assert_eq!(label_of(cx, w, "repo-sort"), "Sort repositories: Name");
    assert_eq!(file.load().unwrap().repo_sort, SortMode::Name);
    assert_eq!(
        main.read_with(cx, |m, cx| m.tree.read(cx).repo_name().map(str::to_string)),
        Some("api".into())
    );

    // From the keyboard: opens on the current mode; Escape closes back to the button.
    let focused = |cx: &mut TestAppContext, id: gpui_kit::ElementId| {
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.find(id).focused() == Some(true)
        })
        .unwrap()
    };
    tab_to(cx, w, "repo-sort");
    keys(cx, w, &["enter"]);
    assert!(focused(cx, ("repo-sort-choice", 1usize).into()));
    keys(cx, w, &["escape"]);
    assert!(!found(cx, w, "repo-sort-menu"));
    assert!(focused(cx, "repo-sort".into()));
    keys(cx, w, &["enter", "up", "enter"]);
    assert!(!found(cx, w, "repo-sort-menu"));
    assert_eq!(sidebar(cx, w, 2), recent);
    assert_eq!(file.load().unwrap().repo_sort, SortMode::Recent);

    // Tab leaves the menu and closes it.
    keys(cx, w, &["enter", "tab"]);
    assert!(!found(cx, w, "repo-sort-menu"));
}

#[gpui_kit::test]
fn live_reorder_keeps_selection_and_scrolls_it_into_view(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    let names: Vec<String> = (0..30).map(|i| format!("r{i:02}")).collect();
    let repos = names.iter().map(|n| repo_named(n)).collect();
    let (w, main) = launch_repos(cx, &file, repos, size(px(1180.), px(400.)));
    let in_view = |cx: &mut TestAppContext, ix: usize| {
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            let list = window.find("repo-list").bounds();
            let row = window.find(("repo", ix)).bounds();
            row.top() >= list.top() && row.bottom() <= list.bottom()
        })
        .unwrap()
    };
    // Nothing saved: the first row is selected.
    assert_eq!(
        label_of(cx, w, ("repo", 0usize)),
        "r00, 1 worktree, selected"
    );
    assert!(in_view(cx, 0));
    assert!(!in_view(cx, 29), "fixture must overflow");

    for (i, name) in names.iter().enumerate().skip(1) {
        commit_lands(cx, name, 1_700_000_000 + i as i64);
    }
    assert_eq!(label_of(cx, w, ("repo", 0usize)), "r29, 1 worktree");
    assert_eq!(
        label_of(cx, w, ("repo", 29usize)),
        "r00, 1 worktree, selected"
    );
    assert!(in_view(cx, 29));
    assert_eq!(
        main.read_with(cx, |m, cx| m.tree.read(cx).repo_name().map(str::to_string)),
        Some("r00".into())
    );
}

#[gpui_kit::test]
fn tree_sort_menu_reorders_persists_and_keeps_selection(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    // Saved from a previous run.
    let file = settings_file(
        &tmp,
        Settings {
            tree_sort: SortMode::Name,
            ..Default::default()
        },
    );
    let launches = fake_launcher(cx, Ok(Outcome::Opened));
    let mut repo = canned();
    repo.worktrees[2].head_time = Some(1_600_000_000); // feature/y
    repo.worktrees[3].head_time = Some(1_700_000_000); // fix/z
    let (w, main) = launch_repo(cx, &file, repo);
    let by_name = ["main", "feature/x", "feature/y", "fix/z"];
    let recent = ["main", "fix/z", "feature/y", "feature/x"];
    assert_eq!(tree_branches(cx, &main), by_name);
    assert_eq!(label_of(cx, w, "tree-sort"), "Sort branches: Name");

    // main, its worktree, feature/, feature/x, its worktree, feature/y.
    keys(cx, w, &["down"; 5]);
    let selected =
        |cx: &mut TestAppContext| main.read_with(cx, |m, cx| m.tree.read(cx).selected().cloned());
    let y = Some(RowId::Branch("feature/y".into()));
    assert_eq!(selected(cx), y);

    click(cx, w, "tree-sort");
    assert!(found(cx, w, "tree-sort-menu"));
    click(cx, w, ("tree-sort-choice", 0usize));
    assert!(!found(cx, w, "tree-sort-menu"));
    assert_eq!(tree_branches(cx, &main), recent);
    assert_eq!(selected(cx), y);
    assert_eq!(label_of(cx, w, "tree-sort"), "Sort branches: Recent");
    assert_eq!(file.load().unwrap().tree_sort, SortMode::Recent);
    // The repo sort is separate.
    assert_eq!(file.load().unwrap().repo_sort, SortMode::Recent);

    // From the keyboard; keys meant for the menu never reach the tree.
    tab_to(cx, w, "tree-sort");
    keys(cx, w, &["down"]);
    assert_eq!(selected(cx), y);
    keys(cx, w, &["up"]);
    assert_eq!(selected(cx), y);
    keys(cx, w, &["enter", "down", "enter"]);
    assert!(!found(cx, w, "tree-sort-menu"));
    assert_eq!(tree_branches(cx, &main), by_name);
    assert_eq!(selected(cx), y);
    assert_eq!(file.load().unwrap().tree_sort, SortMode::Name);
    keys(cx, w, &["enter", "escape"]);
    assert!(!found(cx, w, "tree-sort-menu"));
    assert_eq!(selected(cx), y);
    assert!(launches.lock().unwrap().is_empty());
}

#[gpui_kit::test]
fn recent_tree_reorders_live_and_keeps_selection(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    let (w, main) = launch(cx, &file);
    assert_eq!(
        tree_branches(cx, &main),
        ["main", "feature/x", "feature/y", "fix/z"]
    );
    // main, its worktree, feature/, feature/x.
    keys(cx, w, &["down"; 3]);
    // Opening fix/z's worktree from elsewhere makes it the newest.
    cx.update(|cx| shell::record_open(Path::new("/c/wt/api-z"), cx));
    cx.run_until_parked();
    assert_eq!(
        tree_branches(cx, &main),
        ["main", "fix/z", "feature/x", "feature/y"]
    );
    assert_eq!(
        main.read_with(cx, |m, cx| m.tree.read(cx).selected().cloned()),
        Some(RowId::Branch("feature/x".into()))
    );
}
