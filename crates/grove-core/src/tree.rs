//! The checked-out tree: what the main window shows for one repo.
//!
//! Branches containing `/` are grouped by their first segment (`feature/auth/x` → folder
//! `feature/`, row `auth/x`), matching the design prototype. Top level order: the main
//! worktree's branch, then prefix folders, then plain branches, each alphabetical.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::badge::Badge;
use crate::snapshot::{Presence, RepoSnapshot, Worktree};

/// Stable identity of a row, for selection.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RowId {
    /// Prefix including the trailing slash, e.g. `feature/`.
    Folder(String),
    /// Full branch name.
    Branch(String),
    Worktree(PathBuf),
    DetachedSection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowKind {
    Folder {
        prefix: String,
        /// Checked-out branches inside.
        count: usize,
        collapsed: bool,
    },
    Branch {
        name: String,
        /// Name shown in the row: the part after the folder prefix.
        label: String,
    },
    Worktree {
        path: PathBuf,
        display: DisplayPath,
        is_main: bool,
        locked: bool,
        presence: Presence,
        badges: Vec<Badge>,
    },
    DetachedSection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: RowId,
    pub depth: usize,
    pub parent: Option<RowId>,
    pub kind: RowKind,
}

/// Visible rows of a repo's checked-out tree, top to bottom.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Tree {
    pub rows: Vec<Row>,
    pub worktrees: usize,
    pub branches: usize,
    pub detached: usize,
}

impl Tree {
    pub fn index_of(&self, id: &RowId) -> Option<usize> {
        self.rows.iter().position(|r| &r.id == id)
    }

    /// The worktree a row opens: itself, or a branch's first worktree (main first).
    /// Returns its path and whether its folder is there to open.
    pub fn worktree_for(&self, id: &RowId) -> Option<(&Path, Presence)> {
        let ix = self.index_of(id)?;
        let row = match &self.rows[ix].kind {
            RowKind::Worktree { .. } => &self.rows[ix],
            RowKind::Branch { .. } => self
                .rows
                .get(ix + 1)
                .filter(|r| r.parent.as_ref() == Some(id))?,
            _ => return None,
        };
        match &row.kind {
            RowKind::Worktree { path, presence, .. } => Some((path.as_path(), *presence)),
            _ => None,
        }
    }

    /// Rows a user can select (everything but section headers).
    pub fn selectable(&self, ix: usize) -> bool {
        self.rows
            .get(ix)
            .is_some_and(|r| r.kind != RowKind::DetachedSection)
    }
}

/// A worktree path split for display: `parent` dimmed, `name` bright. `~` for home.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayPath {
    /// Everything up to and including the last `/`, e.g. `~/code/wt/`.
    pub parent: String,
    pub name: String,
}

impl DisplayPath {
    pub fn new(path: &Path, home: Option<&Path>) -> Self {
        let full = tilde(path, home);
        match full.rfind('/') {
            Some(i) if i + 1 < full.len() => Self {
                parent: full[..=i].to_string(),
                name: full[i + 1..].to_string(),
            },
            _ => Self {
                parent: String::new(),
                name: full,
            },
        }
    }

    pub fn full(&self) -> String {
        format!("{}{}", self.parent, self.name)
    }
}

