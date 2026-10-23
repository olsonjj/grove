use std::path::{Path, PathBuf};

use gpui_kit::TestAppContext;
use grove_core::{GitError, Outcome, Settings, Upstream};
use tempfile::TempDir;

use super::common::*;
use crate::test_support::{fake_launcher, settings_file};

#[gpui_kit::test]
fn pull_button_offers_per_upstream_state(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    let repo = pull_repo();
    let (w, main) = launch_repo(cx, &file, repo);
    let row = |cx: &mut TestAppContext, p: &str| row_of(cx, &main, Path::new(p));

    // main: behind 1 → Pull beside the Sync field, whose line shows the same counts.
    let ix = row(cx, "/c/api");
    click(cx, w, ("tree-row", ix));
    assert_eq!(label_of(cx, w, "details-pull"), "Pull");
    assert_eq!(sync_line(cx, &main), "1 behind");

    for (path, why) in [
        ("/c/wt/api-x", "up to date"),
        ("/c/wt/api-y", "no upstream"),
        ("/c/wt/api-z", "upstream gone"),
        ("/c/wt/bisect", "detached"),
    ] {
        let ix = row(cx, path);
        click(cx, w, ("tree-row", ix));
        assert!(!found(cx, w, "details-pull"), "Pull offered for {why}");
    }
}

#[gpui_kit::test]
fn failed_fetch_counts_dont_block_the_button(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    let mut repo = pull_repo();
    repo.fetch_error = Some(GitError::Failed {
        args: "fetch --all --prune --quiet".into(),
        stderr: "fatal: unable to access: offline\n".into(),
    });
    let (w, main) = launch_repo(cx, &file, repo);

    let ix = row_of(cx, &main, Path::new("/c/api"));
    click(cx, w, ("tree-row", ix));
    assert_eq!(label_of(cx, w, "details-pull"), "Pull");
}

#[gpui_kit::test]
fn pull_click_runs_once_and_posts_outcome(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    let pulls = fake_puller(cx, None);
    let (w, main) = launch_repo(cx, &file, pull_repo());
    let ix = row_of(cx, &main, Path::new("/c/api"));
    click(cx, w, ("tree-row", ix));
    // Runs in the background (“Pulling…”), disabled; a second click is refused.
    assert_eq!(click_and_look(cx, w, "details-pull", 2), "Pulling…");

    cx.run_until_parked();
    assert_eq!(*pulls.lock().unwrap(), [PathBuf::from("/c/api")]);
    assert_eq!(status(cx, &main).unwrap().text, "Pulled main");
    assert!(!status(cx, &main).unwrap().error);
}

#[gpui_kit::test]
fn pulled_worktree_rereads_its_status_badge_and_commits(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    let pulls = real_puller(cx);
    // Bare origin plus a teammate clone, as fetch's tests set up: a `feature` worktree
    // one behind its upstream.
    let remotes = tmp.path().join("remotes");
    std::fs::create_dir_all(&remotes).unwrap();
    let origin = remotes.join("origin.git");
    git(&remotes, &["init", "-q", "--bare", "origin.git"]);
    let api = tmp.path().join("code/api");
    std::fs::create_dir_all(&api).unwrap();
    git(&api, &["init", "-q"]);
    std::fs::write(api.join("f.txt"), "base\n").unwrap();
    git(&api, &["add", "-A"]);
    git(&api, &["commit", "-q", "-m", "base"]);
    git(&api, &["remote", "add", "origin", origin.to_str().unwrap()]);
    git(&api, &["push", "-q", "-u", "origin", "main"]);
    let wt = tmp.path().join("wt/api-feature");
    git(
        &api,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature",
            wt.to_str().unwrap(),
        ],
    );
    // The scan reports canonical paths; keep ours comparable.
    let wt = wt.canonicalize().unwrap();
    git(&wt, &["push", "-q", "-u", "origin", "feature"]);
    let other = tmp.path().join("other");
    git(
        tmp.path(),
        &["clone", "-q", origin.to_str().unwrap(), "other"],
    );
    git(&other, &["checkout", "-q", "feature"]);
    git(&other, &["commit", "-q", "--allow-empty", "-m", "theirs"]);
    git(&other, &["push", "-q"]);
    // Test setup: refresh the tracking refs so the scan shows the worktree behind.
    git(&wt, &["fetch", "-q"]);
    let mut repo = grove_core::scan(&tmp.path().join("code")).repos.remove(0);
    // Test setup: the scan leaves statuses unread; the app fills them in the background.
    let entry = repo.worktrees.iter_mut().find(|w| w.path == wt).unwrap();
    entry.status = Some(grove_core::read_worktree_status(&wt));
    let (w, main) = launch_repo(cx, &file, repo);
    let ix = row_of(cx, &main, &wt);
    click(cx, w, ("tree-row", ix));
    assert_eq!(sync_line(cx, &main), "1 behind");
    assert_eq!(row_badges(cx, &main, wt.to_str().unwrap()), ["\u{2193}1"]);

    // Runs in the background (“Pulling…”), disabled; a second click is refused.
    assert_eq!(click_and_look(cx, w, "details-pull", 2), "Pulling…");
    cx.run_until_parked();

    assert_eq!(*pulls.lock().unwrap(), vec![wt.clone()]);
    assert_eq!(status(cx, &main).unwrap().text, "Pulled feature");
    assert!(!status(cx, &main).unwrap().error);
    // The reread after success leaves the worktree up to date: the badge clears, the
    // sync line agrees, and the button is gone (nothing left to pull).
    assert_eq!(
        row_badges(cx, &main, wt.to_str().unwrap()),
        Vec::<String>::new()
    );
    assert_eq!(sync_line(cx, &main), "up to date");
    assert!(!found(cx, w, "details-pull"));
    // Recent commits re-read on their own, as HEAD changed.
    let commits = main
        .read_with(cx, |m, cx| m.details.read(cx).commits().cloned())
        .unwrap();
    match commits {
        Ok(commits) => assert_eq!(commits[0].subject, "theirs"),
        err => panic!("expected commits, got {err:?}"),
    }
}

