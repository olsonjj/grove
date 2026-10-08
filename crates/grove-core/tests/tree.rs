mod common;

use std::path::Path;

use common::{Fixture, add_worktree, git, init_repo};
use grove_core::{
    Badge, LastOpened, NavKey, Presence, RowId, RowKind, Snapshot, SortMode, Tree, navigate,
};

/// One line per visible row, indented by depth. Worktrees show their folder name.
fn shape(tree: &Tree) -> Vec<String> {
    tree.rows
        .iter()
        .map(|r| {
            let text = match &r.kind {
                RowKind::Folder {
                    prefix,
                    count,
                    collapsed,
                } => format!("{} {prefix} {count}", if *collapsed { "▸" } else { "▾" }),
                RowKind::Branch { label, .. } => format!("branch {label}"),
                RowKind::Worktree {
                    display, badges, ..
                } => {
                    let mut s = format!("wt {}", display.name);
                    for b in badges
                        .iter()
                        .filter(|b| matches!(b, Badge::MainWorktree | Badge::Detached(_)))
                    {
                        s.push_str(&format!(" [{}]", b.label()));
                    }
                    s
                }
                RowKind::DetachedSection => "-- detached --".into(),
            };
            format!("{}{text}", "  ".repeat(r.depth))
        })
        .collect()
}

fn tree_of(s: &Snapshot, repo: &str, collapsed: &[&str]) -> Tree {
    tree_sorted(s, repo, collapsed, SortMode::Name, &LastOpened::new())
}

fn tree_sorted(
    s: &Snapshot,
    repo: &str,
    collapsed: &[&str],
    sort: SortMode,
    opened: &LastOpened,
) -> Tree {
    let repo = s.repo(repo).expect("repo");
    assert!(repo.error.is_none(), "{:?}", repo.error);
    let collapsed: Vec<String> = collapsed.iter().map(|c| c.to_string()).collect();
    repo.tree(&collapsed, sort, opened)
}

/// Worktrees (by folder name) opened at the given Unix times. Statuses are unread, so
/// these are their only activity.
fn opened(s: &Snapshot, at: &[(&str, i64)]) -> LastOpened {
    at.iter()
        .map(|(folder, t)| {
            let wt = s
                .repos
                .iter()
                .flat_map(|r| &r.worktrees)
                .find(|w| w.path.file_name().is_some_and(|n| n == *folder))
                .unwrap_or_else(|| panic!("no worktree {folder}"));
            (wt.path.clone(), *t)
        })
        .collect()
}

