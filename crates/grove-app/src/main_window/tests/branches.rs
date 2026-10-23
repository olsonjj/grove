use std::path::{Path, PathBuf};

use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt;
use grove_core::{Outcome, RowId, Settings};
use tempfile::TempDir;

use super::super::*;
use super::common::*;
use crate::test_support::{canned, fake_launcher, settings_file, tab_to};

#[gpui_kit::test]
fn branch_trigger_shows_the_worktree_branch_on_main_and_linked(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    lister(cx, &["feature/x", "fix/z", "main", "spare"]);
    let (w, main) = launch_repo(cx, &file, canned());

    // Main worktree, then linked: the branch line sits at the top of the pane for
    // every worktree, with each worktree's own branch as the value.
    select_worktree(cx, w, &main, "/c/api");
    assert!(found(cx, w, "details-branch"));
    assert_eq!(
        label_of(cx, w, "details-branch"),
        "Switch the worktree's branch"
    );
    assert_eq!(trigger_text(cx, &main).as_deref(), Some("main"));
    // The branch line is the trigger now, so the STATUS section leaves it out.
    let fields = main.read_with(cx, |m, cx| m.details.read(cx).status_fields());
    assert_eq!(fields, [("Working tree", "reading…".to_string())]);

    select_worktree(cx, w, &main, "/c/wt/api-x");
    assert_eq!(trigger_text(cx, &main).as_deref(), Some("feature/x"));
}

#[gpui_kit::test]
fn branch_dropdown_lists_read_branches_alphabetically_and_searchable(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    lister(cx, &["spare", "fix/z", "feature/x", "main"]);
    let (w, main) = launch_repo(cx, &file, canned());
    select_worktree(cx, w, &main, "/c/api");
    assert_eq!(trigger_text(cx, &main).as_deref(), Some("main"));

    click(cx, w, "details-branch");

    // Alphabetical; the branches the repo's other worktrees hold are annotated.
    assert_eq!(
        dropdown(cx, w),
        [
            "feature/x, checked out in worktree api-x/",
            "fix/z, checked out in worktree api-z/",
            "main",
            "spare",
        ]
    );

    // The search narrows the list.
    cx.update_window(w, |_, window, cx| window.input("sp", cx))
        .unwrap();
    assert_eq!(dropdown(cx, w), ["spare"]);
    cx.update_window(w, |_, window, cx| window.press("escape", cx))
        .unwrap();
}

#[gpui_kit::test]
fn branch_held_by_another_worktree_is_annotated_and_refused(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    lister(cx, &["main", "feature/x", "fix/z"]);
    let switches = fake_switcher(cx, None);
    let (w, main) = launch_repo(cx, &file, canned());

    // The pane shows the linked worktree on feature/x; the repo's main worktree
    // holds main, so that pick is impossible and starts no write.
    select_worktree(cx, w, &main, "/c/wt/api-x");
    click(cx, w, "details-branch");
    assert_eq!(
        dropdown(cx, w),
        [
            "feature/x",
            "fix/z, checked out in worktree api-z/",
            "main, checked out in worktree api/",
        ]
    );

    click(cx, w, ("branch-item", 2usize));

    assert!(switches.lock().unwrap().is_empty());
    // The trigger keeps the worktree's own branch.
    assert_eq!(trigger_text(cx, &main).as_deref(), Some("feature/x"));
}

#[gpui_kit::test]
fn detached_worktree_shows_the_placeholder_and_picks_a_branch(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    lister(cx, &["main", "feature/x", "next"]);
    let switches = fake_switcher(cx, None);
    let (w, main) = launch_repo(cx, &file, canned());
    select_worktree(cx, w, &main, "/c/wt/bisect");
    // A detached worktree keeps its identity line and shows the placeholder; the
    // STATUS row still says where its detached HEAD is.
    let fields = main.read_with(cx, |m, cx| m.details.read(cx).status_fields());
    assert!(fields.contains(&("Branch", "detached @ a1f3c9e".to_string())));
    assert_eq!(trigger_text(cx, &main).as_deref(), Some("No branch"));

    click(cx, w, "details-branch");
    // The repo's held branches are annotated; only one pick remains possible.
    assert_eq!(
        dropdown(cx, w),
        [
            "feature/x, checked out in worktree api-x/",
            "main, checked out in worktree api/",
            "next",
        ]
    );
    click(cx, w, ("branch-item", 2usize));
    cx.run_until_parked();

    assert_eq!(
        *switches.lock().unwrap(),
        [(PathBuf::from("/c/wt/bisect"), "next".into())]
    );
    let s = status(cx, &main).unwrap();
    assert_eq!(s.text, "Switched to `next`");
}

