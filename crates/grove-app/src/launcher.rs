//! Detected open targets plus the executor that runs launch plans. Tests swap the executor
//! so nothing really spawns, the presence check so fake paths count as present, and reveal.

use std::path::Path;
use std::sync::Arc;

use async_channel::Receiver;
use gpui_kit::{App, AppContext, Global, Task};
use grove_core::{
    Fallback, LaunchError, LaunchPlan, OpenTarget, Opener, Outcome, Presence, Role, TargetKind,
    Terminal, default_opener, default_terminal, openers, plan, plan_command,
};

use crate::main_window::tilde;
use crate::shell;

pub type LaunchResult = Result<Outcome, LaunchError>;
/// Starts a plan and returns where its result will arrive.
pub type Executor = Arc<dyn Fn(LaunchPlan) -> Receiver<LaunchResult> + Send + Sync>;
pub type PresenceCheck = Arc<dyn Fn(&Path) -> Presence + Send + Sync>;
pub type Reveal = Arc<dyn Fn(&Path, &App) + Send + Sync>;

/// A receiver that already holds `result` (for executors that finish synchronously).
#[cfg(test)]
pub fn finished(result: LaunchResult) -> Receiver<LaunchResult> {
    let (tx, rx) = async_channel::bounded(1);
    let _ = tx.try_send(result);
    rx
}

pub struct Launcher {
    /// `None` until detection finishes.
    pub targets: Option<Vec<OpenTarget>>,
    pub executor: Executor,
    pub presence: PresenceCheck,
    /// Shows a folder in Finder.
    pub reveal: Reveal,
}

impl Global for Launcher {}

/// Installs the real executor and detects editors and terminals in the background.
pub fn install(cx: &mut App) {
    cx.set_global(Launcher {
        targets: None,
        executor: threaded_executor(),
        presence: Arc::new(Presence::of),
        reveal: Arc::new(|path, cx| cx.reveal_path(path)),
    });
    cx.spawn(async move |cx| {
        let targets = cx.background_spawn(async { grove_core::detect() }).await;
        let names: Vec<&str> = targets.iter().map(OpenTarget::name).collect();
        eprintln!("[grove] apps detected: {names:?}");
        cx.update(|cx| {
            cx.global_mut::<Launcher>().targets = Some(targets);
            cx.refresh_windows();
        });
    })
    .detach();
}

/// Own thread per launch: osascript can block for minutes on a permission prompt, which
/// would starve GPUI's background pool.
fn threaded_executor() -> Executor {
    Arc::new(|plan| {
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(grove_core::execute(&plan, grove_core::resolved_env()));
        });
        rx
    })
}

pub fn ready(cx: &App) -> bool {
    cx.try_global::<Launcher>()
        .is_some_and(|l| l.targets.is_some())
}

/// Whether `worktree`'s folder is there right now (a stat, cheap enough for the UI thread).
pub fn presence(worktree: &Path, cx: &App) -> Presence {
    match cx.try_global::<Launcher>() {
        Some(l) => (l.presence)(worktree),
        None => Presence::of(worktree),
    }
}

/// Reveals `path` in Finder.
pub fn reveal(path: &Path, cx: &App) {
    match cx.try_global::<Launcher>() {
        Some(l) => (l.reveal)(path, cx),
        None => cx.reveal_path(path),
    }
}

/// Name of the app or custom command Enter (editor) or ⌘Enter (terminal) opens with, if
/// there is one.
pub fn target_name(role: Role, cx: &App) -> Option<String> {
    target(role, None, cx).ok().map(|o| o.name().to_string())
}

/// A ▾ menu entry: a detected app or custom command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub id: String,
    pub name: String,
}

