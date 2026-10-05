//! Headless core: no GPUI. The app is a thin shell over this.

mod discovery;
mod env;
pub mod git;
mod launch;
mod process;
mod settings;
mod snapshot;
mod targets;
mod tree;

pub use discovery::{RepoRef, discover};
pub use env::{PathSource, ShellEnv, resolved_env};
pub use launch::{LaunchError, LaunchPlan, execute, plan};
pub use settings::{CustomCommand, Settings, SettingsFile, TerminalOpensIn};
pub use snapshot::{RepoSnapshot, Snapshot, SnapshotStore, Worktree, scan};
pub use targets::{Editor, NoEditor, OpenTarget, TargetKind, default_editor, detect, detect_in};
pub use tree::{
    DisplayPath, NavEffect, NavKey, Row, RowId, RowKind, Tree, home_dir, navigate, tilde,
};
