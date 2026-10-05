//! Launcher: pure planning (`plan`) and execution (`execute`). Never dedupes or focuses an
//! existing window; every open asks for a new one.

use std::ffi::OsString;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::env::ShellEnv;
use crate::settings::Settings;
use crate::targets::{OpenTarget, TargetKind};

const OPEN: &str = "/usr/bin/open";
/// Editor CLIs hand off to the app and exit; one still running after this is left running.
const EXIT_WAIT: Duration = Duration::from_secs(5);
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
}

/// How to open `worktree` in `target`. Editor with a CLI → the CLI with its new-window
/// flag; otherwise `open -a <App> <path>` (LaunchServices; the app decides about windows).
pub fn plan(target: &OpenTarget, worktree: &Path, _settings: &Settings) -> LaunchPlan {
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
                None => {
                    let app = target
                        .app
                        .as_ref()
                        .map_or_else(|| OsString::from(editor.name()), OsString::from);
                    (PathBuf::from(OPEN), vec!["-a".into(), app, path])
                }
            };
            LaunchPlan::Spawn {
                program,
                args,
                cwd: worktree.to_path_buf(),
            }
        }
    }
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

/// Runs `plan` with `env`. Blocking (waits up to a few seconds for the CLI to exit); run
/// off the UI thread.
pub fn execute(plan: &LaunchPlan, env: &ShellEnv) -> Result<(), LaunchError> {
    let LaunchPlan::Spawn { program, args, cwd } = plan;
    if !cwd.is_dir() {
        return Err(LaunchError::MissingFolder(cwd.clone()));
    }
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

    let deadline = Instant::now() + EXIT_WAIT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                // Still running (e.g. a launcher that stays in the foreground): that's a
                // successful launch. Reap it whenever it exits.
                thread::spawn(move || child.wait());
                return Ok(());
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
        return Ok(());
    }
    let grace = Instant::now() + STDERR_GRACE;
    while !stderr.lock().map(|s| s.1).unwrap_or(true) && Instant::now() < grace {
        thread::sleep(Duration::from_millis(5));
    }
    let text = stderr
        .lock()
        .map(|s| String::from_utf8_lossy(&s.0).trim().to_string())
        .unwrap_or_default();
    Err(LaunchError::Exit {
        program: name,
        status: status.to_string(),
        stderr: text.lines().last().unwrap_or_default().to_string(),
    })
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
        let LaunchPlan::Spawn { program, args, .. } = plan;
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
                let LaunchPlan::Spawn { cwd, .. } = &p;
                assert_eq!(cwd, Path::new(path));
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
}