#[gpui_kit::test]
fn selection_change_during_pull_doesnt_cancel_it(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    let pulls = fake_puller(cx, None);
    let mut repo = pull_repo();
    // A second pullable worktree, whose button must refuse while one runs.
    repo.worktrees[3].status = upstream_status(Upstream::Tracking {
        name: "origin/fix/z".into(),
        ahead: 0,
        behind: 2,
    });
    let (w, main) = launch_repo(cx, &file, repo);
    let ix = row_of(cx, &main, Path::new("/c/api"));
    click(cx, w, ("tree-row", ix));
    // Start the pull and hold it in flight: nothing parked yet.
    assert_eq!(click_and_look(cx, w, "details-pull", 2), "Pulling…");

    // Moving the selection away doesn't cancel it or start another: the button on the
    // also-behind worktree is there but refuses the click.
    let other = row_of(cx, &main, Path::new("/c/wt/api-z"));
    click_now(cx, w, ("tree-row", other));
    assert_eq!(click_and_look(cx, w, "details-pull", 2), "Pull");

    cx.run_until_parked();
    // The first pull's outcome still lands, and nothing else was pulled.
    assert_eq!(*pulls.lock().unwrap(), [PathBuf::from("/c/api")]);
    assert_eq!(status(cx, &main).unwrap().text, "Pulled main");

    // With no pull running, the still-behind worktree pulls once, its own second
    // click refused while it runs.
    click_now(cx, w, "details-pull");
    assert_eq!(click_and_look(cx, w, "details-pull", 2), "Pulling…");
    cx.run_until_parked();
    assert_eq!(
        *pulls.lock().unwrap(),
        [PathBuf::from("/c/api"), PathBuf::from("/c/wt/api-z")]
    );
    assert_eq!(status(cx, &main).unwrap().text, "Pulled fix/z");
}

#[gpui_kit::test]
fn failed_pull_shows_gits_brief_error_and_changes_nothing(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    let pulls = fake_puller(cx, Some("fatal: Not possible to fast-forward, aborting.\n"));
    let (w, main) = launch_repo(cx, &file, pull_repo());
    let ix = row_of(cx, &main, Path::new("/c/api"));
    click(cx, w, ("tree-row", ix));

    click(cx, w, "details-pull");

    assert_eq!(pulls.lock().unwrap().len(), 1);
    let s = status(cx, &main).unwrap();
    assert_eq!(
        s.text,
        "Couldn't pull main: Not possible to fast-forward, aborting."
    );
    assert!(s.error);
    // Nothing changed: the counts are as they were and the button is back.
    assert_eq!(sync_line(cx, &main), "1 behind");
    assert_eq!(label_of(cx, w, "details-pull"), "Pull");
}
