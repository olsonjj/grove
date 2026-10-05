//! Headless core: no GPUI. The app is a thin shell over this.

mod badge;
mod discovery;
mod env;
pub mod git;
mod launch;
mod process;
mod settings;
mod snapshot;
mod targets;
mod tree;

pub use badge::Badge;
pub use discovery::{RepoRef, discover};
pub use env::{PathSource, ShellEnv, resolved_env};
pub use git::{Upstream, WorktreeStatus, read_worktree_status};
pub use launch::{
    Fallback, LaunchError, LaunchPlan, Outcome, Permission, Runner, execute, execute_with, plan,
};
pub use settings::{CustomCommand, Settings, SettingsFile, TerminalOpensIn};
pub use snapshot::{
    Presence, RepoSnapshot, Snapshot, SnapshotStore, Worktree, read_statuses, scan,
};
pub use targets::{
    Editor, NoTarget, OpenTarget, Role, TargetKind, Terminal, default_editor, default_target,
    default_terminal, detect, detect_in,
};
pub use tree::{
    DisplayPath, NavEffect, NavKey, Row, RowId, RowKind, Tree, home_dir, navigate, tilde,
};
