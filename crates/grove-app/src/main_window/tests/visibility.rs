use std::path::PathBuf;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{TestAppContext, px, size};
use grove_core::{Outcome, Settings};
use tempfile::TempDir;

use super::super::*;
use super::common::*;
use crate::test_support::{canned, fake_launcher, settings_file, tab_to};

#[gpui_kit::test]
fn manage_popover_lists_every_repo_and_closes_on_escape_outside_and_button(
    cx: &mut TestAppContext,
) {
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
    assert_eq!(header_texts(cx, w), ["PINNED · 1", "REPOSITORIES · 2"]);
    assert!(!found(cx, w, "hidden-chip"), "nothing hidden yet");
    assert_eq!(
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            Some(window.find("manage-visibility").label()?.to_string())
        })
        .unwrap(),
        Some("Manage repository visibility".into())
    );

    open_visibility(cx, w);
    assert_eq!(visibility_rows(cx, w), ["web, shown", "api, shown"]);

    // The filter field starts focused, so Escape goes to the popover.
    keys(cx, w, &["escape"]);
    assert!(!found(cx, w, "visibility-menu"));

    open_visibility(cx, w);
    click(cx, w, "repos-header");
    assert!(!found(cx, w, "visibility-menu"));

    open_visibility(cx, w);
    click(cx, w, "manage-visibility");
    assert!(!found(cx, w, "visibility-menu"));
}

#[gpui_kit::test]
fn hiding_applies_at_once_persists_and_the_chip_reopens_the_popover(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    let w = two_repos(cx, &file);

    open_visibility(cx, w);
    click(cx, w, ("visibility-eye", 0usize));
    // api leaves the sidebar at once; the count follows the list and the chip appears.
    assert_eq!(header_texts(cx, w), ["REPOSITORIES · 1"]);
    // The hidden api keeps the selection, so the visible web row is not selected.
    assert_eq!(label_of(cx, w, ("repo", 0usize)), "web, 1 worktree");
    assert_eq!(file.load().unwrap().hidden_repos, [PathBuf::from("/c/api")]);
    // The popover stays open, api now hidden.
    assert_eq!(visibility_rows(cx, w), ["api, hidden", "web, shown"]);

    // The chip reopens the same popover.
    keys(cx, w, &["escape"]);
    click(cx, w, "hidden-chip");
    assert!(found(cx, w, "visibility-menu"));

    click(cx, w, ("visibility-eye", 0usize));
    assert_eq!(header_texts(cx, w), ["REPOSITORIES · 2"]);
    assert!(!found(cx, w, "hidden-chip"));
    assert!(file.load().unwrap().hidden_repos.is_empty());
    assert_eq!(visibility_rows(cx, w), ["api, shown", "web, shown"]);
    assert_eq!(
        label_of(cx, w, ("repo", 0usize)),
        "api, 5 worktrees, selected"
    );
}

#[gpui_kit::test]
fn hiding_a_pinned_repo_leaves_pinned_and_returns_it_intact(cx: &mut TestAppContext) {
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
    let mut web = canned();
    web.display_name = "web".into();
    web.repo.name = "web".into();
    web.repo.path = "/c/web".into();
    web.repo.git_dir = "/c/web/.git".into();
    web.worktrees.truncate(1);
    web.worktrees[0].path = "/c/web".into();
    let (w, main) = launch_repos(cx, &file, vec![web, canned()], size(px(1180.), px(720.)));

    open_visibility(cx, w);
    // Rows are pinned web first, then api.
    assert_eq!(visibility_rows(cx, w), ["web, shown", "api, shown"]);
    let before = details(cx, &main);
    click(cx, w, ("visibility-eye", 0usize));
    assert_eq!(header_texts(cx, w), ["REPOSITORIES · 1"]);
    assert_eq!(
        file.load().unwrap().pinned_repos,
        [PathBuf::from("/c/web")],
        "the pin is preserved"
    );
    // The selected repo keeps the details pane.
    assert_eq!(details(cx, &main), before);

    // Hidden rows are out of arrow-key navigation: down from api lands back on api.
    tab_to(cx, w, ("repo", 0usize));
    keys(cx, w, &["down"]);
    assert_eq!(
        label_of(cx, w, ("repo", 0usize)),
        "api, 5 worktrees, selected"
    );

    click(cx, w, ("visibility-eye", 0usize));
    assert_eq!(header_texts(cx, w), ["PINNED · 1", "REPOSITORIES · 2"]);
    assert_eq!(label_of(cx, w, ("repo", 0usize)), "web, 1 worktree, pinned");
}

#[gpui_kit::test]
fn the_filter_narrows_rows_and_resets_on_close(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    let w = two_repos(cx, &file);

    open_visibility(cx, w);
    click(cx, w, "visibility-filter");
    cx.update_window(w, |_, window, cx| window.input("ap", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(visibility_rows(cx, w), ["api, shown"]);

    keys(cx, w, &["escape"]);
    open_visibility(cx, w);
    assert_eq!(visibility_rows(cx, w), ["api, shown", "web, shown"]);
}

#[gpui_kit::test]
fn visibility_toggles_have_accessible_labels(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    let w = two_repos(cx, &file);
    open_visibility(cx, w);
    assert_eq!(label_of(cx, w, ("visibility-eye", 0usize)), "Hide api");
    click(cx, w, ("visibility-eye", 0usize));
    assert_eq!(label_of(cx, w, ("visibility-eye", 0usize)), "Show api");
}
