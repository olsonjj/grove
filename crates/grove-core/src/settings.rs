use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::snapshot::Snapshot;

pub const MAX_RECENTS: usize = 10;

/// Worktree path → when Grove last opened it, Unix seconds.
pub type LastOpened = BTreeMap<PathBuf, i64>;

/// Everything Grove persists in `settings.json`; missing fields take their defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub code_dir: Option<PathBuf>,
    /// Editor or terminal id (`Editor::id`, `Terminal::id`); editors may also name a custom
    /// command (`CustomCommand::id`). `None` = first detected.
    pub default_editor: Option<String>,
    pub default_terminal: Option<String>,
    /// `None` = per-terminal default (see `Terminal::default_opens_in`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal_opens_in: Option<TerminalOpensIn>,
    pub custom_commands: Vec<CustomCommand>,
    pub show_menu_bar_icon: bool,
    pub background_fetch: bool,
    pub launch_at_login: bool,
    /// The Grove.app whose login item macOS's record is the user's choice for (they can
    /// remove or re-add it in System Settings). `None` until one registers.
    pub login_item_path: Option<PathBuf>,
    /// Opened worktrees, most recent first, at most `MAX_RECENTS`.
    pub recents: Vec<PathBuf>,
    /// Worktree path → when Grove last opened it, Unix seconds. Uncapped; pruned on rescan.
    pub last_opened: LastOpened,
    /// Repo name → collapsed prefix folders.
    pub collapsed: BTreeMap<String, Vec<String>>,
    /// `RepoRef::path`s. Kept while a repo is absent, so it comes back pinned.
    pub pinned_repos: Vec<PathBuf>,
    /// Order of the sidebar's unpinned repos and the popover's groups.
    pub repo_sort: SortMode,
    /// Order of every repo's tree and of worktrees within a popover group.
    pub tree_sort: SortMode,
    pub last_selected_repo: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            code_dir: None,
            default_editor: None,
            default_terminal: None,
            terminal_opens_in: None,
            custom_commands: Vec::new(),
            show_menu_bar_icon: true,
            background_fetch: false,
            launch_at_login: true,
            login_item_path: None,
            recents: Vec::new(),
            last_opened: BTreeMap::new(),
            collapsed: BTreeMap::new(),
            pinned_repos: Vec::new(),
            repo_sort: SortMode::Recent,
            tree_sort: SortMode::Recent,
            last_selected_repo: None,
        }
    }
}

/// What the settings window edits. The rest of `Settings` (recents, last opened, collapse
/// state, pins, sort modes, the login item's owner) keeps changing underneath it while it's
/// open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preferences {
    pub code_dir: Option<PathBuf>,
    pub default_editor: Option<String>,
    pub default_terminal: Option<String>,
    pub terminal_opens_in: Option<TerminalOpensIn>,
    pub custom_commands: Vec<CustomCommand>,
    pub show_menu_bar_icon: bool,
    pub background_fetch: bool,
    pub launch_at_login: bool,
}

/// macOS's record of Grove's login item (`SMAppService`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginItemStatus {
    NotRegistered,
    Enabled,
    /// Registered, but the user has to allow it in System Settings → Login Items.
    RequiresApproval,
    /// Can't be read or changed, e.g. when not running from Grove.app.
    Unavailable,
}

impl LoginItemStatus {
    pub fn is_registered(self) -> bool {
        matches!(self, Self::Enabled | Self::RequiresApproval)
    }
}

/// The running Grove.app, as the login item logic sees it.
#[derive(Debug, Clone, Copy)]
pub struct LoginItemHost<'a> {
    pub bundle: &'a Path,
    /// In /Applications or ~/Applications (see `is_installed_location`).
    pub installed: bool,
    /// The recorded owner (`login_item_path`) is still on disk.
    pub owner_exists: bool,
}

/// Under /Applications or `home`/Applications: the only places a Grove.app registers or
/// adopts the login item on its own. Dev builds, Desktop or Downloads copies and
/// translocated apps only do so on an explicit Save.
pub fn is_installed_location(bundle: &Path, home: Option<&Path>) -> bool {
    bundle.starts_with("/Applications")
        || home.is_some_and(|h| bundle.starts_with(h.join("Applications")))
}

