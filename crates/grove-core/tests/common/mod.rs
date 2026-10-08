#![allow(dead_code, reason = "each test binary uses a subset")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use grove_core::{Snapshot, scan};
use tempfile::TempDir;

/// Runs git with an isolated config so tests work on any machine.
pub fn git(dir: &Path, args: &[&str]) {
    git_env(dir, args, &[]);
}

/// An empty commit in `dir` committed at Unix time `secs`.
pub fn commit_at(dir: &Path, secs: i64) {
    let date = format!("@{secs} +0000");
    git_env(
        dir,
        &["commit", "-q", "--allow-empty", "-m", &format!("at {secs}")],
        &[("GIT_COMMITTER_DATE", &date)],
    );
}

fn git_env(dir: &Path, args: &[&str], env: &[(&str, &str)]) {
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
        .envs(env.iter().copied())
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {args:?} in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
}

pub fn init_repo(path: &Path) -> PathBuf {
    fs::create_dir_all(path).unwrap();
    git(path, &["init", "-q"]);
    git(path, &["commit", "-q", "--allow-empty", "-m", "init"]);
    path.to_path_buf()
}

pub fn add_worktree(repo: &Path, at: &Path, branch: &str) {
    git(
        repo,
        &["worktree", "add", "-q", "-b", branch, at.to_str().unwrap()],
    );
}

pub struct Fixture {
    pub tmp: TempDir,
}

impl Fixture {
    pub fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join("code")).unwrap();
        Self { tmp }
    }
    pub fn code(&self) -> PathBuf {
        self.tmp.path().join("code")
    }
    pub fn outside(&self, name: &str) -> PathBuf {
        self.tmp.path().join("elsewhere").join(name)
    }
    pub fn scan(&self) -> Snapshot {
        scan(&self.code())
    }
}
