//! Launcher: pure planning (`plan`) and execution (`execute`). Never dedupes or focuses an
//! existing window; every open asks for a new window or tab.

use std::ffi::OsString;
use std::fmt::{self, Write as _};
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::env::ShellEnv;
use crate::settings::{CustomCommand, Settings, TerminalOpensIn};
use crate::targets::{NoTarget, OpenTarget, TargetKind, Terminal, default_terminal};

const OPEN: &str = "/usr/bin/open";
const OSASCRIPT: &str = "/usr/bin/osascript";
const SH: &str = "/bin/sh";
/// For terminals started with a program: runs `$1` in the user's login shell, then stays in
/// an interactive one, as if it had been typed.
const IN_LOGIN_SHELL: &str = r#"s="${SHELL:-/bin/zsh}"; "$s" -lc "$1"; exec "$s" -l"#;
/// Editor CLIs hand off to the app and exit; one still running after this is left running.
const EXIT_WAIT: Duration = Duration::from_secs(5);
/// osascript blocks while macOS shows the Automation prompt; give the user time to answer.
const OSASCRIPT_WAIT: Duration = Duration::from_secs(120);
/// After exit, how long to wait for stderr EOF (a daemonized child may hold the pipe).
const STDERR_GRACE: Duration = Duration::from_millis(200);
/// Maximum stderr retained while a launcher runs. Diagnostics use only the tail.
const STDERR_LIMIT: usize = 64 * 1024;

/// What opening a worktree runs, from `plan` or `plan_command`; `execute` runs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchPlan {
    /// Run `program` with `args` in `cwd`, resolved PATH applied. No shell involved.
    Spawn {
        /// Executable path.
        program: PathBuf,
        /// Arguments passed without shell interpretation.
        args: Vec<OsString>,
        /// Child working directory.
        cwd: PathBuf,
    },
    /// Run `script` with osascript; if it fails (e.g. Automation denied), run `fallback`.
    AppleScript {
        /// AppleScript source passed to `osascript`.
        script: String,
        /// Script working directory.
        cwd: PathBuf,
        /// Plan run if the script fails.
        fallback: Box<LaunchPlan>,
    },
}

/// How to open `worktree` in `target`.
///
/// Editors: the CLI with its new-window flag, else `open -a <App> <path>`.
/// Terminals: a new tab or window (per `settings.terminal_opens_in`, where the terminal
/// supports both) whose working directory is `worktree`.
pub fn plan(target: &OpenTarget, worktree: &Path, settings: &Settings) -> LaunchPlan {
    match target.kind {
        TargetKind::Editor(editor) => {
            let path = OsString::from(worktree);
            let (program, args) = match &target.cli {
                Some(cli) => {
                    let mut args: Vec<OsString> =
                        editor.new_window_args().iter().map(Into::into).collect();
                    args.push(path);
                    (cli.clone(), args)
                }
                None => (
                    PathBuf::from(OPEN),
                    vec!["-a".into(), app_arg(target), path],
                ),
            };
            spawn(program, args, worktree)
        }
        TargetKind::Terminal(terminal) => {
            let opens_in = settings
                .terminal_opens_in
                .unwrap_or(terminal.default_opens_in());
            terminal_plan(terminal, target, worktree, opens_in, None)
        }
    }
}

/// How to run `cmd` on `worktree`, `{path}` replaced with the shell-quoted path.
///
/// Run in terminal: a new tab or window of the default terminal (as `plan` opens it) at
/// `worktree`, running the command in the user's shell. Otherwise `/bin/sh -c` in the
/// background in `worktree`, resolved PATH applied.
///
/// # Errors
///
/// Returns [`CommandError`] when no compatible terminal exists or a path containing control
/// characters would have to be typed into a terminal.
pub fn plan_command(
    cmd: &CustomCommand,
    worktree: &Path,
    settings: &Settings,
    targets: &[OpenTarget],
) -> Result<LaunchPlan, CommandError> {
    let line = cmd.template.replace("{path}", &shell_quote(worktree));
    if !cmd.run_in_terminal {
        return Ok(spawn(SH.into(), vec!["-c".into(), line.into()], worktree));
    }
    // Some terminals get the command typed in; refused for all, to behave the same.
    if has_control(worktree) {
        return Err(CommandError::ControlChars(worktree.to_path_buf()));
    }
    let target = default_terminal(settings, targets)?;
    let TargetKind::Terminal(terminal) = target.kind else {
        unreachable!("default_terminal returns terminals");
    };
    if !terminal.runs_commands() {
        return Err(NoTarget::CantRunCommands(terminal).into());
    }
    let opens_in = settings
        .terminal_opens_in
        .unwrap_or(terminal.default_opens_in());
    Ok(terminal_plan(
        terminal,
        target,
        worktree,
        opens_in,
        Some(&line),
    ))
}

/// Why a custom command can't run.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CommandError {
    /// No usable application target is configured.
    #[error(transparent)]
    Target(#[from] NoTarget),
    /// Run in terminal, but the path has control characters, which typed into a shell
    /// would act as keystrokes.
    #[error(
        "Can't run a command in a terminal at {}: the path has control characters",
        .0.to_string_lossy().escape_debug()
    )]
    ControlChars(PathBuf),
}

impl CommandError {
    /// Fixable in settings.
    pub fn in_settings(&self) -> bool {
        matches!(self, CommandError::Target(_))
    }
}

/// What an editor runs, as settings shows it (`{path}` for the worktree): the CLI by name
/// (even when it's the one inside the app), else `open -a`. `None` for terminals.
pub fn command_line(target: &OpenTarget) -> Option<String> {
    let TargetKind::Editor(editor) = target.kind else {
        return None;
    };
    let program = match &target.cli {
        Some(_) => std::iter::once(editor.cli_name())
            .chain(editor.new_window_args().iter().copied())
            .collect::<Vec<_>>()
            .join(" "),
        None => format!("open -a {}", target.name()),
    };
    Some(format!("{program} {{path}}"))
}