impl Settings {
    /// At launch. Returns `Some(true)` to register.
    ///
    /// Once a copy owns the login item (`login_item_path`), macOS's record is the user's
    /// choice and the setting follows it; other copies leave it alone. With no owner (or one
    /// moved or deleted since) the setting wins (on by default) and a registration found is
    /// adopted, but only by an installed copy.
    pub fn sync_login_item(
        &mut self,
        host: LoginItemHost<'_>,
        status: LoginItemStatus,
    ) -> Option<bool> {
        if status == LoginItemStatus::Unavailable {
            return None;
        }
        if self
            .login_item_path
            .as_deref()
            .is_some_and(|owner| owner != host.bundle && !host.owner_exists)
        {
            self.login_item_path = None;
        }
        match &self.login_item_path {
            Some(owner) if owner == host.bundle => {
                self.launch_at_login = status.is_registered();
                None
            }
            Some(_) => None,
            None if !host.installed => None,
            None if status.is_registered() => {
                self.login_item_path = Some(host.bundle.to_path_buf());
                self.launch_at_login = true;
                None
            }
            None => self.launch_at_login.then_some(true),
        }
    }

    /// Settings Save: the user's explicit choice, so the copy at `bundle` takes over the
    /// login item. Returns the change that makes macOS match `launch_at_login`: `Some(true)`
    /// to register, `Some(false)` to unregister.
    pub fn apply_login_item(&mut self, bundle: &Path, status: LoginItemStatus) -> Option<bool> {
        if status == LoginItemStatus::Unavailable {
            return None;
        }
        self.login_item_path = Some(bundle.to_path_buf());
        (status.is_registered() != self.launch_at_login).then_some(self.launch_at_login)
    }

    /// After registering from `bundle`; `status` is what macOS reports now.
    pub fn login_item_registered(&mut self, bundle: &Path, status: LoginItemStatus) {
        if status.is_registered() && self.login_item_path.is_none() {
            self.login_item_path = Some(bundle.to_path_buf());
        }
    }

    pub fn preferences(&self) -> Preferences {
        Preferences {
            code_dir: self.code_dir.clone(),
            default_editor: self.default_editor.clone(),
            default_terminal: self.default_terminal.clone(),
            terminal_opens_in: self.terminal_opens_in,
            custom_commands: self.custom_commands.clone(),
            show_menu_bar_icon: self.show_menu_bar_icon,
            background_fetch: self.background_fetch,
            launch_at_login: self.launch_at_login,
        }
    }

    /// Settings Save. Leaves everything the window doesn't edit as it is now.
    pub fn set_preferences(&mut self, p: Preferences) {
        let Preferences {
            code_dir,
            default_editor,
            default_terminal,
            terminal_opens_in,
            custom_commands,
            show_menu_bar_icon,
            background_fetch,
            launch_at_login,
        } = p;
        self.code_dir = code_dir;
        self.default_editor = default_editor;
        self.default_terminal = default_terminal;
        self.terminal_opens_in = terminal_opens_in;
        self.custom_commands = custom_commands;
        self.show_menu_bar_icon = show_menu_bar_icon;
        self.background_fetch = background_fetch;
        self.launch_at_login = launch_at_login;
    }

    pub fn collapsed(&self, repo: &str) -> &[String] {
        self.collapsed.get(repo).map_or(&[], Vec::as_slice)
    }

    /// Returns whether anything changed. Repos with nothing collapsed are dropped.
    pub fn set_collapsed(&mut self, repo: &str, prefix: &str, collapsed: bool) -> bool {
        let list = self.collapsed.entry(repo.to_string()).or_default();
        let had = list.iter().any(|p| p == prefix);
        if collapsed && !had {
            list.push(prefix.to_string());
            list.sort();
        } else if !collapsed && had {
            list.retain(|p| p != prefix);
        }
        if list.is_empty() {
            self.collapsed.remove(repo);
        }
        had != collapsed
    }

    pub fn is_pinned(&self, repo: &Path) -> bool {
        self.pinned_repos.iter().any(|p| p == repo)
    }

    /// Pins or unpins the repo at `repo`. Returns whether anything changed.
    pub fn toggle_pin(&mut self, repo: &Path) -> bool {
        if self.is_pinned(repo) {
            self.pinned_repos.retain(|p| p != repo);
        } else {
            self.pinned_repos.push(repo.to_path_buf());
        }
        true
    }

    /// Returns whether anything changed.
    pub fn set_repo_sort(&mut self, mode: SortMode) -> bool {
        std::mem::replace(&mut self.repo_sort, mode) != mode
    }

