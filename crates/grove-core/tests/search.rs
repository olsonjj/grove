// Expected highlight ranges read best as `[start..end]`.
#![expect(clippy::single_range_in_vec_init)]

use std::path::PathBuf;

use grove_core::{
    Badge, Hit, Presence, RepoRef, RepoSnapshot, Snapshot, Upstream, Worktree, WorktreeStatus,
    search,
};

fn wt(path: &str, branch: Option<&str>) -> Worktree {
    Worktree {
        path: path.into(),
        branch: branch.map(str::to_string),
        head: Some("a1f3c9e5d2".into()),
        detached: branch.is_none(),
        is_main: false,
        locked: false,
        presence: Presence::Present,
        status: None,
    }
}

fn repo(name: &str, worktrees: Vec<Worktree>) -> RepoSnapshot {
    RepoSnapshot {
        display_name: name.into(),
        repo: RepoRef {
            name: name.into(),
            path: format!("/c/{name}").into(),
            git_dir: format!("/c/{name}/.git").into(),
            bare: false,
        },
        worktrees,
        error: None,
        fetch_error: None,
    }
}

/// api: main, feature/auth-refresh, fix/rate-limit, a detached bisect; web: main,
/// feature/auth-ui.
fn snapshot() -> Snapshot {
    Snapshot {
        code_dir: "/c".into(),
        code_dir_exists: true,
        repos: vec![
            repo(
                "api",
                vec![
                    Worktree {
                        is_main: true,
                        ..wt("/c/api", Some("main"))
                    },
                    wt("/c/wt/auth-refresh", Some("feature/auth-refresh")),
                    wt("/c/wt/rate-limit", Some("fix/rate-limit")),
                    wt("/c/wt/bisect", None),
                ],
            ),
            repo(
                "web",
                vec![
                    Worktree {
                        is_main: true,
                        ..wt("/c/web", Some("main"))
                    },
                    wt("/c/wt/web-auth", Some("feature/auth-ui")),
                ],
            ),
        ],
        last_fetch: None,
    }
}

fn paths(hits: &[Hit]) -> Vec<&str> {
    hits.iter().map(|h| h.path.to_str().unwrap()).collect()
}

fn find<'a>(hits: &'a [Hit], path: &str) -> &'a Hit {
    hits.iter().find(|h| h.path.to_str() == Some(path)).unwrap()
}

#[test]
fn empty_query_lists_recents_first_then_everything_in_order() {
    let recents: Vec<PathBuf> = vec!["/c/wt/web-auth".into(), "/gone".into(), "/c/api".into()];
    let hits = search(&snapshot(), &recents, "  ");
    assert_eq!(
        paths(&hits),
        [
            "/c/wt/web-auth",
            "/c/api",
            "/c/wt/auth-refresh",
            "/c/wt/rate-limit",
            "/c/wt/bisect",
            "/c/web",
        ]
    );
    assert!(hits[0].recent && hits[1].recent && !hits[2].recent);
    assert!(hits.iter().all(|h| h.matches == Default::default()));
}

#[test]
fn contiguous_match_in_branch_is_highlighted() {
    let hits = search(&snapshot(), &[], "rate");
    assert_eq!(hits[0].path, PathBuf::from("/c/wt/rate-limit"));
    // Only the best field (branch over the equally good folder) is highlighted.
    assert_eq!(hits[0].matches.branch, [4..8]);
    assert!(hits[0].matches.folder.is_empty() && hits[0].matches.repo.is_empty());
}

#[test]
fn scattered_letters_match_and_highlight_each_run() {
    let hits = search(&snapshot(), &[], "FxRL");
    assert_eq!(paths(&hits), ["/c/wt/rate-limit"]);
    assert_eq!(hits[0].matches.branch, [0..1, 2..3, 4..5, 9..10]);
}

#[test]
fn every_word_must_match_some_field() {
    let hits = search(&snapshot(), &[], "web auth");
    assert_eq!(paths(&hits), ["/c/wt/web-auth"]);
    let m = &hits[0].matches;
    assert_eq!(m.branch, [8..12]);
    assert_eq!(m.repo, [0..3]);
    assert!(search(&snapshot(), &[], "api ui").is_empty());
}

#[test]
fn detached_worktree_matches_by_folder() {
    let hits = search(&snapshot(), &[], "bisect");
    assert_eq!(paths(&hits), ["/c/wt/bisect"]);
    assert_eq!(hits[0].branch, None);
    assert_eq!(hits[0].matches.folder, [0..6]);
}

#[test]
fn repo_name_matches_all_its_worktrees() {
    let hits = search(&snapshot(), &[], "web");
    // A whole-name folder match outranks the same repo match.
    assert_eq!(paths(&hits), ["/c/web", "/c/wt/web-auth"]);
    assert_eq!(find(&hits, "/c/web").matches.folder, [0..3]);
    assert_eq!(find(&hits, "/c/wt/web-auth").matches.repo, [0..3]);
}

#[test]
fn word_start_and_contiguous_matches_rank_first() {
    let hits = search(&snapshot(), &[], "auth");
    assert_eq!(paths(&hits), ["/c/wt/auth-refresh", "/c/wt/web-auth"]);
    // A run at a word start beats letters scattered across words.
    let hits = search(&snapshot(), &[], "re");
    assert_eq!(hits[0].path, PathBuf::from("/c/wt/auth-refresh"));
    assert_eq!(hits[0].matches.branch, [13..15]);
}

#[test]
fn compact_scattered_match_beats_more_word_starts() {
    let mut s = snapshot();
    s.repos[0].worktrees[1].branch = Some("a-m-a-i-n".into());
    let hits = search(&s, &[], "mn");
    let rank = |branch| {
        hits.iter()
            .position(|h| h.branch.as_deref() == Some(branch))
            .unwrap()
    };
    assert!(rank("main") < rank("a-m-a-i-n"));
    assert_eq!(hits[0].matches.branch, [0..1, 3..4]);
}

#[test]
fn ties_go_to_recently_opened() {
    let recents: Vec<PathBuf> = vec!["/c/web".into()];
    let hits = search(&snapshot(), &recents, "main");
    assert_eq!(paths(&hits), ["/c/web", "/c/api"]);
}

#[test]
fn nothing_matches_is_empty() {
    assert!(search(&snapshot(), &[], "zzz").is_empty());
}

#[test]
fn ranges_are_byte_offsets_for_non_ascii_names() {
    let mut s = snapshot();
    s.repos[0].worktrees[1].branch = Some("fëature/ÜBER".into());
    let hits = search(&s, &[], "über");
    let branch = hits[0].branch.as_deref().unwrap();
    assert_eq!(&branch[hits[0].matches.branch[0].clone()], "ÜBER");
}

#[test]
fn hits_carry_presence_and_badges() {
    let mut s = snapshot();
    s.repos[0].worktrees[1].status = Some(Ok(WorktreeStatus {
        files: vec![],
        upstream: Upstream::None,
    }));
    s.repos[0].worktrees[2].presence = Presence::Missing;
    let hits = search(&s, &[], "");
    assert_eq!(find(&hits, "/c/wt/auth-refresh").badges, [Badge::LocalOnly]);
    let missing = find(&hits, "/c/wt/rate-limit");
    assert_eq!(missing.presence, Presence::Missing);
    assert_eq!(missing.badges, [Badge::Missing]);
}