/// Branch names in row order.
fn branches(tree: &Tree) -> Vec<&str> {
    tree.rows
        .iter()
        .filter_map(|r| match &r.kind {
            RowKind::Branch { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect()
}

fn rev_parse(dir: &Path) -> String {
    let out = std::process::Command::new("git")
        .current_dir(dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// api: main + feature/b, feature/a, fix/c, plain; branches `idle` and `feature/idle`
/// exist with no worktree.
fn prefixed_repo(f: &Fixture) {
    let api = init_repo(&f.code().join("api"));
    let wt = f.code().join("wt");
    add_worktree(&api, &wt.join("api-b"), "feature/b");
    add_worktree(&api, &wt.join("api-a"), "feature/a");
    add_worktree(&api, &wt.join("api-c"), "fix/c");
    add_worktree(&api, &wt.join("api-plain"), "plain");
    git(&api, &["branch", "idle"]);
    git(&api, &["branch", "feature/idle"]);
}

#[test]
fn prefixes_group_branches_by_first_segment() {
    let f = Fixture::new();
    prefixed_repo(&f);
    let tree = tree_of(&f.scan(), "api", &[]);
    assert_eq!(
        shape(&tree),
        [
            "branch main",
            "  wt api [main worktree]",
            "▾ feature/ 2",
            "  branch a",
            "    wt api-a",
            "  branch b",
            "    wt api-b",
            "▾ fix/ 1",
            "  branch c",
            "    wt api-c",
            "branch plain",
            "  wt api-plain",
        ]
    );
    assert_eq!((tree.worktrees, tree.branches, tree.detached), (5, 5, 0));
}

#[test]
fn branches_without_a_worktree_never_appear() {
    let f = Fixture::new();
    prefixed_repo(&f);
    let tree = tree_of(&f.scan(), "api", &[]);
    for row in &tree.rows {
        if let RowKind::Branch { name, .. } = &row.kind {
            assert!(name != "idle" && name != "feature/idle", "{name} shown");
        }
    }
    // A folder with only idle branches is omitted entirely.
    let f = Fixture::new();
    let web = init_repo(&f.code().join("web"));
    git(&web, &["branch", "chore/deps"]);
    assert_eq!(
        shape(&tree_of(&f.scan(), "web", &[])),
        ["branch main", "  wt web [main worktree]"]
    );
}

#[test]
fn collapse_hides_children_and_keeps_count() {
    let f = Fixture::new();
    prefixed_repo(&f);
    let tree = tree_of(&f.scan(), "api", &["feature/"]);
    assert_eq!(
        shape(&tree),
        [
            "branch main",
            "  wt api [main worktree]",
            "▸ feature/ 2",
            "▾ fix/ 1",
            "  branch c",
            "    wt api-c",
            "branch plain",
            "  wt api-plain",
        ]
    );
}

#[test]
fn detached_worktrees_in_bottom_section_with_short_sha() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    add_worktree(&api, &f.code().join("api-x"), "x");
    git(
        &api,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            f.code().join("api-bisect").to_str().unwrap(),
        ],
    );
    let sha = rev_parse(&api)[..7].to_string();
    let tree = tree_of(&f.scan(), "api", &[]);
    assert_eq!(
        shape(&tree),
        [
            "branch main".to_string(),
            "  wt api [main worktree]".into(),
            "branch x".into(),
            "  wt api-x".into(),
            "-- detached --".into(),
            format!("wt api-bisect [detached @ {sha}]"),
        ]
    );
    assert_eq!(tree.detached, 1);
}

#[test]
fn main_only_repo() {
    let f = Fixture::new();
    init_repo(&f.code().join("solo"));
    let tree = tree_of(&f.scan(), "solo", &[]);
    assert_eq!(shape(&tree), ["branch main", "  wt solo [main worktree]"]);
}

#[test]
fn main_worktree_branch_sorts_first() {
    let f = Fixture::new();
    let repo = init_repo(&f.code().join("zed"));
    git(&repo, &["checkout", "-q", "-b", "zzz"]);
    add_worktree(&repo, &f.code().join("zed-b"), "Beta");
    add_worktree(&repo, &f.code().join("zed-a"), "alpha");
    add_worktree(&repo, &f.code().join("zed-m"), "main-ish");
    add_worktree(&repo, &f.code().join("zed-f"), "feat/x");
    let tree = tree_of(&f.scan(), "zed", &[]);
    let branches: Vec<_> = tree
        .rows
        .iter()
        .filter_map(|r| match &r.kind {
            RowKind::Branch { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(branches, ["zzz", "alpha", "Beta", "feat/x", "main-ish"]);
    // Folders sort among plain branches, not before them.
    assert!(matches!(&tree.rows[6].kind, RowKind::Folder { prefix, .. } if prefix == "feat/"));
}

#[test]
fn bare_repo_shows_only_linked_worktrees() {
    let f = Fixture::new();
    let src = init_repo(&f.tmp.path().join("src"));
    let bare = f.code().join("svc.git");
    git(
        f.tmp.path(),
        &[
            "clone",
            "-q",
            "--bare",
            src.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
    );
    add_worktree(&bare, &f.code().join("svc-a"), "feature/a");
    let tree = tree_of(&f.scan(), "svc", &[]);
    assert_eq!(shape(&tree), ["▾ feature/ 1", "  branch a", "    wt svc-a"]);
}

#[test]
fn worktree_path_display_splits_parent_and_name() {
    let f = Fixture::new();
    prefixed_repo(&f);
    let tree = tree_of(&f.scan(), "api", &[]);
    let RowKind::Worktree { display, path, .. } = &tree.rows[4].kind else {
        panic!("row 4 is a worktree");
    };
    assert_eq!(display.name, "api-a");
    assert!(display.parent.ends_with("/wt/"), "{}", display.parent);
    assert_eq!(display.full(), path.display().to_string());
}

#[test]
fn keyboard_navigation_over_real_tree() {
    let f = Fixture::new();
    prefixed_repo(&f);
    let s = f.scan();
    let feature = RowId::Folder("feature/".into());

    let tree = tree_of(&s, "api", &[]);
    // No selection → first row.
    assert_eq!(
        navigate(&tree, None, NavKey::Down).select,
        Some(RowId::Branch("main".into()))
    );
    // Down from the main worktree lands on the folder; Up goes back.
    let main_wt = tree.rows[1].id.clone();
    assert_eq!(
        navigate(&tree, Some(&main_wt), NavKey::Down).select,
        Some(feature.clone())
    );
    assert_eq!(
        navigate(&tree, Some(&feature), NavKey::Up).select,
        Some(main_wt)
    );
    // ← on an open folder collapses it; ← on a child moves to its parent.
    assert_eq!(
        navigate(&tree, Some(&feature), NavKey::Left).toggle,
        Some(("feature/".into(), true))
    );
    let a = RowId::Branch("feature/a".into());
    assert_eq!(
        navigate(&tree, Some(&a), NavKey::Left).select,
        Some(feature.clone())
    );
    // → on a branch moves into its worktree.
    assert_eq!(
        navigate(&tree, Some(&a), NavKey::Right).select,
        Some(tree.rows[4].id.clone())
    );

    let collapsed = tree_of(&s, "api", &["feature/"]);
    assert_eq!(
        navigate(&collapsed, Some(&feature), NavKey::Right).toggle,
        Some(("feature/".into(), false))
    );
    // Down skips hidden children.
    assert_eq!(
        navigate(&collapsed, Some(&feature), NavKey::Down).select,
        Some(RowId::Folder("fix/".into()))
    );
    // Bottom edge: no movement.
    let last = collapsed.rows.last().unwrap().id.clone();
    assert_eq!(navigate(&collapsed, Some(&last), NavKey::Down).select, None);
}

#[test]
fn rows_resolve_to_the_worktree_they_open() {
    let f = Fixture::new();
    prefixed_repo(&f);
    let code = f.code().canonicalize().unwrap();
    let wt = code.join("wt");
    std::fs::remove_dir_all(wt.join("api-c")).unwrap();
    let tree = tree_of(&f.scan(), "api", &[]);
    let api = code.join("api");

    let branch = |n: &str| tree.worktree_for(&RowId::Branch(n.into()));
    assert_eq!(branch("main"), Some((api.as_path(), Presence::Present)));
    assert_eq!(
        branch("feature/a"),
        Some((wt.join("api-a").as_path(), Presence::Present))
    );
    assert_eq!(
        tree.worktree_for(&RowId::Worktree(wt.join("api-b"))),
        Some((wt.join("api-b").as_path(), Presence::Present))
    );
    assert_eq!(
        branch("fix/c"),
        Some((wt.join("api-c").as_path(), Presence::Missing))
    );
    assert_eq!(tree.worktree_for(&RowId::Folder("feature/".into())), None);
}

#[test]
fn name_mode_ignores_activity() {
    let f = Fixture::new();
    prefixed_repo(&f);
    let s = f.scan();
    let at = opened(&s, &[("api-plain", 500), ("api-c", 400)]);
    assert_eq!(
        shape(&tree_sorted(&s, "api", &[], SortMode::Name, &at)),
        shape(&tree_of(&s, "api", &[]))
    );
}

#[test]
fn recent_orders_folders_and_branches_by_activity() {
    let f = Fixture::new();
    prefixed_repo(&f);
    let s = f.scan();
    // fix/c never active: unknown sorts after known.
    let at = opened(
        &s,
        &[
            ("api", 100),
            ("api-plain", 500),
            ("api-a", 300),
            ("api-b", 400),
        ],
    );
    assert_eq!(
        shape(&tree_sorted(&s, "api", &[], SortMode::Recent, &at)),
        [
            "branch main",
            "  wt api [main worktree]",
            "branch plain",
            "  wt api-plain",
            "▾ feature/ 2",
            "  branch b",
            "    wt api-b",
            "  branch a",
            "    wt api-a",
            "▾ fix/ 1",
            "  branch c",
            "    wt api-c",
        ]
    );

    // A folder's activity is its newest branch.
    let at = opened(&s, &[("api-plain", 500), ("api-a", 300), ("api-c", 600)]);
    assert_eq!(
        shape(&tree_sorted(&s, "api", &[], SortMode::Recent, &at))[2..],
        [
            "▾ fix/ 1",
            "  branch c",
            "    wt api-c",
            "branch plain",
            "  wt api-plain",
            "▾ feature/ 2",
            "  branch a",
            "    wt api-a",
            "  branch b",
            "    wt api-b",
        ]
    );
}

#[test]
fn recent_nested_branch_lifts_its_folder() {
    let f = Fixture::new();
    prefixed_repo(&f);
    let api = f.code().join("api");
    add_worktree(&api, &f.code().join("wt/api-auth"), "feature/auth/x");
    let s = f.scan();
    let at = opened(&s, &[("api-auth", 500), ("api-plain", 300)]);
    assert_eq!(
        shape(&tree_sorted(&s, "api", &[], SortMode::Recent, &at)),
        [
            "branch main",
            "  wt api [main worktree]",
            "▾ feature/ 3",
            "  branch auth/x",
            "    wt api-auth",
            "  branch a",
            "    wt api-a",
            "  branch b",
            "    wt api-b",
            "branch plain",
            "  wt api-plain",
            "▾ fix/ 1",
            "  branch c",
            "    wt api-c",
        ]
    );
}

#[test]
fn recent_ties_and_unknown_go_by_name() {
    let f = Fixture::new();
    let repo = init_repo(&f.code().join("zed"));
    for b in ["omega", "beta", "Alpha", "gamma", "Delta"] {
        add_worktree(&repo, &f.code().join(format!("zed-{b}")), b);
    }
    let s = f.scan();
    let at = opened(
        &s,
        &[("zed-beta", 200), ("zed-Alpha", 200), ("zed-omega", 100)],
    );
    // main is never active here, yet stays first.
    assert_eq!(
        branches(&tree_sorted(&s, "zed", &[], SortMode::Recent, &at)),
        ["main", "Alpha", "beta", "omega", "Delta", "gamma"]
    );
}

#[test]
fn recent_collapsed_folder_keeps_its_position() {
    let f = Fixture::new();
    prefixed_repo(&f);
    let s = f.scan();
    let at = opened(&s, &[("api-plain", 300), ("api-b", 400), ("api-c", 200)]);
    assert_eq!(
        shape(&tree_sorted(
            &s,
            "api",
            &["feature/"],
            SortMode::Recent,
            &at
        )),
        [
            "branch main",
            "  wt api [main worktree]",
            "▸ feature/ 2",
            "branch plain",
            "  wt api-plain",
            "▾ fix/ 1",
            "  branch c",
            "    wt api-c",
        ]
    );
}

#[test]
fn recent_orders_detached_by_activity_and_keeps_the_section_last() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    add_worktree(&api, &f.code().join("api-x"), "x");
    for name in ["api-d1", "api-d2", "api-d3"] {
        let at = f.code().join(name);
        git(
            &api,
            &["worktree", "add", "-q", "--detach", at.to_str().unwrap()],
        );
    }
    let s = f.scan();
    let at = opened(&s, &[("api-x", 100), ("api-d2", 900), ("api-d3", 500)]);
    let rows = shape(&tree_sorted(&s, "api", &[], SortMode::Recent, &at));
    let names: Vec<_> = rows.iter().map(|r| r.split(" [").next().unwrap()).collect();
    assert_eq!(
        names,
        [
            "branch main",
            "  wt api",
            "branch x",
            "  wt api-x",
            "-- detached --",
            "wt api-d2",
            "wt api-d3",
            "wt api-d1",
        ]
    );
}

#[test]
fn recent_keeps_a_detached_main_worktree_first_in_its_section() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let d1 = f.code().join("api-d1");
    git(
        &api,
        &["worktree", "add", "-q", "--detach", d1.to_str().unwrap()],
    );
    git(&api, &["checkout", "-q", "--detach"]);
    let s = f.scan();
    let at = opened(&s, &[("api-d1", 900)]);
    let rows = shape(&tree_sorted(&s, "api", &[], SortMode::Recent, &at));
    let names: Vec<_> = rows.iter().map(|r| r.split(" [").next().unwrap()).collect();
    assert_eq!(names, ["-- detached --", "wt api", "wt api-d1"]);
}

#[test]
fn recent_keeps_main_worktree_first_under_its_branch() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    add_worktree(&api, &f.code().join("api-x"), "x");
    // A second checkout of main, newer than everything.
    let again = f.code().join("api-again");
    git(
        &api,
        &[
            "worktree",
            "add",
            "-q",
            "-f",
            again.to_str().unwrap(),
            "main",
        ],
    );
    let s = f.scan();
    let at = opened(&s, &[("api-again", 900), ("api-x", 500)]);
    assert_eq!(
        shape(&tree_sorted(&s, "api", &[], SortMode::Recent, &at)),
        [
            "branch main",
            "  wt api [main worktree]",
            "  wt api-again",
            "branch x",
            "  wt api-x",
        ]
    );
}