/// `command`, if given, runs in the new tab or window (not for Warp, see `runs_commands`);
/// the `open -a` fallbacks only open the folder.
fn terminal_plan(
    terminal: Terminal,
    target: &OpenTarget,
    worktree: &Path,
    opens_in: TerminalOpensIn,
    command: Option<&str>,
) -> LaunchPlan {
    let tab = opens_in == TerminalOpensIn::Tab;
    let path = OsString::from(worktree);
    let program_args = || -> Vec<OsString> {
        command.map_or_else(Vec::new, |c| {
            vec![
                SH.into(),
                "-c".into(),
                IN_LOGIN_SHELL.into(),
                "sh".into(),
                c.into(),
            ]
        })
    };
    // LaunchServices opens a folder in Terminal / iTerm2 as a new window at that folder.
    let open_a = || {
        spawn(
            OPEN.into(),
            vec!["-a".into(), app_arg(target), path.clone()],
            worktree,
        )
    };
    match terminal {
        // Typed into the shell, a control character would act as a keystroke (^U, Enter…).
        Terminal::Terminal | Terminal::ITerm2 if has_control(worktree) => open_a(),
        Terminal::Terminal | Terminal::ITerm2 => {
            let mut typed = format!("cd {}", shell_quote(worktree));
            if let Some(c) = command {
                typed = format!("{typed} && {c}");
            }
            let cd = applescript_string(&typed);
            let script = match (terminal, tab) {
                (Terminal::Terminal, _) => terminal_app_script(&cd, tab),
                _ => iterm_script(&cd, tab),
            };
            applescript(script, worktree, open_a())
        }
        Terminal::Warp => {
            debug_assert!(command.is_none(), "Warp can't run commands");
            let action = if tab { "new_tab" } else { "new_window" };
            let url = format!("warp://action/{action}?path={}", percent_encode(worktree));
            spawn(OPEN.into(), vec![url.into()], worktree)
        }
        Terminal::Ghostty => {
            let mut wd = OsString::from("--working-directory=");
            wd.push(&path);
            // `open -n` starts a new Ghostty process; Ghostty's documented way to pass
            // CLI config on macOS. It always gives a window.
            let mut args = vec!["-na".into(), app_arg(target), "--args".into(), wd];
            if command.is_some() {
                args.push("-e".into());
                args.extend(program_args());
            }
            let window = spawn(OPEN.into(), args, worktree);
            if tab {
                // Tabs need Ghostty's AppleScript dictionary (1.3+); older versions fail
                // to compile the script (`Fallback::Unsupported`) and get the window.
                let dir = applescript_string(&worktree.to_string_lossy());
                let input = command.map_or_else(String::new, |c| {
                    let input = applescript_string(&format!("{c}\n"));
                    format!("\tset initial input of cfg to {input}\n")
                });
                applescript(ghostty_tab_script(&dir, &input), worktree, window)
            } else {
                window
            }
        }
        // Window only: tabs need `kitty @ launch`, which requires remote control to be
        // enabled in kitty.conf. `--single-instance` adds the window to a running kitty.
        Terminal::Kitty => {
            let mut kitty_args: Vec<OsString> = vec![
                "--single-instance".into(),
                "--directory".into(),
                path.clone(),
            ];
            kitty_args.extend(program_args());
            match (&target.app, &target.cli) {
                (None, Some(cli)) => spawn(cli.clone(), kitty_args, worktree),
                _ => {
                    let mut args: Vec<OsString> = vec!["-na".into(), app_arg(target)];
                    args.push("--args".into());
                    args.extend(kitty_args);
                    spawn(OPEN.into(), args, worktree)
                }
            }
        }
    }
}

fn spawn(program: PathBuf, args: Vec<OsString>, cwd: &Path) -> LaunchPlan {
    LaunchPlan::Spawn {
        program,
        args,
        cwd: cwd.to_path_buf(),
    }
}

fn applescript(script: String, cwd: &Path, fallback: LaunchPlan) -> LaunchPlan {
    LaunchPlan::AppleScript {
        script,
        cwd: cwd.to_path_buf(),
        fallback: Box::new(fallback),
    }
}

/// The bundle path if found, else the app name for LaunchServices to resolve.
fn app_arg(target: &OpenTarget) -> OsString {
    target
        .app
        .as_ref()
        .map_or_else(|| OsString::from(target.name()), OsString::from)
}

/// Terminal.app has no "new tab" command: ⌘T via System Events (needs Accessibility too),
/// then `do script` in the new tab. Errors rather than sending ⌘T to another app or typing
/// into the user's current tab. Not running → use the window it opens on launch.
fn terminal_app_script(cd: &str, tab: bool) -> String {
    let running = if tab {
        TERMINAL_TAB
    } else {
        "\t\tdo script {cd}\n"
    };
    TERMINAL_SCRIPT
        .replace("{running}", running)
        .replace("{cd}", cd)
}

/// Same shape as Terminal: a cold launch reuses iTerm2's launch window.
fn iterm_script(cd: &str, tab: bool) -> String {
    let running = if tab {
        ITERM_TAB
    } else {
        "\t\tset s to current session of (create window with default profile)\n"
    };
    ITERM_SCRIPT
        .replace("{running}", running)
        .replace("{cd}", cd)
}

/// Waits ~5 s for a cold-launched app's first window.
const TERMINAL_SCRIPT: &str = r#"set wasRunning to application id "com.apple.Terminal" is running
tell application id "com.apple.Terminal"
	activate
	if not wasRunning then
		repeat 100 times
			if (count of windows) > 0 then exit repeat
			delay 0.05
		end repeat
	end if
	if (count of windows) is 0 then
		do script {cd}
	else if not wasRunning then
		do script {cd} in front window
	else
{running}	end if
end tell"#;

const TERMINAL_TAB: &str = r#"		repeat 40 times
			if frontmost then exit repeat
			delay 0.05
		end repeat
		if not frontmost then error "Terminal didn't come to the front"
		set tabCount to count of tabs of front window
		tell application "System Events" to keystroke "t" using command down
		repeat 40 times
			if (count of tabs of front window) > tabCount then exit repeat
			delay 0.05
		end repeat
		if (count of tabs of front window) is not greater than tabCount then error "Terminal didn't open a new tab"
		do script {cd} in selected tab of front window
"#;

const ITERM_SCRIPT: &str = r#"set wasRunning to application id "com.googlecode.iterm2" is running
tell application id "com.googlecode.iterm2"
	activate
	if not wasRunning then
		repeat 100 times
			if (count of windows) > 0 then exit repeat
			delay 0.05
		end repeat
	end if
	if (count of windows) is 0 or current window is missing value then
		set s to current session of (create window with default profile)
	else if not wasRunning then
		set s to current session of current window
	else
{running}	end if
	tell s to write text {cd}
end tell"#;

