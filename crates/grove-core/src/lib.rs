//! Headless core: no GPUI. The app is a thin shell over this.

mod discovery;
mod env;
pub mod git;
mod process;
mod settings;
mod snapshot;

pub use discovery::{RepoRef, discover};
pub use env::{PathSource, ShellEnv, resolved_env};
pub use settings::{CustomCommand, Settings, SettingsFile, TerminalOpensIn};
pub use snapshot::{RepoSnapshot, Snapshot, SnapshotStore, Worktree, scan};
