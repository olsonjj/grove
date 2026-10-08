use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const MAX_RECENTS: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub code_dir: Option<PathBuf>,
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
    /// Repo name → collapsed prefix folders.
    pub collapsed: BTreeMap<String, Vec<String>>,
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
            collapsed: BTreeMap::new(),
            last_selected_repo: None,
        }
    }
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

    /// Moves `worktree` to the front of recents. Returns whether anything changed.
    pub fn record_open(&mut self, worktree: &Path) -> bool {
        if self.recents.first().is_some_and(|p| p == worktree) {
            return false;
        }
        self.recents.retain(|p| p != worktree);
        self.recents.insert(0, worktree.to_path_buf());
        self.recents.truncate(MAX_RECENTS);
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalOpensIn {
    Tab,
    Window,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomCommand {
    pub label: String,
    /// `{path}` is replaced with the shell-quoted worktree path.
    pub template: String,
    pub run_in_terminal: bool,
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