const ITERM_TAB: &str = r"		tell current window to set t to (create tab with default profile)
		set s to current session of t
";

/// Error number a script raises to hand over to its fallback quietly.
const NOT_RUNNING: &str = "(9001)";

/// Not running → the `open -na` fallback, so a cold launch makes one window, not two.
/// `input` is empty or a whole `set initial input` line. One pass, so neither argument's
/// text is scanned for placeholders.
fn ghostty_tab_script(dir: &str, input: &str) -> String {
    format!(
        r#"if not (application id "com.mitchellh.ghostty" is running) then error "Ghostty isn't running" number 9001
tell application id "com.mitchellh.ghostty"
	activate
	set cfg to new surface configuration
	set initial working directory of cfg to {dir}
{input}	if (count of windows) is 0 then
		new window with configuration cfg
	else
		new tab in front window with configuration cfg
	end if
end tell"#
    )
}

fn has_control(path: &Path) -> bool {
    path.to_string_lossy().chars().any(char::is_control)
}

/// POSIX single-quoted shell word: `it's` → `'it'\''s'`.
pub(crate) fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', r"'\''"))
}

/// AppleScript string literal, quotes included.
fn applescript_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str(r"\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str(r"\n"),
            '\r' => out.push_str(r"\r"),
            '\t' => out.push_str(r"\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Percent-encodes everything but unreserved characters and `/`.
fn percent_encode(path: &Path) -> String {
    let mut out = String::new();
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// Why a plan didn't open anything. Errors are strings so the type stays `Clone + Eq`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LaunchError {
    /// The planned working directory no longer exists.
    #[error("Folder not found: {}", .0.display())]
    MissingFolder(PathBuf),
    /// The program could not be spawned.
    #[error("Couldn't run {program}: {error}")]
    Spawn {
        /// Display name of the executable.
        program: String,
        /// Operating-system spawn error.
        error: String,
    },
    /// Exited non-zero; `stderr` is its last line, maybe empty.
    #[error(fmt = fmt_exit)]
    Exit {
        /// Display name of the executable.
        program: String,
        /// Process exit status.
        status: String,
        /// Last line written to standard error.
        stderr: String,
    },
}

fn fmt_exit(program: &str, status: &str, stderr: &str, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    if stderr.is_empty() {
        write!(f, "{program} failed ({status})")
    } else {
        write!(f, "{program} failed: {stderr}")
    }
}

/// The macOS privacy pane that blocked an AppleScript plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    /// Permission to send Apple events to another application.
    Automation,
    /// Permission to synthesize input through System Events.
    Accessibility,
}

impl Permission {
    /// Returns the matching System Settings privacy pane name.
    pub fn pane(self) -> &'static str {
        match self {
            Permission::Automation => "Automation",
            Permission::Accessibility => "Accessibility",
        }
    }
}

/// Why an AppleScript plan fell back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fallback {
    /// macOS denied a required privacy permission.
    Denied(Permission),
    /// The script didn't compile: the app has no (or an older) scripting dictionary.
    Unsupported(String),
    /// The app wasn't running; the fallback is the normal way to cold-launch it.
    NotRunning,
    /// Any other osascript failure (its message).
    Failed(String),
}

/// How a plan that didn't fail went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The requested plan started successfully.
    Opened,
    /// The AppleScript failed and the fallback plan (a new window) ran instead.
    FellBack(Fallback),
    /// osascript was still running at the deadline, most likely blocked on the Automation
    /// prompt. Left running, so a late Allow still opens the tab; no fallback ran.
    TimedOut,
}

/// How plans are executed; tests swap the osascript binary and shorten its wait.
#[derive(Debug, Clone)]
pub struct Runner {
    /// `osascript` executable, replaceable for tests.
    pub osascript: PathBuf,
    /// Deadline for an AppleScript permission prompt.
    pub osascript_wait: Duration,
}

impl Default for Runner {
    fn default() -> Self {
        Self {
            osascript: OSASCRIPT.into(),
            osascript_wait: OSASCRIPT_WAIT,
        }
    }
}

/// One AppleScript plan at a time: two quick opens must not both see the same "new" tab.
static APPLESCRIPT: Mutex<()> = Mutex::new(());

/// Runs `plan` with `env`. Blocking (seconds for a CLI, up to two minutes for osascript
/// awaiting a permission prompt); run on a dedicated thread.
///
/// # Errors
///
/// Returns [`LaunchError`] when the folder is missing, a process cannot start, or a plan exits
/// unsuccessfully.
pub fn execute(plan: &LaunchPlan, env: &ShellEnv) -> Result<Outcome, LaunchError> {
    execute_with(plan, env, &Runner::default())
}

/// [`execute`] with a custom `runner`.
///
/// # Errors
///
/// Returns [`LaunchError`] under the same conditions as [`execute`].
pub fn execute_with(
    plan: &LaunchPlan,
    env: &ShellEnv,
    runner: &Runner,
) -> Result<Outcome, LaunchError> {
    match plan {
        LaunchPlan::Spawn { program, args, cwd } => {
            check_dir(cwd)?;
            match run(program, args, cwd, env, EXIT_WAIT)? {
                Exit::Ok | Exit::Running => Ok(Outcome::Opened),
                Exit::Failed(e) => Err(e.into_error()),
            }
        }
        LaunchPlan::AppleScript {
            script,
            cwd,
            fallback,
        } => {
            check_dir(cwd)?;
            let serial = APPLESCRIPT.lock().unwrap_or_else(|e| e.into_inner());
            let args = [OsString::from("-e"), OsString::from(script)];
            let reason = match run(&runner.osascript, &args, cwd, env, runner.osascript_wait) {
                Ok(Exit::Ok) => return Ok(Outcome::Opened),
                Ok(Exit::Running) => return Ok(Outcome::TimedOut),
                Ok(Exit::Failed(f)) => classify(&f.stderr),
                Err(e) => Fallback::Failed(e.to_string()),
            };
            drop(serial);
            execute_with(fallback, env, runner)?;
            Ok(Outcome::FellBack(reason))
        }
    }
}