#[gpui_kit::test]
fn branch_trigger_refuses_a_second_open_while_a_switch_runs(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    lister(cx, &["feature/x", "main", "spare"]);
    let switches = fake_switcher(cx, None);
    let (w, main) = launch_repo(cx, &file, canned());
    select_worktree(cx, w, &main, "/c/api");

    let m = main.clone();
    cx.update_window(w, |_, window, cx| {
        m.update(cx, |m, cx| {
            m.details.update(cx, |d, cx| {
                d.hold_switch(
                    PathBuf::from("/c/api"),
                    "spare".into(),
                    PathBuf::from("/c/api/.git"),
                    window,
                    cx,
                );
            });
        });
    })
    .unwrap();
    cx.update_window(w, |_, window, cx| window.render_frame(cx))
        .unwrap();

    // The trigger reads "Switching…", has no committed value, and the dropdown
    // refuses to open while the write runs; no second write starts either way.
    assert_eq!(trigger_text(cx, &main).as_deref(), Some("Switching…"));
    assert_eq!(
        switching(cx, &main),
        Some((PathBuf::from("/c/api"), "spare".into()))
    );
    click(cx, w, "details-branch");
    assert!(dropdown(cx, w).is_empty(), "the dropdown must not reopen");
    assert!(switches.lock().unwrap().is_empty());
}

#[gpui_kit::test]
fn selection_change_during_a_switch_doesnt_cancel_it(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    lister(cx, &["feature/x", "main", "spare"]);
    let switches = fake_switcher(cx, Some("fatal: cannot switch because changes\n"));
    let (w, main) = launch_repo(cx, &file, canned());
    select_worktree(cx, w, &main, "/c/api");

    click(cx, w, "details-branch");
    click(cx, w, ("branch-item", 2usize));

    // Browsing while the write runs: the new row's own branch shows in the trigger,
    // and the running switch is neither cancelled nor repeated.
    let other = row_of(cx, &main, Path::new("/c/wt/api-x"));
    click(cx, w, ("tree-row", other));
    assert_eq!(trigger_text(cx, &main).as_deref(), Some("feature/x"));

    cx.run_until_parked();
    // The outcome notice still lands, exactly once, tied to the switched worktree.
    let s = status(cx, &main).unwrap();
    assert_eq!(
        s.text,
        "Couldn't switch to `spare`: cannot switch because changes"
    );
    assert!(s.error);
    assert_eq!(
        *switches.lock().unwrap(),
        [(PathBuf::from("/c/api"), "spare".into())]
    );
    assert_eq!(trigger_text(cx, &main).as_deref(), Some("feature/x"));
}

#[gpui_kit::test]
fn failed_switch_shows_gits_brief_error_and_restores_the_trigger(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    lister(cx, &["main", "spare"]);
    let switches = fake_switcher(cx, Some("fatal: cannot switch because changes\n"));
    let (w, main) = launch_repo(cx, &file, canned());
    select_worktree(cx, w, &main, "/c/api");

    click(cx, w, "details-branch");
    click(cx, w, ("branch-item", 1usize));

    cx.run_until_parked();
    let s = status(cx, &main).unwrap();
    assert_eq!(
        s.text,
        "Couldn't switch to `spare`: cannot switch because changes"
    );
    assert!(s.error);
    // Nothing changed: the trigger is back on the worktree's branch.
    assert_eq!(trigger_text(cx, &main).as_deref(), Some("main"));
    assert_eq!(switches.lock().unwrap().len(), 1);
}

#[gpui_kit::test]
fn picking_the_current_branch_puts_the_trigger_back_on_the_branch(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    lister(cx, &["feature/x", "main", "spare"]);
    let switches = fake_switcher(cx, None);
    let (w, main) = launch_repo(cx, &file, canned());
    select_worktree(cx, w, &main, "/c/api");

    // `main` is the worktree's own branch: git calls the pick a no-op and Grove
    // reports it like any switch.
    click(cx, w, "details-branch");
    click(cx, w, ("branch-item", 1usize));
    cx.run_until_parked();

    let s = status(cx, &main).unwrap();
    assert_eq!(s.text, "Switched to `main`");
    assert_eq!(
        *switches.lock().unwrap(),
        [(PathBuf::from("/c/api"), "main".into())]
    );
    // The value cleared for the run comes back after the success: the trigger reads
    // the branch, not the placeholder.
    assert_eq!(trigger_text(cx, &main).as_deref(), Some("main"));
}

