//! Detected open targets plus the executor that runs launch plans. Tests swap the executor
//! so nothing really spawns, and the presence check so fake paths count as present.

use std::path::Path;
use std::sync::Arc;

use async_channel::Receiver;
use gpui_kit::{App, AppContext, Global, Task};
use grove_core::{
    Fallback, LaunchError, LaunchPlan, OpenTarget, Outcome, Presence, Role, TargetKind, Terminal,
    default_target, plan,
};

use crate::main_window::tilde;
use crate::shell;

pub type LaunchResult = Result<Outcome, LaunchError>;
/// Starts a plan and returns where its result will arrive.
pub type Executor = Arc<dyn Fn(LaunchPlan) -> Receiver<LaunchResult> + Send + Sync>;
pub type PresenceCheck = Arc<dyn Fn(&Path) -> Presence + Send + Sync>;

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
}

impl Global for Launcher {}

/// Installs the real executor and detects editors and terminals in the background.
pub fn install(cx: &mut App) {
    cx.set_global(Launcher {
        targets: None,
        executor: threaded_executor(),
        presence: Arc::new(Presence::of),
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

/// Name of the app Enter (editor) or ⌘Enter (terminal) opens with, if there is one.
pub fn target_name(role: Role, cx: &App) -> Option<String> {
    target(role, cx).ok().map(|t| t.name().to_string())
}

fn target(role: Role, cx: &App) -> Result<&OpenTarget, String> {
    let launcher = cx.try_global::<Launcher>().ok_or("Launcher not ready")?;
    let targets = launcher
        .targets
        .as_deref()
        .ok_or("Still looking for apps… try again in a moment")?;
    default_target(role, &shell::library(cx).settings, targets).map_err(|e| e.to_string())
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

/// Why an open couldn't start, for the status bar.
pub struct OpenError {
    pub message: String,
    /// Fixable in settings (shows the settings link).
    pub settings: bool,
}

/// Starts opening `worktree` in the default app for `role`; the launch runs off the main
/// thread.
pub fn open(role: Role, worktree: &Path, cx: &mut App) -> Result<Opening, OpenError> {
    let target = target(role, cx).map_err(|message| OpenError {
        message,
        settings: true,
    })?;
    let p = plan(target, worktree, &shell::library(cx).settings);
    let (path, app, kind) = (tilde(worktree), target.name().to_string(), target.kind);
    let message = format!("Opening {path} in {app}…");
    let follow_up = Box::new(move |outcome: &Outcome| {
        let privacy =
            |pane: &str| format!("allow Grove in System Settings → Privacy & Security → {pane}");
        let window = format!("Opened {path} in a new {app} window");
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
                format!("{window}. For tabs, {}", privacy(perm.pane())),
                true,
            ),
            Outcome::FellBack(Fallback::Unsupported(e)) => match kind {
                TargetKind::Terminal(Terminal::Ghostty) => {
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
        rx.recv().await.unwrap_or_else(|_| {
            Err(LaunchError::Spawn {
                program: "launcher".into(),
                error: "launch thread ended without a result".into(),
            })
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