/// Classifies osascript's error output.
fn classify(stderr: &str) -> Fallback {
    let s = stderr.to_ascii_lowercase();
    let message = osascript_message(stderr);
    if s.contains("-1743") || s.contains("not authorized to send apple events") {
        Fallback::Denied(Permission::Automation)
    } else if s.contains("(1002)")
        || s.contains("-25211")
        || s.contains("send keystrokes")
        || s.contains("assistive")
    {
        Fallback::Denied(Permission::Accessibility)
    } else if s.contains("(-2741)") || s.contains("(-2740)") {
        Fallback::Unsupported(message)
    } else if s.contains(NOT_RUNNING) {
        Fallback::NotRunning
    } else {
        Fallback::Failed(message)
    }
}

/// Last stderr line without osascript's `12:34: execution error: ` prefix.
fn osascript_message(stderr: &str) -> String {
    let line = stderr.trim().lines().last().unwrap_or_default();
    let mut rest = line;
    for _ in 0..2 {
        if let Some((n, tail)) = rest.split_once(':')
            && !n.is_empty()
            && n.bytes().all(|b| b.is_ascii_digit())
        {
            rest = tail;
        }
    }
    let rest = rest.trim_start();
    ["execution error: ", "syntax error: "]
        .iter()
        .find_map(|p| rest.strip_prefix(p))
        .unwrap_or(rest)
        .to_string()
}

fn check_dir(cwd: &Path) -> Result<(), LaunchError> {
    if cwd.is_dir() {
        Ok(())
    } else {
        Err(LaunchError::MissingFolder(cwd.to_path_buf()))
    }
}

enum Exit {
    Ok,
    /// Still running after the wait (e.g. a launcher that stays up). Reaped in background.
    Running,
    Failed(Failure),
}

struct Failure {
    program: String,
    status: String,
    /// Whole stderr, trimmed.
    stderr: String,
}

impl Failure {
    fn into_error(self) -> LaunchError {
        LaunchError::Exit {
            program: self.program,
            status: self.status,
            stderr: self.stderr.lines().last().unwrap_or_default().to_string(),
        }
    }
}

fn run(
    program: &Path,
    args: &[OsString],
    cwd: &Path,
    env: &ShellEnv,
    wait: Duration,
) -> Result<Exit, LaunchError> {
    let name = program.file_name().map_or_else(
        || program.display().to_string(),
        |n| n.to_string_lossy().into(),
    );
    let mut child = env
        .command(program)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| LaunchError::Spawn {
            program: name.clone(),
            error: e.to_string(),
        })?;

    let stderr = Arc::new(Mutex::new(StderrTail::default()));
    if let Some(mut pipe) = child.stderr.take() {
        let sink = stderr.clone();
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match pipe.read(&mut buf) {
                    Ok(0) | Err(_) => {
                        sink.lock().unwrap_or_else(|e| e.into_inner()).done = true;
                        break;
                    }
                    Ok(n) => sink
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(&buf[..n]),
                }
            }
        });
    }

    let deadline = Instant::now() + wait;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                thread::spawn(move || child.wait());
                return Ok(Exit::Running);
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(e) => {
                return Err(LaunchError::Spawn {
                    program: name,
                    error: e.to_string(),
                });
            }
        }
    };
    if status.success() {
        return Ok(Exit::Ok);
    }
    let grace = Instant::now() + STDERR_GRACE;
    while !stderr.lock().map(|s| s.done).unwrap_or(true) && Instant::now() < grace {
        thread::sleep(Duration::from_millis(5));
    }
    let text = stderr
        .lock()
        .map(|s| String::from_utf8_lossy(&s.bytes).trim().to_string())
        .unwrap_or_default();
    Ok(Exit::Failed(Failure {
        program: name,
        status: status.to_string(),
        stderr: text,
    }))
}

#[derive(Default)]
struct StderrTail {
    bytes: Vec<u8>,
    done: bool,
}

