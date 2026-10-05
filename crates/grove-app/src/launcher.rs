//! Detected open targets plus the executor that runs launch plans. Tests swap the executor
//! so nothing really spawns.

use std::path::Path;
use std::sync::Arc;

use gpui_kit::{App, AppContext, Global, Task};
use grove_core::{LaunchError, LaunchPlan, OpenTarget, default_editor, plan};

use crate::main_window::tilde;
use crate::shell;

pub type Executor = Arc<dyn Fn(LaunchPlan) -> Result<(), LaunchError> + Send + Sync>;

pub struct Launcher {
    /// `None` until detection finishes.
    pub targets: Option<Vec<OpenTarget>>,
    pub executor: Executor,
}

impl Global for Launcher {}

/// Installs the real executor and detects editors in the background.
pub fn install(cx: &mut App) {
    cx.set_global(Launcher {
        targets: None,
        executor: Arc::new(|plan| grove_core::execute(&plan, grove_core::resolved_env())),
    });
    cx.spawn(async move |cx| {
        let targets = cx.background_spawn(async { grove_core::detect() }).await;
        let names: Vec<&str> = targets.iter().map(OpenTarget::name).collect();
        eprintln!("[grove] editors detected: {names:?}");
        cx.update(|cx| {
            cx.global_mut::<Launcher>().targets = Some(targets);
            cx.refresh_windows();
        });
    })
    .detach();
}

pub fn ready(cx: &App) -> bool {
    cx.try_global::<Launcher>()
        .is_some_and(|l| l.targets.is_some())
}

/// Name of the editor Enter opens with, if there is one.
pub fn editor_name(cx: &App) -> Option<String> {
    editor(cx).ok().map(|t| t.name().to_string())
}

fn editor(cx: &App) -> Result<&OpenTarget, String> {
    let launcher = cx.try_global::<Launcher>().ok_or("Launcher not ready")?;
    let targets = launcher
        .targets
        .as_deref()
        .ok_or("Still looking for editors… try again in a moment")?;
    default_editor(&shell::library(cx).settings, targets).map_err(|e| e.to_string())
}

/// A launch in flight: the confirmation to show now and the task that finishes it.
pub struct Opening {
    pub message: String,
    pub done: Task<Result<(), LaunchError>>,
}

/// Why an open couldn't start, for the status bar.
pub struct OpenError {
    pub message: String,
    /// Fixable in settings (shows the settings link).
    pub settings: bool,
}

/// Starts opening `worktree` in the default editor; the launch runs off the main thread.
pub fn open_in_editor(worktree: &Path, cx: &mut App) -> Result<Opening, OpenError> {
    let target = editor(cx).map_err(|message| OpenError {
        message,
        settings: true,
    })?;
    let p = plan(target, worktree, &shell::library(cx).settings);
    let message = format!("Opening {} in {}…", tilde(worktree), target.name());
    let executor = cx.global::<Launcher>().executor.clone();
    eprintln!("[grove] launch: {p:?}");
    let done = cx.background_spawn(async move { executor(p) });
    Ok(Opening { message, done })
}
