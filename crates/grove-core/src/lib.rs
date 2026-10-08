//! Headless core: no GPUI. The app is a thin shell over this.

mod badge;
mod cache;
mod details;
mod discovery;
mod env;
pub mod git;
mod launch;
mod menu;
mod order;
mod process;
mod removal;
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
    recent_commits, remove_worktree,
};
pub use launch::{
    CommandError, Fallback, LaunchError, LaunchPlan, Outcome, Permission, Runner, command_line,
    execute, execute_with, plan, plan_command,
};
pub use menu::{MenuGroup, MenuRow, StateLine, menu};
pub use order::{RepoOrder, activity_time, repo_activity, repo_order};
pub use removal::{RemovalPlan, read_removal_plan};
pub use search::{Hit, Matches, search};
pub use settings::{
    CustomCommand, LastOpened, LoginItemHost, LoginItemStatus, MAX_RECENTS, Preferences, Settings,
    SettingsFile, SortMode, TerminalOpensIn, is_installed_location,
};
pub use snapshot::{
    FetchRead, FetchReport, FetchResult, Presence, RepoRead, RepoResult, RepoSnapshot, ScanRead,
    ScanResult, Snapshot, SnapshotStore, StatusReads, StatusResult, Worktree, scan,
};
pub use targets::{
    Editor, NoTarget, OpenTarget, Opener, Role, TargetKind, Terminal, default_editor,
    default_opener, default_target, default_terminal, detect, detect_in, openers,
};
pub use tree::{
    DisplayPath, NavEffect, NavKey, Row, RowId, RowKind, Tree, home_dir, navigate, tilde,
};
pub use watch::{Changes, WatchError, Watcher};
