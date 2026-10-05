use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use grove_core::{Settings, SettingsFile, Snapshot, scan};
use tempfile::TempDir;

/// Runs git with an isolated config so tests work on any machine.
fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .args([
            "-c",
            "user.name=Grove Test",
            "-c",
            "user.email=test@grove.invalid",
        ])
        .args([
            "-c",
            "init.defaultBranch=main",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {args:?} in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn init_repo(path: &Path) -> PathBuf {
    fs::create_dir_all(path).unwrap();
    git(path, &["init", "-q"]);
    git(path, &["commit", "-q", "--allow-empty", "-m", "init"]);
    path.to_path_buf()
}

fn add_worktree(repo: &Path, at: &Path, branch: &str) {
    git(
        repo,
        &["worktree", "add", "-q", "-b", branch, at.to_str().unwrap()],
    );
}

struct Fixture {
    tmp: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join("code")).unwrap();
        Self { tmp }
    }
    fn code(&self) -> PathBuf {
        self.tmp.path().join("code")
    }
    fn outside(&self, name: &str) -> PathBuf {
        self.tmp.path().join("elsewhere").join(name)
    }
    fn scan(&self) -> Snapshot {
        scan(&self.code())
    }
}

/// (repo name, worktree count) pairs.
fn summary(s: &Snapshot) -> Vec<(String, usize)> {
    s.repos
        .iter()
        .map(|r| {
            assert!(r.error.is_none(), "{}: {:?}", r.name(), r.error);
            (r.name().to_string(), r.worktree_count())
        })
        .collect()
}

fn pairs(v: &[(&str, usize)]) -> Vec<(String, usize)> {
    v.iter().map(|(n, c)| (n.to_string(), *c)).collect()
}

#[test]
fn flat_repo() {
    let f = Fixture::new();
    init_repo(&f.code().join("api"));
    let s = f.scan();
    assert!(s.code_dir_exists);
    assert_eq!(summary(&s), pairs(&[("api", 1)]));
    let main = &s.repos[0].worktrees[0];
    assert!(main.is_main);
    assert_eq!(main.branch.as_deref(), Some("main"));
}

#[test]
fn nested_org_repo() {
    let f = Fixture::new();
    init_repo(&f.code().join("acme/web"));
    init_repo(&f.code().join("tools"));
    assert_eq!(summary(&f.scan()), pairs(&[("acme/web", 1), ("tools", 1)]));
}

#[test]
fn linked_worktrees_inside_and_outside_code_dir() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    add_worktree(&api, &f.code().join("api-feature"), "feature/login");
    add_worktree(&api, &f.outside("api-hotfix"), "hotfix");
    let s = f.scan();
    // The inside worktree is grouped under its repo, not listed as one; both are counted.
    assert_eq!(summary(&s), pairs(&[("api", 3)]));
    let branches: Vec<_> = s.repos[0]
        .worktrees
        .iter()
        .map(|w| w.branch.clone().unwrap())
        .collect();
    assert_eq!(branches, ["main", "feature/login", "hotfix"]);
}

#[test]
fn linked_worktree_in_nested_folder_is_not_a_repo() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    add_worktree(&api, &f.code().join("worktrees/api-x"), "x");
    assert_eq!(summary(&f.scan()), pairs(&[("api", 2)]));
}

#[test]
fn bare_repo_with_linked_worktrees() {
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
    add_worktree(&bare, &f.code().join("svc-a"), "a");
    add_worktree(&bare, &f.outside("svc-b"), "b");
    let s = f.scan();
    assert_eq!(summary(&s), pairs(&[("svc", 2)]));
    assert!(s.repos[0].repo.bare);
    assert!(s.repos[0].worktrees.iter().all(|w| !w.is_main));
}

#[test]
fn bare_repo_without_worktrees_is_listed() {
    let f = Fixture::new();
    git(&f.code(), &["init", "-q", "--bare", "plain.git"]);
    assert_eq!(summary(&f.scan()), pairs(&[("plain", 0)]));
}

#[test]
fn dot_bare_layout_named_after_parent() {
    let f = Fixture::new();
    let d = f.code().join("proj");
    fs::create_dir_all(&d).unwrap();
    let src = init_repo(&f.tmp.path().join("src"));
    git(
        &d,
        &["clone", "-q", "--bare", src.to_str().unwrap(), ".bare"],
    );
    fs::write(d.join(".git"), "gitdir: ./.bare\n").unwrap();
    git(&d, &["worktree", "add", "-q", "-b", "dev", "dev"]);
    assert_eq!(summary(&f.scan()), pairs(&[("proj", 1)]));
}

#[test]
fn empty_folder() {
    let f = Fixture::new();
    fs::create_dir(f.code().join("notes")).unwrap();
    let s = f.scan();
    assert!(s.code_dir_exists);
    assert!(s.repos.is_empty());
}

#[test]
fn missing_folder() {
    let f = Fixture::new();
    let s = scan(&f.tmp.path().join("gone"));
    assert!(!s.code_dir_exists);
    assert!(s.repos.is_empty());
}

#[test]
fn does_not_descend_into_repos() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    init_repo(&api.join("vendored"));
    init_repo(&api.join("node_modules/dep"));
    assert_eq!(summary(&f.scan()), pairs(&[("api", 1)]));
}

#[test]
fn ignores_repos_deeper_than_two_levels() {
    let f = Fixture::new();
    init_repo(&f.code().join("a/b/deep"));
    init_repo(&f.code().join("a/shallow"));
    assert_eq!(summary(&f.scan()), pairs(&[("a/shallow", 1)]));
}

#[test]
fn git_is_available_and_new_enough() {
    let info = grove_core::git::check().expect("git");
    assert!(info.version >= grove_core::git::MIN_VERSION);
}

#[test]
fn settings_round_trip() {
    let tmp = TempDir::new().unwrap();
    let file = SettingsFile::new(tmp.path().join("Grove/settings.json"));
    assert_eq!(file.load().unwrap(), Settings::default());

    let mut s = Settings {
        code_dir: Some(tmp.path().join("code")),
        last_selected_repo: Some("api".into()),
        ..Default::default()
    };
    s.collapsed.insert("api".into(), vec!["feature/".into()]);
    file.save(&s).unwrap();
    assert_eq!(file.load().unwrap(), s);
}

#[test]
fn settings_tolerate_missing_fields() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("settings.json");
    fs::write(&path, r#"{"code_dir":"/x"}"#).unwrap();
    let s = SettingsFile::new(&path).load().unwrap();
    assert_eq!(s.code_dir, Some(PathBuf::from("/x")));
    assert!(s.show_menu_bar_icon && s.launch_at_login && !s.background_fetch);
}