#[gpui_kit::test]
fn a_running_pull_blocks_the_switcher_until_it_finishes(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    lister(cx, &["feature/x", "main", "spare"]);
    fake_puller(cx, None);
    let switches = fake_switcher(cx, None);
    let (w, main) = launch_repo(cx, &file, pull_repo());
    select_worktree(cx, w, &main, "/c/api");

    // The pull starts and stays in flight (the executor isn't parked).
    click_now(cx, w, "details-pull");
    assert_eq!(label_of(cx, w, "details-pull"), "Pulling…");

    // The branch trigger is disabled while the pull runs: the dropdown stays shut
    // and no switch starts, so pull's fast-forward can't collide with a checkout.
    click_now(cx, w, "details-branch");
    assert!(dropdown(cx, w).is_empty());
    assert!(switches.lock().unwrap().is_empty());

    cx.run_until_parked();
    let s = status(cx, &main).unwrap();
    assert_eq!(s.text, "Pulled main");
    // The pull done, the switcher is back; no switch ever started.
    click(cx, w, "details-branch");
    assert!(!dropdown(cx, w).is_empty());
    assert!(switches.lock().unwrap().is_empty());
}

#[gpui_kit::test]
fn a_running_switch_blocks_pull(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    lister(cx, &["feature/x", "main", "spare"]);
    let pulls = fake_puller(cx, None);
    let (w, main) = launch_repo(cx, &file, pull_repo());
    select_worktree(cx, w, &main, "/c/api");

    // A switch is held in flight, exactly as the pick handler leaves it.
    let m = main.clone();
    cx.update_window(w, |_, window, cx| {
        m.update(cx, |m, cx| {
            m.details.update(cx, |d, cx| {
                d.hold_switch(
                    PathBuf::from("/c/api"),
                    "spare".into(),
                    PathBuf::from("/c/api/.git"),
                    window,
                    cx,
                );
            });
        });
    })
    .unwrap();
    cx.update_window(w, |_, window, cx| window.render_frame(cx))
        .unwrap();

    // Pull waits: the button reads Pull, stays disabled, and the click starts no
    // pull — two writes never race on one worktree.
    assert_eq!(label_of(cx, w, "details-pull"), "Pull");
    click_now(cx, w, "details-pull");
    assert!(pulls.lock().unwrap().is_empty());
}

/// A failed branch-list read keeps the worktree's own branch as the value and names
/// the failure in the dropdown, instead of reading as an empty repository.
#[gpui_kit::test]
fn failed_branch_read_keeps_the_branch_and_names_the_failure(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    failing_lister(cx, "fatal: not a git repository\n");
    let (w, main) = launch_repo(cx, &file, canned());

    select_worktree(cx, w, &main, "/c/api");
    assert_eq!(trigger_text(cx, &main).as_deref(), Some("main"));
    click(cx, w, "details-branch");
    // The one branch the pane knows is still offered, and it is the value.
    assert_eq!(dropdown(cx, w), ["main"]);
    cx.update_window(w, |_, window, cx| window.press("escape", cx))
        .unwrap();

    // A worktree with no branch of its own gets the read's failure as its empty row,
    // not the same words an empty repository would say.
    select_worktree(cx, w, &main, "/c/wt/bisect");
    assert_eq!(trigger_text(cx, &main).as_deref(), Some("No branch"));
    click(cx, w, "details-branch");
    assert!(dropdown(cx, w).is_empty());
    assert_eq!(label_of(cx, w, "branch-empty"), "Couldn't list branches");
}

/// A real repo through the real switch read and write: the worktree re-nests under
/// its new branch node, the pane follows, and the selection stays row-for-row.
#[gpui_kit::test]
fn switched_worktree_renests_and_selection_stays(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    let api = tmp.path().join("code/api");
    std::fs::create_dir_all(&api).unwrap();
    git(&api, &["init", "-q"]);
    std::fs::write(api.join("f.txt"), "base\n").unwrap();
    git(&api, &["add", "-A"]);
    git(&api, &["commit", "-q", "-m", "base"]);
    git(&api, &["switch", "-q", "-c", "feature"]);
    // feature's checkout differs in size, so a switch must move the Size row too.
    std::fs::write(api.join("f.txt"), vec![b'x'; 4096]).unwrap();
    git(&api, &["add", "-A"]);
    git(&api, &["commit", "-q", "-m", "theirs"]);
    git(&api, &["switch", "-q", "main"]);
    let mut repo = grove_core::scan(&tmp.path().join("code")).repos.remove(0);
    // Test setup: the scan leaves statuses unread; the app fills them in the background.
    let wt_path = repo.worktrees[0].path.clone();
    repo.worktrees[0].status = Some(grove_core::read_worktree_status(&wt_path));
    let (w, main) = launch_repo(cx, &file, repo);
    select_worktree(cx, w, &main, wt_path.to_str().unwrap());
    cx.run_until_parked();
    assert_eq!(trigger_text(cx, &main).as_deref(), Some("main"));

    /// The pane's Size line as SizeSource keys it, across the switch below.
    fn size_line(cx: &mut TestAppContext, main: &Entity<MainWindow>) -> String {
        main.read_with(cx, |m, cx| {
            m.details
                .read(cx)
                .status_fields()
                .into_iter()
                .find_map(|(l, v)| (l == "Size").then_some(v))
                .unwrap()
        })
    }
    assert_eq!(size_line(cx, &main), "5 B");

    click(cx, w, "details-branch");
    assert_eq!(dropdown(cx, w), ["feature", "main"]);
    click(cx, w, ("branch-item", 0usize));
    cx.run_until_parked();

    let s = status(cx, &main).unwrap();
    assert_eq!(s.text, "Switched to `feature`");
    // The worktree re-nested under its new branch node while the selection stayed on
    // the worktree row; trigger and pane agree on the committed branch.
    assert_eq!(trigger_text(cx, &main).as_deref(), Some("feature"));
    let ix = row_of(cx, &main, &wt_path);
    click(cx, w, ("tree-row", ix));
    assert_eq!(
        main.read_with(cx, |m, cx| m.tree.read(cx).selected().cloned()),
        Some(RowId::Worktree(wt_path.clone()))
    );
    assert!(tree_rows_branches(cx, &main).contains(&"feature".to_string()));
    // Recent commits re-read on their own, as HEAD changed.
    let commits = main
        .read_with(cx, |m, cx| m.details.read(cx).commits().cloned())
        .unwrap();
    match commits {
        Ok(commits) => assert_eq!(commits[0].subject, "theirs"),
        err => panic!("expected commits, got {err:?}"),
    }
    // The checkout also moved HEAD, so the size key moved and the walk re-read:
    // feature's f.txt is bigger.
    assert_eq!(size_line(cx, &main), "4 kB");
}

