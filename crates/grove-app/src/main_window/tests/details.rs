use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt;
use grove_core::{Outcome, Presence, Settings, Upstream, WorktreeStatus, relative_time};
use tempfile::TempDir;

use super::super::*;
use super::common::*;
use crate::test_support::{canned, fake_launcher, fake_presence, settings_file};

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
        head: None,
        head_time: None,
        changes_time: None,
    }));
    let (w, main) = launch_repo(cx, &file, repo);
    cx.run_until_parked();
    let fields = main.read_with(cx, |m, cx| m.details.read(cx).status_fields());
    assert_eq!(
        fields,
        [
            ("Upstream", "origin/main".into()),
            ("Sync", "2 ahead, 1 behind".into()),
            ("Working tree", "2 changed files".into()),
            ("Size", "0 B".into()),
        ]
    );
    assert!(has(cx, w, "changed-file", 0));
    assert!(has(cx, w, "changed-file", 1));
    assert!(!has(cx, w, "changed-file", 2));
    keys(cx, w, &["down", "down", "down", "down"]);
    assert!(!has(cx, w, "changed-file", 0));
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

#[gpui_kit::test]
fn details_open_disabled_for_missing_worktree(cx: &mut TestAppContext) {
    let (launches, s) = try_details_opens(cx, Presence::Missing);
    assert!(launches.lock().unwrap().is_empty());
    assert_eq!(
        s.text,
        "Can't reveal /c/wt/api-x: folder is missing (use Remove missing entry…)"
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
        "Can't open /c/wt/api-x: folder is missing (use Remove missing entry…)"
    );
    // The refusal rechecked presence, so the pane now shows it disabled.
    click(cx, w, "details-open-editor");
    assert!(launches.lock().unwrap().is_empty());
    assert_eq!(row_badges(cx, &main, "/c/wt/api-x"), ["missing"]);
}

#[gpui_kit::test]
fn details_list_last_five_commits_in_background(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    let repo_dir = tmp.path().join("api");
    std::fs::create_dir(&repo_dir).unwrap();
    git(&repo_dir, &["init", "-q"]);
    for m in ["one", "two", "three", "four", "five", "six"] {
        git(
            &repo_dir,
            &[
                "-c",
                "user.name=olsonjj",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                m,
            ],
        );
    }
    let mut repo = canned();
    repo.worktrees[0].path = repo_dir;
    let (w, main) = launch_repo(cx, &file, repo);
    cx.run_until_parked();
    assert!((0..5).all(|i| has(cx, w, "commit", i)));
    assert!(!has(cx, w, "commit", 5));
    // Two-line rows: each commit's unit (its preserved `commit` id) names who wrote
    // it and when, per the pane's own read.
    let commits = main
        .read_with(cx, |m, cx| m.details.read(cx).commits().cloned())
        .unwrap()
        .unwrap();
    let now = grove_core::unix_now();
    for (i, c) in commits.iter().enumerate() {
        let label = cx
            .update_window(w, |_, window, cx| {
                window.render_frame(cx);
                window.find(("commit", i)).label().unwrap().to_string()
            })
            .unwrap();
        // The row reads as one unit (VoiceOver): hash, author, time, subject.
        assert!(label.contains(c.short_sha.as_str()), "{label}");
        assert!(label.contains(c.author.as_str()), "{label}");
        assert!(
            label.contains(relative_time(c.time, now).as_str()),
            "{label}"
        );
        assert!(label.contains(c.subject.as_str()), "{label}");
        // And the visible leaves themselves carry what they render: the metadata
        // line's author and time, not just the row's composed label.
        let (author, time_text) = cx
            .update_window(w, |_, window, cx| {
                window.render_frame(cx);
                (
                    window
                        .find(("commit-author", i))
                        .label()
                        .unwrap_or_default()
                        .to_string(),
                    window.find(("commit-time", i)).label().unwrap().to_string(),
                )
            })
            .unwrap();
        assert_eq!(author, "olsonjj");
        assert_eq!(time_text, relative_time(c.time, now));
    }
    // A linked worktree that isn't a repo shows the error instead of stale commits.
    keys(cx, w, &["down", "down", "down", "down"]);
    let err = main.read_with(cx, |m, cx| match m.details.read(cx).commits() {
        Some(Err(e)) => e.to_string(),
        other => panic!("expected an error, got {other:?}"),
    });
    assert!(
        err.starts_with("git log -5") && err.contains("failed:"),
        "{err}"
    );
    assert!(!has(cx, w, "commit", 0));
}

#[gpui_kit::test]
fn details_show_the_worktree_size(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    let repo_dir = tmp.path().join("api");
    std::fs::create_dir(&repo_dir).unwrap();
    git(&repo_dir, &["init", "-q"]);
    // Three kilobytes across a nested folder, committed so the tree is clean: "3 kB".
    std::fs::create_dir(repo_dir.join("d")).unwrap();
    std::fs::write(repo_dir.join("d/plain.bin"), vec![0u8; 3072]).unwrap();
    git(&repo_dir, &["add", "-A"]);
    git(&repo_dir, &["commit", "-q", "-m", "one"]);
    let mut repo = grove_core::scan(tmp.path()).repos.remove(0);
    repo.worktrees[0].status = grove_core::read_worktree_status(&repo.worktrees[0].path)
        .ok()
        .map(Ok);
    let (_w, main) = launch_repo(cx, &file, repo);
    cx.run_until_parked();
    let fields = main.read_with(cx, |m, cx| m.details.read(cx).status_fields());
    assert_eq!(
        fields,
        [
            ("Upstream", "none".into()),
            ("Sync", "no upstream".into()),
            ("Working tree", "clean".into()),
            ("Size", "3 kB".into()),
        ]
    );
}

/// An edit mid-session re-keys the walk (newest changes mtime); the last landed
/// size keeps showing until the fresh walk lands, however much the key moves around.
#[gpui_kit::test]
fn an_edit_during_size_walks_keeps_the_last_size_showing(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    let api = tmp.path().join("code/api");
    std::fs::create_dir_all(&api).unwrap();
    git(&api, &["init", "-q"]);
    std::fs::write(api.join("f.txt"), "base\n").unwrap();
    git(&api, &["add", "-A"]);
    git(&api, &["commit", "-q", "-m", "base"]);
    let mut repo = grove_core::scan(&tmp.path().join("code")).repos.remove(0);
    // Test setup: the scan leaves statuses unread; the app fills them in the background.
    let wt_path = repo.worktrees[0].path.clone();
    repo.worktrees[0].status = Some(grove_core::read_worktree_status(&wt_path));
    let (w, main) = launch_repo(cx, &file, repo);
    /// The pane's Size line, across the edit below.
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
    select_worktree(cx, w, &main, wt_path.to_str().unwrap());
    cx.run_until_parked();
    assert_eq!(size_line(cx, &main), "5 B");

    // The edit moves the key and starts a fresh walk — still in flight, since the
    // executor isn't parked.
    std::fs::write(wt_path.join("f.txt"), vec![b'x'; 4096]).unwrap();
    let m = main.clone();
    cx.update_window(w, |_, window, cx| {
        m.update(cx, |m, cx| {
            m.details.update(cx, |d, cx| {
                d.hold_size_walk(cx);
            });
        });
        window.render_frame(cx);
    })
    .unwrap();
    // The last known size shows across the pending read, not the placeholder.
    assert_eq!(size_line(cx, &main), "5 B");

    cx.run_until_parked();
    // The fresh walk lands and the row catches up with the edit.
    assert_eq!(size_line(cx, &main), "4 kB");
}