/// Every detected app for `role` in detection order, then custom commands (the ▾ menu),
/// each with whether it's the default.
pub fn choices(role: Role, cx: &App) -> Vec<(Choice, bool)> {
    let Some(targets) = cx
        .try_global::<Launcher>()
        .and_then(|l| l.targets.as_deref())
    else {
        return Vec::new();
    };
    let settings = &shell::library(cx).settings;
    let default = default_opener(role, settings, targets).ok().map(|o| o.id());
    openers(role, settings, targets)
        .iter()
        .map(|o| {
            let id = o.id();
            let is_default = default.as_ref() == Some(&id);
            let name = o.name().to_string();
            (Choice { id, name }, is_default)
        })
        .collect()
}

/// `chosen` if given (a one-off pick from the ▾ menu), else the default for `role`.
fn target<'a>(role: Role, chosen: Option<&Choice>, cx: &'a App) -> Result<Opener<'a>, String> {
    let launcher = cx.try_global::<Launcher>().ok_or("Launcher not ready")?;
    let targets = launcher
        .targets
        .as_deref()
        .ok_or("Still looking for apps… try again in a moment")?;
    let settings = &shell::library(cx).settings;
    match chosen {
        Some(choice) => openers(role, settings, targets)
            .into_iter()
            .find(|o| o.id() == choice.id)
            .ok_or_else(|| format!("{} isn't available anymore", choice.name)),
        None => default_opener(role, settings, targets).map_err(|e| e.to_string()),
    }
}

/// A launch in flight: the confirmation to show now and the task that finishes it.
pub struct Opening {
    pub message: String,
    /// Replaces `message` once the launch finishes, if there's more to say.
    pub follow_up: FollowUp,
    pub done: Task<LaunchResult>,
}

pub type FollowUp = Box<dyn Fn(&Outcome) -> Option<Notice> + Send>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
    /// Warn styling.
    pub warn: bool,
}

impl Notice {
    /// Posts this to the main window's status bar, which outlives the switcher and popover.
    pub fn post(&self, settings: bool, cx: &mut App) {
        let status = crate::main_window::Status {
            text: self.text.clone(),
            error: self.warn,
            settings,
        };
        shell::post(status, cx);
    }
}

impl Opening {
    /// Waits for the launch: what to show in place of `message`, if there's more to say.
    pub async fn finish(self) -> Option<Notice> {
        match self.done.await {
            Ok(outcome) => {
                eprintln!("[grove] launch: {outcome:?}");
                (self.follow_up)(&outcome)
            }
            Err(e) => {
                eprintln!("[grove] launch failed: {e}");
                Some(Notice {
                    text: e.to_string(),
                    warn: true,
                })
            }
        }
    }
}

/// Why an open couldn't start, for the status bar.
pub struct OpenError {
    pub message: String,
    /// Fixable in settings (shows the settings link).
    pub settings: bool,
}

/// Opens `worktree` unless its folder is gone, rechecking presence first: `scanned` was read
/// at scan time and the folder may have gone or come back since. Every open from any
/// surface comes through here, so each one is recorded to recents.
pub fn open_worktree(
    worktree: &Path,
    scanned: Presence,
    role: Role,
    chosen: Option<&Choice>,
    cx: &mut App,
) -> Result<Opening, OpenError> {
    let now = presence(worktree, cx);
    if now != scanned {
        cx.global_mut::<shell::Library>()
            .store
            .set_presence(worktree, now);
        if now == Presence::Present {
            shell::read_status(worktree, cx);
        }
    }
    if let Some(reason) = now.reason() {
        return Err(OpenError {
            message: format!("Can't open {}: {reason}", tilde(worktree)),
            settings: false,
        });
    }
    let opening = open(role, chosen, worktree, cx)?;
    shell::record_open(worktree, cx);
    Ok(opening)
}