/// The trigger is in the focus order, and the dropdown works by keyboard alone:
/// Enter opens, arrows move the cursor, Enter picks, and Escape closes without picking.
#[gpui_kit::test]
fn branch_trigger_is_keyboard_operable(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    lister(cx, &["spare", "main"]);
    let switches = fake_switcher(cx, None);
    let (w, main) = launch_repo(cx, &file, canned());
    select_worktree(cx, w, &main, "/c/api");
    assert_eq!(trigger_text(cx, &main).as_deref(), Some("main"));

    // Reach the branch line by focus order, open it, and close it with Escape.
    tab_to(cx, w, "details-branch");
    cx.update_window(w, |_, window, cx| window.press("enter", cx))
        .unwrap();
    assert_eq!(dropdown(cx, w), ["main", "spare"]);
    cx.update_window(w, |_, window, cx| window.press("escape", cx))
        .unwrap();
    assert!(dropdown(cx, w).is_empty());

    // Reopen, move the cursor down to `spare`, and pick with Enter.
    cx.update_window(w, |_, window, cx| {
        window.press("enter", cx);
        window.press("down", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();

    assert_eq!(
        *switches.lock().unwrap(),
        [(PathBuf::from("/c/api"), "spare".into())]
    );
}

/// A branch created in a terminal lands in the dropdown when the repo is reread,
/// with no Grove action: the watcher's ref hits drive repo-level refreshes.
#[gpui_kit::test]
fn branch_created_in_a_terminal_lands_after_repo_refresh(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    let api = tmp.path().join("code/api");
    std::fs::create_dir_all(&api).unwrap();
    git(&api, &["init", "-q"]);
    git(&api, &["commit", "-q", "--allow-empty", "-m", "base"]);
    let mut repo = grove_core::scan(&tmp.path().join("code")).repos.remove(0);
    let wt_path = repo.worktrees[0].path.clone();
    let git_dir = repo.repo.git_dir.clone();
    repo.worktrees[0].status = Some(grove_core::read_worktree_status(&wt_path));
    let (w, main) = launch_repo(cx, &file, repo);
    select_worktree(cx, w, &main, wt_path.to_str().unwrap());
    click(cx, w, "details-branch");
    assert_eq!(dropdown(cx, w), ["main"]);
    cx.update_window(w, |_, window, cx| window.press("escape", cx))
        .unwrap();

    // Created outside Grove; the repo reread that follows a ref change rebuilds the
    // list.
    git(&api, &["branch", "later"]);
    cx.update(|cx| shell::reread_repo(git_dir.as_path(), cx));
    cx.run_until_parked();

    click(cx, w, "details-branch");
    assert_eq!(dropdown(cx, w), ["later", "main"]);
    cx.update_window(w, |_, window, cx| window.press("escape", cx))
        .unwrap();

    // Deleted outside Grove, it drops the same way.
    git(&api, &["branch", "-D", "later"]);
    cx.update(|cx| shell::reread_repo(git_dir.as_path(), cx));
    cx.run_until_parked();

    click(cx, w, "details-branch");
    assert_eq!(dropdown(cx, w), ["main"]);
}
