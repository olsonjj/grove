use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

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
    /// Most recent first, at most 10.
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
            recents: Vec::new(),
            collapsed: BTreeMap::new(),
            last_selected_repo: None,
        }
    }
}

impl Settings {
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