/// Starts opening `worktree` in `chosen`, else the default app for `role`; the launch runs
/// off the main thread. A one-off choice never changes the default.
fn open(
    role: Role,
    chosen: Option<&Choice>,
    worktree: &Path,
    cx: &mut App,
) -> Result<Opening, OpenError> {
    let in_settings = |message| OpenError {
        message,
        settings: true,
    };
    let opener = target(role, chosen, cx).map_err(in_settings)?;
    let settings = &shell::library(cx).settings;
    let targets = cx
        .global::<Launcher>()
        .targets
        .as_deref()
        .unwrap_or_default();
    let path = tilde(worktree);
    let message = format!("Opening {path} in {}…", opener.name());
    // What the outcome is about: the app, or for a command the terminal it runs in.
    // `background` names a command run without a terminal, for its failures.
    let (p, app, kind, skipped, background) = match opener {
        Opener::App(t) => (
            plan(t, worktree, settings),
            t.name(),
            Some(t.kind),
            None,
            None,
        ),
        Opener::Command(c) => {
            let p = plan_command(c, worktree, settings, targets).map_err(|e| OpenError {
                message: e.to_string(),
                settings: e.in_settings(),
            })?;
            match default_terminal(settings, targets) {
                Ok(t) if c.run_in_terminal => {
                    // Their `open -a` fallback opens the folder only.
                    let skipped = matches!(
                        t.kind,
                        TargetKind::Terminal(Terminal::Terminal | Terminal::ITerm2)
                    )
                    .then(|| c.label.clone());
                    (p, t.name(), Some(t.kind), skipped, None)
                }
                _ => (p, c.label.as_str(), None, None, Some(c.label.clone())),
            }
        }
    };
    let app = app.to_string();
    let follow_up = Box::new(move |outcome: &Outcome| {
        let privacy =
            |pane: &str| format!("allow Grove in System Settings → Privacy & Security → {pane}");
        let mut window = format!("Opened {path} in a new {app} window");
        if let Some(label) = &skipped {
            window.push_str(" without running ");
            window.push_str(label);
        }
        let (text, warn) = match outcome {
            Outcome::Opened | Outcome::FellBack(Fallback::NotRunning) => return None,
            Outcome::TimedOut => (
                format!(
                    "Still waiting for permission to control {app}. To open it, {}",
                    privacy("Automation")
                ),
                true,
            ),
            Outcome::FellBack(Fallback::Denied(perm)) => (
                format!(
                    "{window}. For {}, {}",
                    if skipped.is_some() {
                        "commands"
                    } else {
                        "tabs"
                    },
                    privacy(perm.pane())
                ),
                true,
            ),
            Outcome::FellBack(Fallback::Unsupported(e)) => match kind {
                Some(TargetKind::Terminal(Terminal::Ghostty)) => {
                    (format!("{window} (tabs need Ghostty 1.3 or later)"), false)
                }
                _ => (format!("{window} (new tab failed: {e})"), true),
            },
            Outcome::FellBack(Fallback::Failed(e)) => {
                (format!("{window} (new tab failed: {e})"), true)
            }
        };
        Some(Notice { text, warn })
    });
    let executor = cx.global::<Launcher>().executor.clone();
    eprintln!("[grove] launch: {p:?}");
    let rx = executor(p);
    let done = cx.background_spawn(async move {
        let result = rx.recv().await.unwrap_or_else(|_| {
            Err(LaunchError::Spawn {
                program: "launcher".into(),
                error: "launch thread ended without a result".into(),
            })
        });
        // "Sublime failed: …" rather than "sh failed: …".
        result.map_err(|e| match (e, background) {
            (LaunchError::Exit { status, stderr, .. }, Some(label)) => LaunchError::Exit {
                program: label,
                status,
                stderr,
            },
            (e, _) => e,
        })
    });
    Ok(Opening {
        message,
        follow_up,
        done,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threaded_executor_delivers_result_from_its_own_thread() {
        let tmp = tempfile::TempDir::new().unwrap();
        let plan = |program: &str| LaunchPlan::Spawn {
            program: program.into(),
            args: vec![],
            cwd: tmp.path().into(),
        };
        let exec = threaded_executor();
        assert_eq!(
            exec(plan("/usr/bin/true")).recv_blocking().unwrap(),
            Ok(Outcome::Opened)
        );
        let err = exec(plan("/usr/bin/false")).recv_blocking().unwrap();
        assert_eq!(
            err.unwrap_err().to_string(),
            "false failed (exit status: 1)"
        );
    }
}
