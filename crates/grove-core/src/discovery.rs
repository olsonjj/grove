use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::git;

const MAX_DEPTH: usize = 2;

/// A repo found in the code directory.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoRef {
    /// Path relative to the code directory, e.g. `api` or `org/api` (`.git` suffix dropped).
    pub name: String,
    /// Main worktree for normal repos; the git dir itself for bare repos.
    pub path: PathBuf,
    /// Canonical common git dir; identifies the repo.
    pub git_dir: PathBuf,
    pub bare: bool,
}

/// Repos up to 2 levels below `code_dir`, sorted by name. A folder with a `.git` dir or a
/// bare repo is a repo and isn't descended into. A folder with a `.git` file is a linked
/// worktree: never a repo itself, but its repo is included if that lives in `code_dir`
/// (covers the `repo/.bare` + `.git` file layout).
pub fn discover(code_dir: &Path) -> Vec<RepoRef> {
    let root = code_dir
        .canonicalize()
        .unwrap_or_else(|_| code_dir.to_path_buf());
    let mut repos = Vec::new();
    let mut seen = HashSet::new();
    walk(&root, &root, 1, &mut repos, &mut seen);
    repos.sort_by(|a, b| a.name.cmp(&b.name));
    repos
}

fn walk(
    root: &Path,
    dir: &Path,
    depth: usize,
    repos: &mut Vec<RepoRef>,
    seen: &mut HashSet<PathBuf>,
) {
    let Ok(read) = fs::read_dir(dir) else { return };
    let mut children: Vec<PathBuf> = read
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    children.sort();

    for child in children {
        let dot_git = child.join(".git");
        let found = if dot_git.is_dir() {
            Some(repo_ref(
                root,
                &child,
                dot_git.canonicalize().unwrap_or(dot_git),
                false,
            ))
        } else if dot_git.is_file() {
            linked_worktree_repo(root, &child)
        } else if is_bare_repo(&child) {
            let git_dir = child.canonicalize().unwrap_or_else(|_| child.clone());
            Some(repo_ref(root, &child, git_dir, true))
        } else {
            if depth < MAX_DEPTH {
                walk(root, &child, depth + 1, repos, seen);
            }
            continue;
        };
        if let Some(repo) = found
            && seen.insert(repo.git_dir.clone())
        {
            repos.push(repo);
        }
    }
}

fn linked_worktree_repo(root: &Path, worktree: &Path) -> Option<RepoRef> {
    let git_dir = git::common_dir(worktree).ok()?;
    if !git_dir.starts_with(root) {
        return None;
    }
    if git_dir.file_name().is_some_and(|n| n == ".git") {
        let main = git_dir.parent()?.to_path_buf();
        Some(repo_ref(root, &main, git_dir, false))
    } else {
        Some(repo_ref(root, &git_dir, git_dir.clone(), true))
    }
}

fn repo_ref(root: &Path, path: &Path, git_dir: PathBuf, bare: bool) -> RepoRef {
    let named = if path
        .file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with('.'))
    {
        path.parent().unwrap_or(path) // `repo/.bare` → `repo`
    } else {
        path
    };
    let rel = named.strip_prefix(root).unwrap_or(named).to_string_lossy();
    let name = rel.strip_suffix(".git").unwrap_or(&rel).to_string();
    RepoRef {
        name,
        path: path.to_path_buf(),
        git_dir,
        bare,
    }
}

/// Same shape test git itself uses for a git dir: HEAD file plus objects/ and refs/.
fn is_bare_repo(dir: &Path) -> bool {
    dir.join("HEAD").is_file() && dir.join("objects").is_dir() && dir.join("refs").is_dir()
}
