//! Headless core: no GPUI. The app is a thin shell over this.

mod badge;
mod cache;
mod details;
mod discovery;
mod env;
pub mod git;
mod launch;
mod menu;
mod process;
mod search;
mod settings;
mod snapshot;
mod targets;
mod tree;
mod watch;

pub use badge::Badge;
pub use cache::SnapshotCache;
pub use details::{Details, cd_command, relative_time, unix_now};
pub use discovery::{RepoRef, discover};
pub use env::{PathSource, ShellEnv, resolved_env};
pub use git::{
    Change, ChangedFile, Commit, GitError, Upstream, WorktreeStatus, read_worktree_status,
    recent_commits,
};
pub use launch::{
    Fallback, LaunchError, LaunchPlan, Outcome, Permission, Runner, execute, execute_with, plan,
};
pub use menu::{MenuGroup, MenuRow, StateLine, menu};
pub use search::{Hit, Matches, search};
pub use settings::{CustomCommand, MAX_RECENTS, Settings, SettingsFile, TerminalOpensIn};
pub use snapshot::{
    FetchRead, FetchReport, FetchResult, Presence, RepoRead, RepoResult, RepoSnapshot, ScanRead,
    ScanResult, Snapshot, SnapshotStore, StatusReads, StatusResult, Worktree, scan,
};
pub use targets::{
    Editor, NoTarget, OpenTarget, Role, TargetKind, Terminal, default_editor, default_target,
    default_terminal, detect, detect_in,
};
pub use tree::{
    DisplayPath, NavEffect, NavKey, Row, RowId, RowKind, Tree, home_dir, navigate, tilde,
};
pub use watch::{Changes, WatchError, Watcher};
