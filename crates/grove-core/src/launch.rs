//! Launcher: pure planning (`plan`) and execution (`execute`). Never dedupes or focuses an
//! existing window; every open asks for a new window or tab.

use std::ffi::OsString;
use std::fmt;
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::env::ShellEnv;
use crate::settings::{Settings, TerminalOpensIn};
use crate::targets::{OpenTarget, TargetKind, Terminal};

const OPEN: &str = "/usr/bin/open";
const OSASCRIPT: &str = "/usr/bin/osascript";
/// Editor CLIs hand off to the app and exit; one still running after this is left running.
const EXIT_WAIT: Duration = Duration::from_secs(5);
/// osascript blocks while macOS shows the Automation prompt; give the user time to answer.
const OSASCRIPT_WAIT: Duration = Duration::from_secs(120);
/// After exit, how long to wait for stderr EOF (a daemonized child may hold the pipe).
const STDERR_GRACE: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchPlan {
    /// Run `program` with `args` in `cwd`, resolved PATH applied. No shell involved.
    Spawn {
        program: PathBuf,
        args: Vec<OsString>,
        cwd: PathBuf,
    },
    /// Run `script` with osascript; if it fails (e.g. Automation denied), run `fallback`.
    AppleScript {
        script: String,
        cwd: PathBuf,
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
            terminal_plan(terminal, target, worktree, settings.terminal_opens_in)
        }
    }
}

fn terminal_plan(
    terminal: Terminal,
    target: &OpenTarget,
    worktree: &Path,
    opens_in: TerminalOpensIn,
) -> LaunchPlan {
    let tab = opens_in == TerminalOpensIn::Tab;
    let path = OsString::from(worktree);
    // LaunchServices opens a folder in Terminal / iTerm2 as a new window at that folder.
    let open_a = || {
        spawn(
            OPEN.into(),
            vec!["-a".into(), app_arg(target), path.clone()],
            worktree,
        )
    };
    match terminal {
        Terminal::Terminal | Terminal::ITerm2 => {
            let cd = applescript_string(&format!("cd {}", shell_quote(worktree)));
            let script = match (terminal, tab) {
                (Terminal::Terminal, _) => terminal_app_script(&cd, tab),
                _ => iterm_script(&cd, tab),
            };
            applescript(script, worktree, open_a())
        }
        Terminal::Warp => {
            let action = if tab { "new_tab" } else { "new_window" };
            let url = format!("warp://action/{action}?path={}", percent_encode(worktree));
            spawn(OPEN.into(), vec![url.into()], worktree)
        }
        Terminal::Ghostty => {
            let mut wd = OsString::from("--working-directory=");
            wd.push(&path);
            // `open -n` starts a new Ghostty process; Ghostty's documented way to pass
            // CLI config on macOS. It always gives a window.
            let window = spawn(
                OPEN.into(),
                vec!["-na".into(), app_arg(target), "--args".into(), wd],
                worktree,
            );
            if tab {
                // Tabs need Ghostty's AppleScript dictionary (1.3+); older versions fail
                // to compile the script (`Fallback::Unsupported`) and get the window.
                let dir = applescript_string(&worktree.to_string_lossy());
                applescript(GHOSTTY_TAB.replace("{dir}", &dir), worktree, window)
            } else {
                window
            }
        }
        // Window only: tabs need `kitty @ launch`, which requires remote control to be
        // enabled in kitty.conf. `--single-instance` adds the window to a running kitty.
        Terminal::Kitty => {
            let kitty_args: Vec<OsString> = vec![
                "--single-instance".into(),
                "--directory".into(),
                path.clone(),
            ];
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

const ITERM_TAB: &str = r#"		tell current window to set t to (create tab with default profile)
		set s to current session of t
"#;

/// Error number a script raises to hand over to its fallback quietly.
const NOT_RUNNING: &str = "(9001)";

/// Not running → the `open -na` fallback, so a cold launch makes one window, not two.
const GHOSTTY_TAB: &str = r#"if not (application id "com.mitchellh.ghostty" is running) then error "Ghostty isn't running" number 9001
tell application id "com.mitchellh.ghostty"
	activate
	set cfg to new surface configuration
	set initial working directory of cfg to {dir}
	if (count of windows) is 0 then
		new window with configuration cfg
	else
		new tab in front window with configuration cfg
	end if
end tell"#;

/// POSIX single-quoted shell word: `it's` → `'it'\''s'`.
fn shell_quote(path: &Path) -> String {
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
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchError {
    MissingFolder(PathBuf),
    Spawn {
        program: String,
        error: String,
    },
    Exit {
        program: String,
        status: String,
        stderr: String,
    },
}

impl fmt::Display for LaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LaunchError::MissingFolder(p) => write!(f, "Folder not found: {}", p.display()),
            LaunchError::Spawn { program, error } => write!(f, "Couldn't run {program}: {error}"),
            LaunchError::Exit {
                program,
                status,
                stderr,
            } if stderr.is_empty() => write!(f, "{program} failed ({status})"),
            LaunchError::Exit {
                program, stderr, ..
            } => write!(f, "{program} failed: {stderr}"),
        }
    }
}

/// The macOS privacy pane that blocked an AppleScript plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Automation,
    Accessibility,
}

impl Permission {
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
    Denied(Permission),
    /// The script didn't compile: the app has no (or an older) scripting dictionary.
    Unsupported(String),
    /// The app wasn't running; the fallback is the normal way to cold-launch it.
    NotRunning,
    /// Any other osascript failure (its message).
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
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
    pub osascript: PathBuf,
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
pub fn execute(plan: &LaunchPlan, env: &ShellEnv) -> Result<Outcome, LaunchError> {
    execute_with(plan, env, &Runner::default())
}

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

    let stderr = Arc::new(Mutex::new((Vec::new(), false)));
    if let Some(mut pipe) = child.stderr.take() {
        let sink = stderr.clone();
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = pipe.read(&mut buf) {
                let mut s = sink.lock().unwrap_or_else(|e| e.into_inner());
                if n == 0 {
                    s.1 = true;
                    break;
                }
                s.0.extend_from_slice(&buf[..n]);
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
    while !stderr.lock().map(|s| s.1).unwrap_or(true) && Instant::now() < grace {
        thread::sleep(Duration::from_millis(5));
    }
    let text = stderr
        .lock()
        .map(|s| String::from_utf8_lossy(&s.0).trim().to_string())
        .unwrap_or_default();
    Ok(Exit::Failed(Failure {
        program: name,
        status: status.to_string(),
        stderr: text,
    }))
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
            terminal_opens_in: o,
            ..Default::default()
        }
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
            plan(&t, Path::new("/c"), &Settings::default())
        else {
            panic!()
        };
        let front = script.find(r#"if not frontmost then error"#).unwrap();
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
}
