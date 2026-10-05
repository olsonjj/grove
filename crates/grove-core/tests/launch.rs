//! Execution through a stub editor CLI: what actually reaches the process.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use grove_core::{
    LaunchPlan, PathSource, Settings, ShellEnv, default_editor, detect_in, execute, plan,
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
    let LaunchPlan::Spawn { program, .. } = &p;
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
