use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

use crate::process;

const SHELL_TIMEOUT: Duration = Duration::from_secs(5);
const START: &str = "__GROVE_PATH_START__";
const END: &str = "__GROVE_PATH_END__";
/// Appended when missing so Homebrew/system tools resolve even if the shell lookup fails.
const FALLBACK_DIRS: &[&str] = &[
    "/opt/homebrew/bin",
    "/opt/homebrew/sbin",
    "/usr/local/bin",
    "/usr/bin",
    "/bin",
    "/usr/sbin",
    "/sbin",
];

/// Where the subprocess PATH came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathSource {
    /// Extracted from the user's login shell.
    LoginShell,
    /// Login shell failed or timed out; inherited PATH plus fallback dirs.
    Fallback,
}

/// Environment every Grove subprocess runs with.
#[derive(Debug, Clone)]
pub struct ShellEnv {
    /// Resolved colon-separated executable search path.
    pub path: String,
    /// How `path` was resolved.
    pub source: PathSource,
    /// User shell used for login-shell commands.
    pub shell: String,
}

impl ShellEnv {
    /// A command for `program` with the resolved PATH applied.
    pub fn command(&self, program: impl AsRef<Path>) -> Command {
        let mut cmd = Command::new(program.as_ref());
        cmd.env("PATH", &self.path);
        cmd
    }

    /// First executable named `name` on the resolved PATH.
    pub fn which(&self, name: &str) -> Option<PathBuf> {
        std::env::split_paths(&OsString::from(&self.path))
            .map(|dir| dir.join(name))
            .find(|p| is_executable(p))
    }
}

/// Resolved once per process: the user's login-shell PATH, so tools installed via Homebrew
/// etc. are found even when Grove is launched from Finder with a minimal environment.
pub fn resolved_env() -> &'static ShellEnv {
    static ENV: OnceLock<ShellEnv> = OnceLock::new();
    ENV.get_or_init(resolve)
}

fn resolve() -> ShellEnv {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/zsh".into());
    let inherited = std::env::var("PATH").unwrap_or_default();
    let (base, source) = login_shell_path(&shell)
        .map_or((inherited, PathSource::Fallback), |path| {
            (path, PathSource::LoginShell)
        });
    ShellEnv {
        path: with_fallbacks(&base),
        source,
        shell,
    }
}

fn login_shell_path(shell: &str) -> Option<String> {
    let mut cmd = Command::new(shell);
    cmd.args(["-l", "-c"])
        .arg(format!("printf '{START}%s{END}' \"$PATH\""));
    let out = process::run(cmd, SHELL_TIMEOUT).ok()?;
    if !out.status.success() {
        return None;
    }
    let start = out.stdout.rfind(START)? + START.len();
    let len = out.stdout[start..].find(END)?;
    let path = out.stdout[start..start + len].trim();
    (!path.is_empty()).then(|| path.to_string())
}

fn with_fallbacks(path: &str) -> String {
    let mut dirs: Vec<&str> = path.split(':').filter(|d| !d.is_empty()).collect();
    for dir in FALLBACK_DIRS {
        if !dirs.contains(dir) {
            dirs.push(dir);
        }
    }
    dirs.join(":")
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}