/// `/Users/me/code` → `~/code` when `home` is `/Users/me`.
pub fn tilde(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

impl RepoSnapshot {
    /// The checked-out tree; folders whose prefix is in `collapsed` hide their children.
    pub fn tree(&self, collapsed: &[String]) -> Tree {
        build(&self.worktrees, collapsed, home_dir().as_deref())
    }
}

fn build(worktrees: &[Worktree], collapsed: &[String], home: Option<&Path>) -> Tree {
    let mut branches: BTreeMap<&str, Vec<&Worktree>> = BTreeMap::new();
    let mut detached = Vec::new();
    for wt in worktrees {
        match &wt.branch {
            Some(b) => branches.entry(b).or_default().push(wt),
            None => detached.push(wt),
        }
    }
    let main_branch = worktrees
        .iter()
        .find(|w| w.is_main)
        .and_then(|w| w.branch.as_deref());

    let mut tree = Tree {
        worktrees: worktrees.len(),
        branches: branches.len(),
        detached: detached.len(),
        rows: Vec::new(),
    };

    let mut folders: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut plain = Vec::new();
    for &name in branches.keys() {
        if Some(name) == main_branch {
            continue;
        }
        match name.split_once('/') {
            Some((first, rest)) if !first.is_empty() && !rest.is_empty() => {
                folders.entry(&name[..=first.len()]).or_default().push(name)
            }
            _ => plain.push(name),
        }
    }

    let push_branch = |tree: &mut Tree, name: &str, label: &str, depth, parent: Option<RowId>| {
        let id = RowId::Branch(name.to_string());
        tree.rows.push(Row {
            id: id.clone(),
            depth,
            parent,
            kind: RowKind::Branch {
                name: name.to_string(),
                label: label.to_string(),
            },
        });
        let mut wts = branches[name].clone();
        wts.sort_by(|a, b| b.is_main.cmp(&a.is_main).then(a.path.cmp(&b.path)));
        for wt in wts {
            tree.rows
                .push(worktree_row(wt, depth + 1, Some(id.clone()), home));
        }
    };

    if let Some(main) = main_branch {
        push_branch(&mut tree, main, main, 0, None);
    }

    // Folders and plain branches share one alphabetical list.
    let mut entries: Vec<(&str, Option<Vec<&str>>)> = folders
        .into_iter()
        .map(|(prefix, names)| (prefix, Some(names)))
        .chain(plain.into_iter().map(|name| (name, None)))
        .collect();
    entries.sort_by(|a, b| by_name(a.0, b.0));
    for (key, names) in entries {
        let Some(mut names) = names else {
            push_branch(&mut tree, key, key, 0, None);
            continue;
        };
        let is_collapsed = collapsed.iter().any(|c| c == key);
        let id = RowId::Folder(key.to_string());
        tree.rows.push(Row {
            id: id.clone(),
            depth: 0,
            parent: None,
            kind: RowKind::Folder {
                prefix: key.to_string(),
                count: names.len(),
                collapsed: is_collapsed,
            },
        });
        if is_collapsed {
            continue;
        }
        names.sort_by(|a, b| by_name(a, b));
        for name in names {
            push_branch(&mut tree, name, &name[key.len()..], 1, Some(id.clone()));
        }
    }

    if !detached.is_empty() {
        detached.sort_by(|a, b| b.is_main.cmp(&a.is_main).then(a.path.cmp(&b.path)));
        tree.rows.push(Row {
            id: RowId::DetachedSection,
            depth: 0,
            parent: None,
            kind: RowKind::DetachedSection,
        });
        for wt in detached {
            tree.rows.push(worktree_row(wt, 0, None, home));
        }
    }
    tree
}

fn worktree_row(wt: &Worktree, depth: usize, parent: Option<RowId>, home: Option<&Path>) -> Row {
    Row {
        id: RowId::Worktree(wt.path.clone()),
        depth,
        parent,
        kind: RowKind::Worktree {
            path: wt.path.clone(),
            display: DisplayPath::new(&wt.path, home),
            is_main: wt.is_main,
            locked: wt.locked,
            presence: wt.presence,
            badges: wt.badges(),
        },
    }
}

/// Case-insensitive, ties broken by exact bytes so order is total.
fn by_name(a: &str, b: &str) -> Ordering {
    a.to_lowercase().cmp(&b.to_lowercase()).then(a.cmp(b))
}

/// Keyboard input the tree understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavKey {
    Up,
    Down,
    Left,
    Right,
}

/// What a key press does: new selection and/or a folder to collapse or expand.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NavEffect {
    pub select: Option<RowId>,
    /// `(prefix, collapse)`.
    pub toggle: Option<(String, bool)>,
}

/// ↑/↓ move across selectable rows; ← collapses an open folder or moves to the parent;
/// → expands a closed folder or moves into an open one. No selection → first row.
pub fn navigate(tree: &Tree, selected: Option<&RowId>, key: NavKey) -> NavEffect {
    let first = (0..tree.rows.len()).find(|&i| tree.selectable(i));
    let Some(cur) = selected.and_then(|id| tree.index_of(id)) else {
        return NavEffect {
            select: first.map(|i| tree.rows[i].id.clone()),
            toggle: None,
        };
    };
    let row = &tree.rows[cur];
    let select = |i: usize| NavEffect {
        select: Some(tree.rows[i].id.clone()),
        toggle: None,
    };
    match key {
        NavKey::Up => (0..cur)
            .rev()
            .find(|&i| tree.selectable(i))
            .map(select)
            .unwrap_or_default(),
        NavKey::Down => (cur + 1..tree.rows.len())
            .find(|&i| tree.selectable(i))
            .map(select)
            .unwrap_or_default(),
        NavKey::Left => match &row.kind {
            RowKind::Folder {
                prefix,
                collapsed: false,
                ..
            } => NavEffect {
                select: None,
                toggle: Some((prefix.clone(), true)),
            },
            _ => row
                .parent
                .as_ref()
                .and_then(|p| tree.index_of(p))
                .map(select)
                .unwrap_or_default(),
        },
        NavKey::Right => match &row.kind {
            RowKind::Folder {
                prefix,
                collapsed: true,
                ..
            } => NavEffect {
                select: None,
                toggle: Some((prefix.clone(), false)),
            },
            RowKind::Folder { .. } | RowKind::Branch { .. } => tree
                .rows
                .get(cur + 1)
                .filter(|r| r.parent.as_ref() == Some(&row.id))
                .map(|_| select(cur + 1))
                .unwrap_or_default(),
            _ => NavEffect::default(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_path_splits_and_tildes() {
        let home = Path::new("/Users/me");
        let d = DisplayPath::new(Path::new("/Users/me/code/wt/api-x"), Some(home));
        assert_eq!(
            (d.parent.as_str(), d.name.as_str()),
            ("~/code/wt/", "api-x")
        );
        let d = DisplayPath::new(Path::new("/Volumes/ext/api"), Some(home));
        assert_eq!(
            (d.parent.as_str(), d.name.as_str()),
            ("/Volumes/ext/", "api")
        );
        assert_eq!(DisplayPath::new(home, Some(home)).full(), "~");
        assert_eq!(
            tilde(Path::new("/Users/meow/x"), Some(home)),
            "/Users/meow/x"
        );
    }
}
