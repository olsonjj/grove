//! Execution through stub CLIs / a stub osascript: what actually reaches the process.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use grove_core::{
    Fallback, LaunchError, LaunchPlan, OpenTarget, Outcome, PathSource, Permission, Runner,
    Settings, ShellEnv, TargetKind, Terminal, TerminalOpensIn, default_editor, default_terminal,
    detect_in, execute, execute_with, plan,
};
use tempfile::TempDir;

/// A fake `code` that records its argv (one per line) and cwd.
fn stub_code(bin: &Path, record: &Path) {
    fs::create_dir_all(bin).unwrap();
    let script = format!(
        "#!/bin/sh\n{{ pwd -P; for a in \"$@\"; do printf '%s\\n' \"$a\"; done; }} > '{}'\n",
        record.display()
    );
    let cli = bin.join("code");
    fs::write(&cli, script).unwrap();
    fs::set_permissions(&cli, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn stub_cli_receives_new_window_flag_path_and_cwd() {
    let tmp = TempDir::new().unwrap();
    let bin = tmp.path().join("bin");
    let record = tmp.path().join("record.txt");
    stub_code(&bin, &record);
    let wt = tmp.path().join("my code/it's \"wt\" café");
    fs::create_dir_all(&wt).unwrap();

    let env = ShellEnv {
        path: format!("{}:/usr/bin:/bin", bin.display()),
        source: PathSource::LoginShell,
        shell: "/bin/sh".into(),
    };
    let targets = detect_in(&env, &[], false);
    let settings = Settings::default();
    let editor = default_editor(&settings, &targets).unwrap();
    assert_eq!(editor.name(), "VS Code");
    let p = plan(editor, &wt, &settings);
    let LaunchPlan::Spawn { program, .. } = &p else {
        panic!("{p:?}");
    };
    assert_eq!(program, &bin.join("code"));

    execute(&p, &env).unwrap();
    let got = fs::read_to_string(&record).unwrap();
    let lines: Vec<&str> = got.lines().collect();
    let real = wt.canonicalize().unwrap();
    assert_eq!(
        lines,
        [real.to_str().unwrap(), "--new-window", wt.to_str().unwrap()]
    );
}

#[cfg(target_os = "macos")]
#[test]
fn open_a_reports_unknown_app() {
    let tmp = TempDir::new().unwrap();
    let p = LaunchPlan::Spawn {
        program: "/usr/bin/open".into(),
        args: vec![
            "-a".into(),
            tmp.path().join("No Such Editor.app").into(),
            tmp.path().into(),
        ],
        cwd: tmp.path().into(),
    };
    let err = execute(&p, grove_core::resolved_env()).unwrap_err();
    assert!(err.to_string().starts_with("open failed"), "{err}");
}

/// A program that records cwd, argv (one per line) to `record`, writes `stderr`, exits `code`.
fn stub(path: &Path, record: &Path, stderr: &str, code: i32) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let script = format!(
        "#!/bin/sh\n{{ pwd -P; for a in \"$@\"; do printf '%s\\n' \"$a\"; done; }} > '{}'\nprintf '%s\\n' '{}' >&2\nexit {code}\n",
        record.display(),
        stderr
    );
    fs::write(path, script).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn env() -> ShellEnv {
    ShellEnv {
        path: "/usr/bin:/bin".into(),
        source: PathSource::LoginShell,
        shell: "/bin/sh".into(),
    }
}

struct Rig {
    tmp: TempDir,
    wt: std::path::PathBuf,
    osascript: std::path::PathBuf,
}

impl Rig {
    fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let wt = tmp.path().join("my code/it's \"wt\" $x");
        fs::create_dir_all(&wt).unwrap();
        let osascript = tmp.path().join("bin/osascript");
        Rig { tmp, wt, osascript }
    }

    fn runner(&self) -> Runner {
        Runner {
            osascript: self.osascript.clone(),
            ..Default::default()
        }
    }

    fn record(&self, name: &str) -> std::path::PathBuf {
        self.tmp.path().join(name)
    }

    fn lines(&self, name: &str) -> Vec<String> {
        fs::read_to_string(self.record(name))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// Terminal tab plan whose `open` fallback is swapped for a recording stub.
    fn terminal_plan(&self, fallback_code: i32) -> LaunchPlan {
        let terminal = OpenTarget {
            kind: TargetKind::Terminal(Terminal::Terminal),
            app: Some("/System/Applications/Utilities/Terminal.app".into()),
            cli: None,
        };
        let mut p = plan(&terminal, &self.wt, &Settings::default());
        let LaunchPlan::AppleScript { fallback, .. } = &mut p else {
            panic!("{p:?}");
        };
        let LaunchPlan::Spawn { program, .. } = fallback.as_mut() else {
            panic!("{fallback:?}");
        };
        let open = self.tmp.path().join("bin/open");
        stub(&open, &self.record("open.txt"), "open: boom", fallback_code);
        *program = open;
        p
    }
}

#[test]
fn applescript_plan_runs_osascript_in_worktree() {
    let rig = Rig::new();
    stub(&rig.osascript, &rig.record("osa.txt"), "", 0);
    let p = rig.terminal_plan(0);
    let LaunchPlan::AppleScript { script, .. } = &p else {
        unreachable!()
    };
    let script = script.clone();
    assert_eq!(execute_with(&p, &env(), &rig.runner()), Ok(Outcome::Opened));
    let got = fs::read_to_string(rig.record("osa.txt")).unwrap();
    let real = rig.wt.canonicalize().unwrap();
    assert_eq!(got, format!("{}\n-e\n{script}\n", real.display()));
    assert!(script.contains(r#"do script "cd '"#));
    assert!(rig.lines("open.txt").is_empty(), "no fallback on success");
}

#[test]
fn automation_denied_falls_back_to_open_a_window() {
    let rig = Rig::new();
    let err = "execution error: Not authorized to send Apple events to Terminal. (-1743)";
    stub(&rig.osascript, &rig.record("osa.txt"), err, 1);
    let p = rig.terminal_plan(0);
    assert_eq!(
        execute_with(&p, &env(), &rig.runner()),
        Ok(Outcome::FellBack(Fallback::Denied(Permission::Automation)))
    );
    let real = rig.wt.canonicalize().unwrap();
    assert_eq!(
        rig.lines("open.txt"),
        [
            real.to_str().unwrap(),
            "-a",
            "/System/Applications/Utilities/Terminal.app",
            rig.wt.to_str().unwrap()
        ]
    );
}

#[test]
fn accessibility_denied_and_other_errors_also_fall_back() {
    let rig = Rig::new();
    let err = "System Events got an error: osascript is not allowed to send keystrokes. (1002)";
    stub(&rig.osascript, &rig.record("osa.txt"), err, 1);
    let p = rig.terminal_plan(0);
    assert_eq!(
        execute_with(&p, &env(), &rig.runner()),
        Ok(Outcome::FellBack(Fallback::Denied(
            Permission::Accessibility
        )))
    );

    stub(
        &rig.osascript,
        &rig.record("osa.txt"),
        "431:470: execution error: Terminal did not open a new tab (-2700)",
        1,
    );
    assert_eq!(
        execute_with(&p, &env(), &rig.runner()),
        Ok(Outcome::FellBack(Fallback::Failed(
            "Terminal did not open a new tab (-2700)".into()
        )))
    );
    assert_eq!(rig.lines("open.txt").len(), 4);
}

#[test]
fn failed_fallback_is_the_error() {
    let rig = Rig::new();
    stub(&rig.osascript, &rig.record("osa.txt"), "(-1743)", 1);
    let p = rig.terminal_plan(1);
    assert_eq!(
        execute_with(&p, &env(), &rig.runner()),
        Err(LaunchError::Exit {
            program: "open".into(),
            status: "exit status: 1".into(),
            stderr: "open: boom".into(),
        })
    );
}

#[test]
fn missing_worktree_runs_nothing() {
    let rig = Rig::new();
    stub(&rig.osascript, &rig.record("osa.txt"), "", 0);
    let p = rig.terminal_plan(0);
    fs::remove_dir(&rig.wt).unwrap();
    assert_eq!(
        execute_with(&p, &env(), &rig.runner()),
        Err(LaunchError::MissingFolder(rig.wt.clone()))
    );
    assert!(rig.lines("osa.txt").is_empty());
}

/// Spawn plans for terminals (Kitty CLI here) reach the program with the worktree as cwd.
#[test]
fn kitty_cli_plan_reaches_stub_with_directory() {
    let rig = Rig::new();
    let bin = rig.tmp.path().join("bin");
    stub(&bin.join("kitty"), &rig.record("kitty.txt"), "", 0);
    let env = ShellEnv {
        path: format!("{}:/usr/bin:/bin", bin.display()),
        ..env()
    };
    let targets = detect_in(&env, &[], false);
    let settings = Settings {
        default_terminal: Some("kitty".into()),
        terminal_opens_in: Some(TerminalOpensIn::Tab),
        ..Default::default()
    };
    let kitty = default_terminal(&settings, &targets).unwrap();
    assert_eq!(
        execute(&plan(kitty, &rig.wt, &settings), &env),
        Ok(Outcome::Opened)
    );
    let real = rig.wt.canonicalize().unwrap();
    assert_eq!(
        rig.lines("kitty.txt"),
        [
            real.to_str().unwrap(),
            "--single-instance",
            "--directory",
            rig.wt.to_str().unwrap()
        ]
    );
}

/// Rapid repeated opens run their scripts one after another, never interleaved.
#[test]
fn applescript_plans_never_overlap() {
    let rig = Rig::new();
    let log = rig.record("log.txt");
    fs::create_dir_all(rig.osascript.parent().unwrap()).unwrap();
    fs::write(
        &rig.osascript,
        format!(
            "#!/bin/sh\necho start >> '{0}'\nsleep 0.3\necho end >> '{0}'\n",
            log.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&rig.osascript, fs::Permissions::from_mode(0o755)).unwrap();
    let p = rig.terminal_plan(0);
    let runner = rig.runner();
    std::thread::scope(|s| {
        let runs: Vec<_> = (0..3)
            .map(|_| s.spawn(|| execute_with(&p, &env(), &runner)))
            .collect();
        for r in runs {
            assert_eq!(r.join().unwrap(), Ok(Outcome::Opened));
        }
    });
    assert_eq!(
        fs::read_to_string(&log).unwrap(),
        "start\nend\nstart\nend\nstart\nend\n"
    );
}

/// osascript still blocked (permission prompt) at the deadline: reported, no fallback.
#[test]
fn osascript_timeout_is_not_success_and_runs_no_fallback() {
    let rig = Rig::new();
    fs::create_dir_all(rig.osascript.parent().unwrap()).unwrap();
    fs::write(&rig.osascript, "#!/bin/sh\nsleep 10\n").unwrap();
    fs::set_permissions(&rig.osascript, fs::Permissions::from_mode(0o755)).unwrap();
    let p = rig.terminal_plan(0);
    let runner = Runner {
        osascript_wait: std::time::Duration::from_millis(200),
        ..rig.runner()
    };
    let start = std::time::Instant::now();
    assert_eq!(execute_with(&p, &env(), &runner), Ok(Outcome::TimedOut));
    // Other tests may hold the AppleScript lock briefly; still well under the stub's 10 s.
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
    assert!(rig.lines("open.txt").is_empty());
}

#[test]
fn compile_error_and_not_running_fall_back() {
    let rig = Rig::new();
    let p = rig.terminal_plan(0);
    stub(
        &rig.osascript,
        &rig.record("osa.txt"),
        "88:95: syntax error: Expected end of line but found identifier. (-2741)",
        1,
    );
    assert_eq!(
        execute_with(&p, &env(), &rig.runner()),
        Ok(Outcome::FellBack(Fallback::Unsupported(
            "Expected end of line but found identifier. (-2741)".into()
        )))
    );
    stub(
        &rig.osascript,
        &rig.record("osa.txt"),
        "0:130: execution error: Ghostty is not running (9001)",
        1,
    );
    assert_eq!(
        execute_with(&p, &env(), &rig.runner()),
        Ok(Outcome::FellBack(Fallback::NotRunning))
    );
    assert_eq!(rig.lines("open.txt").len(), 4);
}