impl StderrTail {
    fn push(&mut self, bytes: &[u8]) {
        if bytes.len() >= STDERR_LIMIT {
            self.bytes.clear();
            self.bytes
                .extend_from_slice(&bytes[bytes.len() - STDERR_LIMIT..]);
            return;
        }
        let overflow = self
            .bytes
            .len()
            .saturating_add(bytes.len())
            .saturating_sub(STDERR_LIMIT);
        if overflow > 0 {
            self.bytes.drain(..overflow);
        }
        self.bytes.extend_from_slice(bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::targets::Editor;

    fn editor(e: Editor, cli: Option<&str>, app: Option<&str>) -> OpenTarget {
        OpenTarget {
            kind: TargetKind::Editor(e),
            app: app.map(PathBuf::from),
            cli: cli.map(PathBuf::from),
        }
    }

    fn argv(plan: &LaunchPlan) -> Vec<String> {
        let LaunchPlan::Spawn { program, args, .. } = plan else {
            panic!("not a spawn: {plan:?}");
        };
        std::iter::once(program.display().to_string())
            .chain(args.iter().map(|a| a.to_string_lossy().into_owned()))
            .collect()
    }

    const PATHS: &[&str] = &[
        "/Users/me/code/api",
        "/Users/me/my code/api x",
        "/Users/me/code/it's \"quoted\" $HOME `x`",
        "/Users/me/code/café-日本/wt",
    ];

    #[test]
    fn plan_table() {
        use Editor::*;
        let s = Settings::default();
        // (target, expected argv before the path)
        let cases: Vec<(OpenTarget, Vec<&str>)> = vec![
            (
                editor(VsCode, Some("/opt/homebrew/bin/code"), None),
                vec!["/opt/homebrew/bin/code", "--new-window"],
            ),
            (
                editor(VsCode, None, Some("/Applications/Visual Studio Code.app")),
                vec![
                    "/usr/bin/open",
                    "-a",
                    "/Applications/Visual Studio Code.app",
                ],
            ),
            (
                editor(Cursor, Some("/usr/local/bin/cursor"), None),
                vec!["/usr/local/bin/cursor", "--new-window"],
            ),
            (
                editor(Cursor, None, Some("/Applications/Cursor.app")),
                vec!["/usr/bin/open", "-a", "/Applications/Cursor.app"],
            ),
            (
                editor(
                    Zed,
                    Some("/usr/local/bin/zed"),
                    Some("/Applications/Zed.app"),
                ),
                vec!["/usr/local/bin/zed", "--new"],
            ),
            (
                editor(Zed, None, Some("/Applications/Zed.app")),
                vec!["/usr/bin/open", "-a", "/Applications/Zed.app"],
            ),
            (
                editor(IntelliJ, Some("/usr/local/bin/idea"), None),
                vec!["/usr/local/bin/idea"],
            ),
            (
                editor(IntelliJ, None, Some("/Applications/IntelliJ IDEA.app")),
                vec!["/usr/bin/open", "-a", "/Applications/IntelliJ IDEA.app"],
            ),
            (
                editor(Sublime, Some("/opt/homebrew/bin/subl"), None),
                vec!["/opt/homebrew/bin/subl", "--new-window"],
            ),
            (
                editor(Sublime, None, Some("/Applications/Sublime Text.app")),
                vec!["/usr/bin/open", "-a", "/Applications/Sublime Text.app"],
            ),
            // Neither CLI nor bundle path: LaunchServices resolves the app by name.
            (
                editor(Sublime, None, None),
                vec!["/usr/bin/open", "-a", "Sublime Text"],
            ),
        ];
        for (target, prefix) in &cases {
            for path in PATHS {
                let p = plan(target, Path::new(path), &s);
                let mut want: Vec<String> = prefix.iter().map(|s| s.to_string()).collect();
                // The path is one argv entry, byte for byte; no shell quoting.
                want.push(path.to_string());
                assert_eq!(argv(&p), want, "{:?}", target.kind);
                assert_eq!(cwd(&p), Path::new(path));
            }
        }
    }

    #[test]
    fn execute_reports_missing_folder_and_spawn_failure() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env = crate::resolved_env();
        let gone = tmp.path().join("gone");
        let p = LaunchPlan::Spawn {
            program: "/bin/echo".into(),
            args: vec![],
            cwd: gone.clone(),
        };
        assert_eq!(execute(&p, env), Err(LaunchError::MissingFolder(gone)));

        let p = LaunchPlan::Spawn {
            program: tmp.path().join("no-such-cli"),
            args: vec![],
            cwd: tmp.path().into(),
        };
        let err = execute(&p, env).unwrap_err();
        assert!(
            err.to_string().starts_with("Couldn't run no-such-cli:"),
            "{err}"
        );
    }

    #[test]
    fn stderr_tail_retains_only_the_configured_tail() {
        let mut tail = StderrTail::default();
        let input: Vec<u8> = (0..STDERR_LIMIT + 137)
            .map(|i| u8::try_from(i % 251).unwrap())
            .collect();
        for chunk in input.chunks(997) {
            tail.push(chunk);
        }

        assert_eq!(tail.bytes, input[input.len() - STDERR_LIMIT..]);
    }

    #[test]
    fn stderr_tail_bounds_a_single_oversized_read() {
        let mut tail = StderrTail::default();
        let input = vec![b'x'; STDERR_LIMIT + 1];
        tail.push(&input);

        assert_eq!(tail.bytes.len(), STDERR_LIMIT);
    }

    #[test]
    fn run_returns_while_noisy_child_remains_running() {
        let tmp = tempfile::TempDir::new().unwrap();
        let args = [
            OsString::from("-c"),
            OsString::from("yes noisy >&2 & writer=$!; sleep 1; kill $writer"),
        ];

        let exit = run(
            Path::new("/bin/sh"),
            &args,
            tmp.path(),
            crate::resolved_env(),
            Duration::from_millis(30),
        )
        .unwrap();

        assert!(matches!(exit, Exit::Running));
    }

    #[test]
    fn execute_reports_exit_status_with_stderr() {
        let tmp = tempfile::TempDir::new().unwrap();
        let p = LaunchPlan::Spawn {
            program: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                "echo noise >&2; echo 'boom: bad flag' >&2; exit 3".into(),
            ],
            cwd: tmp.path().into(),
        };
        let err = execute(&p, crate::resolved_env()).unwrap_err();
        assert_eq!(err.to_string(), "sh failed: boom: bad flag");
        let p = LaunchPlan::Spawn {
            program: "/usr/bin/false".into(),
            args: vec![],
            cwd: tmp.path().into(),
        };
        assert_eq!(
            execute(&p, crate::resolved_env()).unwrap_err().to_string(),
            "false failed (exit status: 1)"
        );
    }

    fn cwd(plan: &LaunchPlan) -> &Path {
        match plan {
            LaunchPlan::Spawn { cwd, .. } | LaunchPlan::AppleScript { cwd, .. } => cwd,
        }
    }

    fn term(t: Terminal, app: Option<&str>, cli: Option<&str>) -> OpenTarget {
        OpenTarget {
            kind: TargetKind::Terminal(t),
            app: app.map(PathBuf::from),
            cli: cli.map(PathBuf::from),
        }
    }

    fn opens_in(o: TerminalOpensIn) -> Settings {
        Settings {
            terminal_opens_in: Some(o),
            ..Default::default()
        }
    }

    #[test]
    fn unset_opens_in_uses_window_for_terminal_app_and_tab_elsewhere() {
        let dir = Path::new("/w");
        let unset = Settings::default();
        let LaunchPlan::AppleScript { script, .. } =
            plan(&term(Terminal::Terminal, None, None), dir, &unset)
        else {
            panic!("Terminal.app plans AppleScript");
        };
        assert!(!script.contains("keystroke"), "window mode sends no ⌘T");
        let LaunchPlan::Spawn { args, .. } = plan(&term(Terminal::Warp, None, None), dir, &unset)
        else {
            panic!("Warp plans a spawn");
        };
        assert!(args.iter().any(|a| a.to_string_lossy().contains("new_tab")));
    }

    #[test]
    fn command_line_names_the_cli_or_open_a() {
        use Editor::*;
        let line = |t: OpenTarget| command_line(&t);
        let zed_app = Some("/Applications/Zed.app");
        let bundled = Some("/Applications/Zed.app/Contents/MacOS/cli");
        assert_eq!(
            line(editor(VsCode, Some("/opt/homebrew/bin/code"), None)).as_deref(),
            Some("code --new-window {path}")
        );
        assert_eq!(
            line(editor(Zed, bundled, zed_app)).as_deref(),
            Some("zed --new {path}")
        );
        assert_eq!(
            line(editor(
                IntelliJ,
                None,
                Some("/Applications/IntelliJ IDEA.app")
            ))
            .as_deref(),
            Some("open -a IntelliJ IDEA {path}")
        );
        assert_eq!(line(term(Terminal::Warp, None, None)), None);
    }

