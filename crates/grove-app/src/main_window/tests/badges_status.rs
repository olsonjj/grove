use std::path::Path;
use std::time::{Duration, Instant};

use gpui_kit::TestAppContext;
use grove_core::{
    Outcome, Presence, RowId, Settings, Snapshot, SnapshotStore, Upstream, WorktreeStatus,
};
use tempfile::TempDir;

use super::super::sidebar::rows::fetch_label;
use super::super::*;
use super::common::*;
use crate::shell::Library;
use crate::test_support::{canned, code, fake_launcher, fake_presence, settings_file};

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
                head: None,
                head_time: None,
                changes_time: None,
            }),
        );
    });
    cx.run_until_parked();
    assert_eq!(row_badges(cx, &main, "/c/wt/api-x"), ["3 changed", "↑2 ↓1"]);
    let after = main.read_with(cx, |m, cx| m.tree.read(cx).selected().cloned());
    assert_eq!(after, before);
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
        "Can't open /c/wt/api-x: folder is missing (use Remove missing entry…)"
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

#[gpui_kit::test]
fn fetch_all_failure_shows_repo_and_error_and_marks_counts_stale(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    // `main` two behind `origin/main`, whose remote is unreachable.
    let code = tmp.path().join("code");
    let api = code.join("api");
    std::fs::create_dir_all(&api).unwrap();
    git(&api, &["init", "-q"]);
    for m in ["one", "two", "three"] {
        git(&api, &["commit", "-q", "--allow-empty", "-m", m]);
    }
    let gone = tmp.path().join("gone.git");
    git(&api, &["remote", "add", "origin", gone.to_str().unwrap()]);
    git(&api, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    git(&api, &["branch", "-q", "-u", "origin/main"]);
    git(&api, &["reset", "-q", "--hard", "HEAD~2"]);
    let mut repo = grove_core::scan(&code).repos.remove(0);
    let path = repo.worktrees[0].path.clone();
    repo.worktrees[0].status = grove_core::read_worktree_status(&path).ok().map(Ok);
    let (w, main) = launch_repo(cx, &file, repo);
    assert_eq!(cx.update(|cx| fetch_label(cx)), "Not fetched yet");
    let path = path.to_str().unwrap();
    assert_eq!(row_badges(cx, &main, path), ["main worktree", "↓2"]);

    click(cx, w, "fetch-all");

    let s = status(cx, &main).unwrap();
    assert!(s.text.starts_with("Fetch failed for api: "), "{}", s.text);
    assert!(s.error);
    assert_eq!(
        row_badges(cx, &main, path),
        ["main worktree", "↓2", "stale"]
    );
    // Nothing fetched, so no fetch time over the stale counts.
    assert_eq!(cx.update(|cx| fetch_label(cx)), "Not fetched yet");
}

#[gpui_kit::test]
fn fetch_report_shows_once_in_status_line(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    let (_, main) = launch(cx, &file);
    cx.update(|cx| {
        let fetcher = cx.default_global::<shell::Fetcher>();
        fetcher.finished += 1;
        fetcher.report = Some(grove_core::FetchReport {
            fetched: 3,
            failures: vec![],
        });
    });
    cx.run_until_parked();
    assert_eq!(
        status(cx, &main).unwrap(),
        Status {
            text: "Fetched 3 repos".into(),
            error: false,
            settings: false,
        }
    );

    // A later fetch starting doesn't repeat the old report.
    main.update(cx, |m, _| m.status = None);
    cx.update(|cx| cx.global_mut::<shell::Fetcher>().running = true);
    cx.run_until_parked();
    assert_eq!(status(cx, &main), None);
    assert_eq!(cx.update(|cx| fetch_label(cx)), "Not fetched yet");
}

#[gpui_kit::test]
fn status_bar_shows_last_fetch_time(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    launch(cx, &file);
    cx.update(|cx| {
        let snapshot = Snapshot {
            last_fetch: Some(grove_core::unix_now() - 180),
            ..shell::library(cx).store.snapshot().as_ref().clone()
        };
        cx.global_mut::<Library>().store = SnapshotStore::from_snapshot(snapshot);
    });
    assert_eq!(cx.update(|cx| fetch_label(cx)), "Last fetch 3 minutes ago");
}

#[gpui_kit::test]
fn background_fetch_runs_every_15_minutes_only_when_on(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    launch(cx, &file);
    cx.update(shell::start_background_fetch);
    let fetches = |cx: &mut TestAppContext| {
        cx.update(|cx| cx.try_global::<shell::Fetcher>().map_or(0, |f| f.finished))
    };
    let quarter_hour = std::time::Duration::from_secs(15 * 60);

    cx.executor().advance_clock(quarter_hour);
    cx.run_until_parked();
    assert_eq!(fetches(cx), 0);

    cx.update(|cx| cx.global_mut::<Library>().settings.background_fetch = true);
    cx.executor()
        .advance_clock(quarter_hour - std::time::Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(fetches(cx), 0);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(fetches(cx), 1);
    cx.executor().advance_clock(quarter_hour);
    cx.run_until_parked();
    assert_eq!(fetches(cx), 2);
}

#[gpui_kit::test]
fn notice_posted_with_no_main_window_shows_when_one_opens(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    let hint = Status {
        text: "Allow Grove to control Terminal".into(),
        error: true,
        settings: false,
    };
    cx.update(|cx| shell::post(hint.clone(), cx));
    let (_, main) = launch(cx, &file);
    assert_eq!(status(cx, &main), Some(hint.clone()));
    // Once shown, the next window doesn't repeat it.
    let (_, again) = launch(cx, &file);
    assert_eq!(status(cx, &again), None);

    // Nor a stale one.
    cx.update(|cx| {
        shell::post(hint, cx);
        let at = Instant::now()
            .checked_sub(Duration::from_secs(120))
            .unwrap();
        cx.global_mut::<shell::Notices>().posted_at = Some(at);
    });
    let (_, late) = launch(cx, &file);
    assert_eq!(status(cx, &late), None);
}