    /// Returns whether anything changed.
    pub fn set_tree_sort(&mut self, mode: SortMode) -> bool {
        std::mem::replace(&mut self.tree_sort, mode) != mode
    }

    /// Moves `worktree` to the front of recents and stamps it opened `at` (Unix seconds).
    /// Returns whether anything changed.
    pub fn record_open(&mut self, worktree: &Path, at: i64) -> bool {
        let stamped = self.last_opened.insert(worktree.to_path_buf(), at) != Some(at);
        if self.recents.first().is_some_and(|p| p == worktree) {
            return stamped;
        }
        self.recents.retain(|p| p != worktree);
        self.recents.insert(0, worktree.to_path_buf());
        self.recents.truncate(MAX_RECENTS);
        true
    }

    /// After Grove removed `worktree`: forgets when it was opened. Returns whether anything
    /// changed.
    pub fn forget_worktree(&mut self, worktree: &Path) -> bool {
        self.last_opened.remove(worktree).is_some()
    }

    /// After a full rescan: forgets when worktrees no longer in `snapshot` were opened.
    /// Skipped while the code dir is missing or any repo's worktrees couldn't be listed, so
    /// an unmounted volume or a failed read doesn't wipe the history. Returns whether
    /// anything changed.
    pub fn prune_last_opened(&mut self, snapshot: &Snapshot) -> bool {
        if !snapshot.code_dir_exists || snapshot.repos.iter().any(|r| r.error.is_some()) {
            return false;
        }
        let known: HashSet<&Path> = snapshot
            .repos
            .iter()
            .flat_map(|r| &r.worktrees)
            .map(|w| w.path.as_path())
            .collect();
        let before = self.last_opened.len();
        self.last_opened.retain(|p, _| known.contains(p.as_path()));
        self.last_opened.len() != before
    }
}

/// How a list is ordered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortMode {
    /// Newest activity first; ties and unknown activity by name, unknown last.
    #[default]
    Recent,
    /// Alphabetical.
    Name,
}

impl SortMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Recent => "Recent",
            Self::Name => "Name",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalOpensIn {
    Tab,
    Window,
}

/// A user-defined shell command offered alongside the editors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomCommand {
    pub label: String,
    /// `{path}` is replaced with the shell-quoted worktree path.
    pub template: String,
    #[serde(default = "yes")]
    pub run_in_terminal: bool,
}

fn yes() -> bool {
    true
}

const CUSTOM_ID_PREFIX: &str = "custom:";

impl CustomCommand {
    /// Key stored as `default_editor` when this is the default.
    pub fn id(&self) -> String {
        custom_id(&self.label)
    }

    /// Why it can't be saved alongside `others`, if it can't. Expects trimmed fields.
    pub fn problem(&self, others: &[CustomCommand]) -> Option<&'static str> {
        if self.label.is_empty() {
            Some("Give the command a name.")
        } else if others.iter().any(|c| c.label == self.label) {
            Some("Another command already has this name.")
        } else if !self.template.contains("{path}") {
            Some("The command needs {path} where the worktree path goes.")
        } else {
            None
        }
    }
}

pub(crate) fn custom_id(label: &str) -> String {
    format!("{CUSTOM_ID_PREFIX}{label}")
}

/// The label in a custom command's id.
pub(crate) fn custom_label(id: &str) -> Option<&str> {
    id.strip_prefix(CUSTOM_ID_PREFIX)
}

/// Settings JSON on disk.
#[derive(Debug, Clone)]
pub struct SettingsFile {
    path: PathBuf,
}

impl SettingsFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// `$GROVE_SETTINGS_PATH` if set, else `~/Library/Application Support/Grove/settings.json`.
    pub fn default_location() -> Self {
        if let Some(p) = std::env::var_os("GROVE_SETTINGS_PATH") {
            return Self::new(p);
        }
        let home = std::env::var_os("HOME").unwrap_or_default();
        Self::new(Path::new(&home).join("Library/Application Support/Grove/settings.json"))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Missing file → defaults. Unreadable or corrupt → error.
    pub fn load(&self) -> io::Result<Settings> {
        match fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(e) => Err(e),
        }
    }

    /// Atomic write (temp file + rename).
    pub fn save(&self, settings: &Settings) -> io::Result<()> {
        let json = serde_json::to_vec_pretty(settings).map_err(io::Error::other)?;
        write_atomic(&self.path, &json)
    }
}

/// Writes via a temp file + rename so readers never see a partial file.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}