    /// (path, shell-quoted `cd` as an AppleScript literal, Warp `path=` value)
    const TRICKY: &[(&str, &str, &str)] = &[
        (
            "/Users/me/code/api",
            r#""cd '/Users/me/code/api'""#,
            "/Users/me/code/api",
        ),
        (
            "/Users/me/my code/api x",
            r#""cd '/Users/me/my code/api x'""#,
            "/Users/me/my%20code/api%20x",
        ),
        (
            "/c/it's \"q\"",
            r#""cd '/c/it'\\''s \"q\"'""#,
            "/c/it%27s%20%22q%22",
        ),
        (
            r"/c/back\slash $HOME `x`",
            r#""cd '/c/back\\slash $HOME `x`'""#,
            "/c/back%5Cslash%20%24HOME%20%60x%60",
        ),
        (
            "/c/café-日本",
            r#""cd '/c/café-日本'""#,
            "/c/caf%C3%A9-%E6%97%A5%E6%9C%AC",
        ),
    ];

    const TABS: [TerminalOpensIn; 2] = [TerminalOpensIn::Tab, TerminalOpensIn::Window];

    #[test]
    fn terminal_and_iterm_plan_applescript_with_open_a_fallback() {
        let cases = [
            (
                term(
                    Terminal::Terminal,
                    Some("/System/Applications/Utilities/Terminal.app"),
                    None,
                ),
                "/System/Applications/Utilities/Terminal.app",
            ),
            (term(Terminal::Terminal, None, None), "Terminal"),
            (
                term(Terminal::ITerm2, Some("/Applications/iTerm.app"), None),
                "/Applications/iTerm.app",
            ),
        ];
        for (target, app) in &cases {
            for o in TABS {
                for (path, cd, _) in TRICKY {
                    let p = plan(target, Path::new(path), &opens_in(o));
                    let LaunchPlan::AppleScript {
                        script, fallback, ..
                    } = &p
                    else {
                        panic!("{p:?}");
                    };
                    assert_eq!(cwd(&p), Path::new(path));
                    assert!(script.contains(cd), "{script}\nmissing {cd}");
                    assert_eq!(argv(fallback), ["/usr/bin/open", "-a", app, path]);
                    assert_eq!(cwd(fallback), Path::new(path));

                    let tab = o == TerminalOpensIn::Tab;
                    let new_tab = match target.kind {
                        TargetKind::Terminal(Terminal::Terminal) => {
                            assert!(script.contains("application id \"com.apple.Terminal\""));
                            r#"keystroke "t" using command down"#
                        }
                        _ => {
                            assert!(script.contains("application id \"com.googlecode.iterm2\""));
                            assert!(script.contains("write text"));
                            "create tab with default profile"
                        }
                    };
                    assert_eq!(script.contains(new_tab), tab, "{o:?}\n{script}");
                }
            }
        }
    }

    #[test]
    fn warp_plans_uri_for_tab_and_window() {
        let target = term(Terminal::Warp, Some("/Applications/Warp.app"), None);
        for o in TABS {
            let action = match o {
                TerminalOpensIn::Tab => "new_tab",
                TerminalOpensIn::Window => "new_window",
            };
            for (path, _, encoded) in TRICKY {
                let p = plan(&target, Path::new(path), &opens_in(o));
                assert_eq!(
                    argv(&p),
                    [
                        "/usr/bin/open".to_string(),
                        format!("warp://action/{action}?path={encoded}")
                    ]
                );
                assert_eq!(cwd(&p), Path::new(path));
            }
        }
    }

    #[test]
    fn ghostty_window_via_open_n_and_tab_via_applescript() {
        let target = term(
            Terminal::Ghostty,
            Some("/Applications/Ghostty.app"),
            Some("/Applications/Ghostty.app/Contents/MacOS/ghostty"),
        );
        for (path, _, _) in TRICKY {
            let window = [
                "/usr/bin/open".to_string(),
                "-na".into(),
                "/Applications/Ghostty.app".into(),
                "--args".into(),
                format!("--working-directory={path}"),
            ];
            let p = plan(&target, Path::new(path), &opens_in(TerminalOpensIn::Window));
            assert_eq!(argv(&p), window);
            assert_eq!(cwd(&p), Path::new(path));

            let p = plan(&target, Path::new(path), &opens_in(TerminalOpensIn::Tab));
            let LaunchPlan::AppleScript {
                script, fallback, ..
            } = &p
            else {
                panic!("{p:?}");
            };
            let dir = applescript_string(path);
            assert!(
                script.contains(&format!("set initial working directory of cfg to {dir}")),
                "{script}"
            );
            assert!(script.contains("new tab in front window"));
            assert_eq!(argv(fallback), window);
            assert_eq!(cwd(&p), Path::new(path));
        }
    }

    #[test]
    fn kitty_always_opens_a_window() {
        let app = term(Terminal::Kitty, Some("/Applications/kitty.app"), None);
        let cli_only = term(Terminal::Kitty, None, Some("/opt/homebrew/bin/kitty"));
        for o in TABS {
            for (path, _, _) in TRICKY {
                let p = plan(&app, Path::new(path), &opens_in(o));
                assert_eq!(
                    argv(&p),
                    [
                        "/usr/bin/open",
                        "-na",
                        "/Applications/kitty.app",
                        "--args",
                        "--single-instance",
                        "--directory",
                        path
                    ]
                );
                assert_eq!(cwd(&p), Path::new(path));
                let p = plan(&cli_only, Path::new(path), &opens_in(o));
                assert_eq!(
                    argv(&p),
                    [
                        "/opt/homebrew/bin/kitty",
                        "--single-instance",
                        "--directory",
                        path
                    ]
                );
            }
        }
    }

    /// The quoting really round-trips: osascript decodes the literal back to the `cd`
    /// command, and sh/zsh running it land in the directory. Runs no app's AppleScript.
    #[cfg(target_os = "macos")]
    #[test]
    fn cd_quoting_round_trips_through_osascript_and_shells() {
        let tmp = tempfile::TempDir::new().unwrap();
        let names = [
            "plain",
            "my code",
            "it's \"q\"",
            r"back\slash \\ two",
            "$HOME `x` $(echo no)",
            "café-日本",
            "tab\there; & | * ?",
        ];
        for name in names {
            let dir = tmp.path().join(name);
            std::fs::create_dir_all(&dir).unwrap();
            let cd = format!("cd {}", shell_quote(&dir));
            let lit = applescript_string(&cd);
            let out = std::process::Command::new(OSASCRIPT)
                .arg("-e")
                .arg(format!("return {lit}"))
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            let decoded = String::from_utf8(out.stdout).unwrap();
            assert_eq!(decoded.strip_suffix('\n').unwrap(), cd);
            for shell in ["/bin/sh", "/bin/zsh", "/bin/bash"] {
                let out = std::process::Command::new(shell)
                    .arg("-c")
                    .arg(format!("{cd} && pwd -P"))
                    .current_dir("/")
                    .output()
                    .unwrap();
                let got = String::from_utf8(out.stdout).unwrap();
                assert_eq!(
                    got.trim_end_matches('\n'),
                    dir.canonicalize().unwrap().to_str().unwrap(),
                    "{shell} {cd}"
                );
            }
        }
    }

