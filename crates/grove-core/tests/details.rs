mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use common::{Fixture, add_worktree, git, init_repo};
use grove_core::{RowId, Snapshot, SnapshotStore, WorktreeStatus, recent_commits};

fn read(f: &Fixture) -> Snapshot {
    let mut store = SnapshotStore::new(&f.code());
    store.refresh_statuses();
    store.snapshot().as_ref().clone()
}

fn status<'a>(s: &'a Snapshot, path: &Path) -> &'a WorktreeStatus {
    let wt = s
        .repos
        .iter()
        .flat_map(|r| &r.worktrees)
        .find(|w| w.path.canonicalize().ok() == path.canonicalize().ok())
        .unwrap_or_else(|| panic!("no worktree {} in {s:#?}", path.display()));
    wt.status.as_ref().unwrap().as_ref().unwrap()
}

fn commit_all(dir: &Path, msg: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", msg]);
}

#[test]
fn changed_files_carry_git_status_codes() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    for name in ["edit.txt", "gone.txt", "old name.txt"] {
        fs::write(api.join(name), name).unwrap();
    }
    commit_all(&api, "files");
    fs::write(api.join("edit.txt"), "changed").unwrap();
    fs::remove_file(api.join("gone.txt")).unwrap();
    fs::write(api.join("added.txt"), "a").unwrap();
    git(&api, &["add", "added.txt"]);
    git(&api, &["mv", "old name.txt", "new name.txt"]);
    fs::write(api.join("untracked file.txt"), "u").unwrap();

    let s = read(&f);
    let mut got: Vec<(String, String, Option<String>)> = status(&s, &api)
        .files
        .iter()
        .map(|c| (c.change.code().into(), c.path.clone(), c.from.clone()))
        .collect();
    got.sort();
    let want = [
        ("??", "untracked file.txt", None),
        ("A", "added.txt", None),
        ("D", "gone.txt", None),
        ("M", "edit.txt", None),
        ("R", "new name.txt", Some("old name.txt")),
    ]
    .map(|(c, p, f)| (c.to_string(), p.to_string(), f.map(str::to_string)));
    assert_eq!(got, want);
    assert_eq!(status(&s, &api).summary(), "5 changed files");
}

#[test]
fn clean_worktree_lists_no_files() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let s = read(&f);
    assert!(status(&s, &api).files.is_empty());
    assert_eq!(status(&s, &api).summary(), "clean");
}

#[test]
fn last_three_commits_newest_first_with_special_characters() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let messages = [
        "second",
        "fix: \"quotes\" & 'apostrophes' $HOME `cmd` | ; \\ émoji 🌳",
        "tabs\tand  spaces",
        "newest\n\nbody that is not the subject",
    ];
    for m in messages {
        git(&api, &["commit", "-q", "--allow-empty", "-m", m]);
    }
    let commits = recent_commits(&api).unwrap();
    let subjects: Vec<&str> = commits.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(subjects, ["newest", "tabs\tand  spaces", messages[1]]);
    assert!(
        commits
            .iter()
            .all(|c| c.short_sha.len() >= 7 && c.short_sha.chars().all(|ch| ch.is_ascii_hexdigit()))
    );
    let now = grove_core::unix_now();
    assert!(commits.iter().all(|c| (now - c.time).abs() < 600));
}

#[test]
fn linked_worktree_reads_its_own_commits() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let wt = f.code().join("api-x");
    add_worktree(&api, &wt, "feature/x");
    git(&wt, &["commit", "-q", "--allow-empty", "-m", "on x"]);
    assert_eq!(recent_commits(&wt).unwrap()[0].subject, "on x");
    assert_eq!(recent_commits(&api).unwrap()[0].subject, "init");
}

const FAKE_GPG: &str = r#"#!/bin/sh
case " $* " in
*" --verify "*) echo 'gpg: Good signature' >&2; echo '[GNUPG:] GOODSIG 0 T' >&2 ;;
*) cat >/dev/null; echo '[GNUPG:] SIG_CREATED D 1 8 00 0 X' >&2
   printf -- '-----BEGIN PGP SIGNATURE-----\n\nZmFrZQ==\n-----END PGP SIGNATURE-----\n' ;;
esac
"#;

#[test]
fn show_signature_config_does_not_break_parsing() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    // Stand-in gpg so the commit is "signed" and `log` has signature output to print.
    let gpg = f.tmp.path().join("fake-gpg");
    fs::write(&gpg, FAKE_GPG).unwrap();
    fs::set_permissions(&gpg, fs::Permissions::from_mode(0o755)).unwrap();
    git(&api, &["config", "gpg.program", gpg.to_str().unwrap()]);
    git(&api, &["config", "log.showSignature", "true"]);
    git(
        &api,
        &["commit", "-q", "-S", "--allow-empty", "-m", "signed"],
    );
    let commits = recent_commits(&api).unwrap();
    let subjects: Vec<&str> = commits.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(subjects, ["signed", "init"]);
    assert!(
        commits
            .iter()
            .all(|c| c.short_sha.chars().all(|ch| ch.is_ascii_hexdigit())),
        "{commits:?}"
    );
}

#[test]
fn repo_without_commits_has_none() {
    let f = Fixture::new();
    let api = f.code().join("api");
    fs::create_dir_all(&api).unwrap();
    git(&api, &["init", "-q"]);
    assert_eq!(recent_commits(&api).unwrap(), []);
}

#[test]
fn details_name_branch_and_worktree_kinds() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let wt = f.code().join("api-x");
    add_worktree(&api, &wt, "feature/x");
    git(&api, &["worktree", "add", "-q", "--detach", "../bisect"]);
    let s = f.scan();
    let repo = &s.repos[0];
    let kind = |id: RowId| {
        let d = repo.details(&id).unwrap();
        (d.title, d.kind)
    };
    let wt_id = |p: &Path| {
        RowId::Worktree(
            repo.worktrees
                .iter()
                .find(|w| w.path.ends_with(p))
                .unwrap()
                .path
                .clone(),
        )
    };
    assert_eq!(
        kind(RowId::Branch("feature/x".into())),
        ("feature/x".into(), "Branch · has worktree".into())
    );
    assert_eq!(
        kind(wt_id(Path::new("api-x"))),
        ("api-x".into(), "Worktree · linked worktree".into())
    );
    assert_eq!(
        kind(wt_id(Path::new("api"))),
        ("api".into(), "Worktree · main worktree".into())
    );
    assert_eq!(
        kind(wt_id(Path::new("bisect"))),
        ("bisect".into(), "Worktree · detached HEAD".into())
    );
    let branch = repo.details(&RowId::Branch("feature/x".into())).unwrap();
    assert!(branch.worktree.unwrap().ends_with("api-x"));
}