    #[test]
    fn classifies_osascript_errors() {
        use Fallback::*;
        let cases = [
            (
                "0:120: execution error: Not authorized to send Apple events to Terminal. (-1743)",
                Denied(Permission::Automation),
            ),
            (
                "execution error: System Events got an error: osascript is not allowed to send keystrokes. (1002)",
                Denied(Permission::Accessibility),
            ),
            (
                "System Events got an error: osascript is not allowed assistive access. (-1719)",
                Denied(Permission::Accessibility),
            ),
            // -1719 without "assistive" is an ordinary "invalid index" error.
            (
                "12:40: execution error: Can't get window 1. Invalid index. (-1719)",
                Failed("Can't get window 1. Invalid index. (-1719)".into()),
            ),
            (
                "88:95: syntax error: Expected end of line but found identifier. (-2741)",
                Unsupported("Expected end of line but found identifier. (-2741)".into()),
            ),
            (
                "1:2: syntax error: A identifier can't go after this identifier. (-2740)",
                Unsupported("A identifier can't go after this identifier. (-2740)".into()),
            ),
            (
                "0:130: execution error: Ghostty isn't running (9001)",
                NotRunning,
            ),
            (
                "noise\n431:470: execution error: Terminal didn't open a new tab (-2700)",
                Failed("Terminal didn't open a new tab (-2700)".into()),
            ),
        ];
        for (stderr, want) in cases {
            assert_eq!(classify(stderr), want, "{stderr}");
        }
    }

    #[test]
    fn terminal_tab_refuses_to_send_cmd_t_elsewhere() {
        let t = term(Terminal::Terminal, None, None);
        let LaunchPlan::AppleScript { script, .. } =
            plan(&t, Path::new("/c"), &opens_in(TerminalOpensIn::Tab))
        else {
            panic!()
        };
        let front = script.find(r"if not frontmost then error").unwrap();
        let keystroke = script.find("keystroke").unwrap();
        assert!(front < keystroke, "{script}");
        assert!(script.contains("repeat 100 times"), "~5 s cold-launch wait");
    }

    #[test]
    fn iterm_writes_into_the_tab_it_created_and_reuses_cold_launch_window() {
        let t = term(Terminal::ITerm2, None, None);
        let LaunchPlan::AppleScript { script, .. } =
            plan(&t, Path::new("/c"), &Settings::default())
        else {
            panic!()
        };
        assert!(script.contains("set t to (create tab with default profile)"));
        assert!(script.contains("set s to current session of t"));
        assert!(script.contains("tell s to write text \"cd '/c'\""));
        assert!(script.contains(
            "else if not wasRunning then\n\t\tset s to current session of current window"
        ));
        assert!(!script.contains("current session of w"));
    }

    #[test]
    fn ghostty_tab_hands_cold_launch_to_open_n() {
        let t = term(Terminal::Ghostty, Some("/Applications/Ghostty.app"), None);
        let LaunchPlan::AppleScript { script, .. } =
            plan(&t, Path::new("/c"), &Settings::default())
        else {
            panic!()
        };
        assert!(script.starts_with(
            r#"if not (application id "com.mitchellh.ghostty" is running) then error "Ghostty isn't running" number 9001"#
        ));
    }

    fn command(template: &str, run_in_terminal: bool) -> CustomCommand {
        CustomCommand {
            label: "Mine".into(),
            template: template.into(),
            run_in_terminal,
        }
    }

    /// (path, as `{path}` becomes)
    const QUOTED: &[(&str, &str)] = &[
        ("/Users/me/code/api", "'/Users/me/code/api'"),
        ("/Users/me/my code/api x", "'/Users/me/my code/api x'"),
        ("/c/it's", r"'/c/it'\''s'"),
        ("/c/\"q\" x", "'/c/\"q\" x'"),
        ("/c/$HOME `x` $(no)", "'/c/$HOME `x` $(no)'"),
        (r"/c/back\slash", r"'/c/back\slash'"),
        ("/c/{input}{dir}", "'/c/{input}{dir}'"),
    ];

    /// Fine for background commands; refused when typed into a terminal.
    const CONTROL: &[(&str, &str)] = &[
        ("/c/line\nbreak", "'/c/line\nbreak'"),
        ("/c/x\u{15}\nrm -rf ~", "'/c/x\u{15}\nrm -rf ~'"),
        ("/c/tab\there", "'/c/tab\there'"),
    ];

    #[test]
    fn command_off_runs_sh_in_the_background() {
        let cmd = command("subl -n {path} && echo {path}", false);
        // No terminal needed.
        for (path, q) in QUOTED.iter().chain(CONTROL) {
            let p = plan_command(&cmd, Path::new(path), &Settings::default(), &[]).unwrap();
            assert_eq!(
                argv(&p),
                ["/bin/sh", "-c", &format!("subl -n {q} && echo {q}")]
            );
            assert_eq!(cwd(&p), Path::new(path));
        }
    }

    fn with_terminal(t: OpenTarget, o: TerminalOpensIn) -> (Settings, Vec<OpenTarget>) {
        let s = Settings {
            default_terminal: Some(t.id().into()),
            terminal_opens_in: Some(o),
            ..Default::default()
        };
        (s, vec![editor(Editor::Zed, Some("/bin/zed"), None), t])
    }

    #[test]
    fn command_on_types_it_into_terminal_and_iterm() {
        let cmd = command("nvim {path}", true);
        let cases = [
            (
                term(Terminal::Terminal, Some("/Apps/Terminal.app"), None),
                r#"keystroke "t" using command down"#,
            ),
            (
                term(Terminal::ITerm2, Some("/Apps/iTerm.app"), None),
                "create tab with default profile",
            ),
        ];
        for (target, new_tab) in cases {
            let app = target.app.clone().unwrap();
            for o in TABS {
                let (s, targets) = with_terminal(target.clone(), o);
                for (path, q) in QUOTED {
                    let p = plan_command(&cmd, Path::new(path), &s, &targets).unwrap();
                    let LaunchPlan::AppleScript {
                        script, fallback, ..
                    } = &p
                    else {
                        panic!("{p:?}");
                    };
                    let typed = applescript_string(&format!("cd {q} && nvim {q}"));
                    assert!(script.contains(&typed), "{script}\nmissing {typed}");
                    assert_eq!(script.contains(new_tab), o == TerminalOpensIn::Tab);
                    assert_eq!(cwd(&p), Path::new(path));
                    let app = app.to_str().unwrap();
                    assert_eq!(argv(fallback), ["/usr/bin/open", "-a", app, path]);
                }
            }
        }
    }

    #[test]
    fn command_on_runs_it_in_ghostty_and_kitty() {
        let cmd = command("nvim {path}", true);
        let ghostty = term(Terminal::Ghostty, Some("/Apps/Ghostty.app"), None);
        let kitty = term(Terminal::Kitty, None, Some("/bin/kitty"));
        for (path, q) in QUOTED {
            let line = format!("nvim {q}");
            let run = ["/bin/sh", "-c", IN_LOGIN_SHELL, "sh", &line].map(String::from);
            let mut window = vec![
                "/usr/bin/open".to_string(),
                "-na".into(),
                "/Apps/Ghostty.app".into(),
                "--args".into(),
                format!("--working-directory={path}"),
                "-e".into(),
            ];
            window.extend(run.clone());

            let (s, targets) = with_terminal(ghostty.clone(), TerminalOpensIn::Window);
            let p = plan_command(&cmd, Path::new(path), &s, &targets).unwrap();
            assert_eq!(argv(&p), window);

            let (s, targets) = with_terminal(ghostty.clone(), TerminalOpensIn::Tab);
            let p = plan_command(&cmd, Path::new(path), &s, &targets).unwrap();
            let LaunchPlan::AppleScript {
                script, fallback, ..
            } = &p
            else {
                panic!("{p:?}");
            };
            let input = applescript_string(&format!("{line}\n"));
            assert!(
                script.contains(&format!("set initial input of cfg to {input}")),
                "{script}"
            );
            assert_eq!(argv(fallback), window);

            for o in TABS {
                let (s, targets) = with_terminal(kitty.clone(), o);
                let p = plan_command(&cmd, Path::new(path), &s, &targets).unwrap();
                let mut want = vec![
                    "/bin/kitty".to_string(),
                    "--single-instance".into(),
                    "--directory".into(),
                    path.to_string(),
                ];
                want.extend(run.clone());
                assert_eq!(argv(&p), want);
                assert_eq!(cwd(&p), Path::new(path));
            }
        }
    }

    #[test]
    fn command_on_needs_a_terminal_that_runs_commands() {
        let cmd = command("nvim {path}", true);
        let dir = Path::new("/c");
        let (s, targets) = with_terminal(term(Terminal::Warp, None, None), TerminalOpensIn::Tab);
        let err = plan_command(&cmd, dir, &s, &targets).unwrap_err();
        assert_eq!(err, NoTarget::CantRunCommands(Terminal::Warp).into());
        assert!(err.in_settings());
        assert!(
            err.to_string()
                .starts_with("Warp can't run custom commands")
        );
        assert_eq!(
            plan_command(&cmd, dir, &Settings::default(), &targets[..1]),
            Err(NoTarget::NoneDetected(crate::targets::Role::Terminal).into())
        );
    }

    #[test]
    fn control_characters_are_never_typed_into_a_terminal() {
        let cmd = command("nvim {path}", true);
        let terminals = [
            term(Terminal::Terminal, Some("/Apps/Terminal.app"), None),
            term(Terminal::ITerm2, Some("/Apps/iTerm.app"), None),
            term(Terminal::Ghostty, Some("/Apps/Ghostty.app"), None),
            term(Terminal::Kitty, None, Some("/bin/kitty")),
        ];
        for target in terminals {
            for o in TABS {
                let (s, targets) = with_terminal(target.clone(), o);
                for (path, _) in CONTROL {
                    let err = plan_command(&cmd, Path::new(path), &s, &targets).unwrap_err();
                    assert_eq!(err, CommandError::ControlChars(path.into()));
                    assert!(!err.in_settings());
                    assert!(!err.to_string().chars().any(char::is_control), "{err}");
                    // A plain open of Terminal / iTerm2 just opens the folder.
                    let p = plan(&target, Path::new(path), &s);
                    if let TargetKind::Terminal(Terminal::Terminal | Terminal::ITerm2) = target.kind
                    {
                        let app = target.app.as_ref().unwrap().to_str().unwrap();
                        assert_eq!(argv(&p), ["/usr/bin/open", "-a", app, path]);
                    }
                }
            }
        }
    }

    /// The background command and the in-terminal wrapper really get the path as one word.
    #[test]
    fn command_quoting_round_trips_through_sh() {
        let tmp = tempfile::TempDir::new().unwrap();
        let cmd = command("printf '%s' {path} > out", false);
        for name in [
            "my code",
            "it's \"q\"",
            "$HOME `x` $(echo no)",
            r"back\slash",
            "café; & |",
        ] {
            let dir = tmp.path().join(name);
            std::fs::create_dir_all(&dir).unwrap();
            let p = plan_command(&cmd, &dir, &Settings::default(), &[]).unwrap();
            let env = ShellEnv {
                path: "/usr/bin:/bin".into(),
                source: crate::PathSource::Fallback,
                shell: SH.into(),
            };
            execute(&p, &env).unwrap();
            let out = std::fs::read_to_string(dir.join("out")).unwrap();
            assert_eq!(Path::new(&out), dir);

            let line = cmd.template.replace("{path}", &shell_quote(&dir));
            std::fs::remove_file(dir.join("out")).unwrap();
            let ok = std::process::Command::new(SH)
                .args(["-c", IN_LOGIN_SHELL, "sh", &line])
                .env("SHELL", SH)
                .current_dir(&dir)
                .stdin(Stdio::null())
                .status()
                .unwrap()
                .success();
            assert!(ok);
            let out = std::fs::read_to_string(dir.join("out")).unwrap();
            assert_eq!(Path::new(&out), dir);
        }
    }
}
