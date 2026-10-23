use std::path::{Path, PathBuf};

use gpui_kit::base::Button;
use gpui_kit::component::WindowExt;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AppContext, Context, FocusHandle, FontWeight, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, TestSupportExt, WeakEntity, Window, div, px,
};
use grove_core::{GitError, RemovalPlan, Worktree, read_removal_plan};

use crate::removal;
use crate::tree_view::OpenRequested;
use crate::{launcher, shell, theme};

use super::MainWindow;
use super::Status;
use super::sidebar::rows::tilde;

/// A worktree to remove, and its repo.
#[derive(Clone)]
pub(super) struct Target {
    repo: PathBuf,
    git_dir: PathBuf,
    worktree: Worktree,
}

impl Target {
    async fn read_plan(self) -> RemovalPlan {
        read_removal_plan(&self.git_dir, &self.worktree)
    }
}

#[derive(Clone)]
/// The remove-confirm dialog's state for one targeted worktree.
pub(super) struct Removal {
    target: Target,
    plan: RemovalPlan,
    confirm: removal::Confirm,
    running: bool,
    cancel: FocusHandle,
    ok: FocusHandle,
}

impl MainWindow {
    pub(super) fn set_status(
        &mut self,
        text: String,
        error: bool,
        settings: bool,
        cx: &mut Context<Self>,
    ) {
        self.status = Some(Status {
            text,
            error,
            settings,
        });
        cx.notify();
    }

    /// Launches one worktree and reports the outcome in the status bar.
    pub(super) fn open(&mut self, req: &OpenRequested, cx: &mut Context<Self>) {
        self.open_seq += 1;
        let seq = self.open_seq;
        match launcher::open_worktree(&req.path, req.presence, req.role, req.target.as_ref(), cx) {
            Err(e) => self.set_status(e.message, true, e.settings, cx),
            Ok(opening) => {
                self.set_status(opening.message.clone(), false, false, cx);
                cx.spawn(async move |this, cx| {
                    let Some(n) = opening.finish().await else {
                        return;
                    };
                    let _ = this.update(cx, |this, cx| {
                        if this.open_seq == seq {
                            this.set_status(n.text, n.warn, false, cx);
                        }
                    });
                })
                .detach();
            }
        }
    }

    /// Remove worktree… from the tree or details pane: reads the worktree fresh, then
    /// confirms.
    pub(super) fn request_removal(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.removal.is_some() {
            return;
        }
        let snapshot = shell::library(cx).store.snapshot().clone();
        let Some(target) = snapshot.repos.iter().find_map(|r| {
            let wt = r.worktrees.iter().find(|w| w.path == path)?;
            Some(Target {
                repo: r.repo.path.clone(),
                git_dir: r.repo.git_dir.clone(),
                worktree: wt.clone(),
            })
        }) else {
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            let plan = cx.background_spawn(target.clone().read_plan()).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.confirm_removal(target, plan, window, cx)
            });
        })
        .detach();
    }

    fn refuse_removal(&mut self, target: &Target, why: &str, cx: &mut Context<Self>) {
        let path = tilde(&target.worktree.path);
        self.open_seq += 1;
        self.set_status(format!("Can't remove {path}: {why}"), true, false, cx);
    }

    fn confirm_removal(
        &mut self,
        target: Target,
        plan: RemovalPlan,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.removal.is_some() {
            return;
        }
        let Some(confirm) = removal::confirm(&target.worktree, &plan) else {
            if let RemovalPlan::NotAllowed(why) = &plan {
                self.refuse_removal(&target, why, cx);
            }
            return;
        };
        let cancel = cx.focus_handle();
        self.removal = Some(Removal {
            target,
            plan,
            confirm,
            running: false,
            cancel: cancel.clone(),
            ok: cx.focus_handle(),
        });
        let main = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, cx| {
            let Some(r) = main.upgrade().and_then(|m| m.read(cx).removal.clone()) else {
                return alert;
            };
            let (on_ok, on_cancel) = (main.clone(), main.clone());
            alert
                .title(r.confirm.title.clone())
                .description(
                    div()
                        .id("remove-message")
                        .aria_label(r.confirm.message.clone())
                        .child(r.confirm.message.clone())
                        .test_support(),
                )
                .footer(removal_footer(&r, main.clone()))
                // The dialog binds Enter to OK; make it press the focused button instead.
                .on_ok(move |_, window, cx| {
                    on_ok
                        .update(cx, |this, cx| {
                            let Some(r) = &this.removal else { return true };
                            if r.cancel.is_focused(window) {
                                this.cancel_removal(cx)
                            } else {
                                if r.ok.is_focused(window) {
                                    this.start_removal(window, cx);
                                }
                                false
                            }
                        })
                        .unwrap_or(true)
                })
                .on_cancel(move |_, _, cx| {
                    on_cancel
                        .update(cx, |this, cx| this.cancel_removal(cx))
                        .unwrap_or(true)
                })
        });
        window.focus(&cancel, cx);
        cx.notify();
    }

    /// False while removal runs: it can't be stopped.
    fn cancel_removal(&mut self, cx: &mut Context<Self>) -> bool {
        if self.removal.as_ref().is_some_and(|r| r.running) {
            return false;
        }
        self.removal = None;
        cx.notify();
        true
    }

    /// Rereads the worktree first: if it changed since the confirm opened, the confirm shows
    /// the new plan instead of removing more than it said.
    fn start_removal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(r) = self.removal.as_mut().filter(|r| !r.running) else {
            return;
        };
        r.running = true;
        let target = r.target.clone();
        window.refresh();
        cx.spawn_in(window, async move |this, cx| {
            let plan = cx.background_spawn(target.clone().read_plan()).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.removal_checked(plan, window, cx)
            });
        })
        .detach();
    }

    fn removal_checked(&mut self, plan: RemovalPlan, window: &mut Window, cx: &mut Context<Self>) {
        let Some(r) = self.removal.as_mut() else {
            return;
        };
        if r.plan == plan {
            let (target, force) = (r.target.clone(), plan.force());
            return self.run_removal(target, force, window, cx);
        }
        match removal::confirm(&r.target.worktree, &plan) {
            Some(confirm) => {
                r.plan = plan;
                r.confirm = confirm;
                r.running = false;
                window.focus(&r.cancel.clone(), cx);
                window.refresh();
            }
            None => {
                let target = r.target.clone();
                self.removal = None;
                window.close_dialog(cx);
                if let RemovalPlan::NotAllowed(why) = &plan {
                    self.refuse_removal(&target, why, cx);
                }
            }
        }
    }

    /// Bookkeeping runs at app level, so closing the window mid-removal doesn't lose it.
    fn run_removal(
        &mut self,
        target: Target,
        force: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let run = removal::runner(cx);
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let (repo, path) = (target.repo.clone(), target.worktree.path.clone());
            let result = cx
                .background_spawn(async move { run(&repo, &path, force) })
                .await;
            if result.is_ok() {
                cx.update(|cx| shell::worktree_removed(&target.git_dir, &target.worktree.path, cx));
            }
            let _ = handle.update(cx, |_, window, cx| {
                this.update(cx, |this, cx| {
                    this.finish_removal(&target, result, window, cx)
                })
            });
        })
        .detach();
    }

    fn finish_removal(
        &mut self,
        target: &Target,
        result: Result<(), GitError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.removal.take().is_some() {
            window.close_dialog(cx);
        }
        let path = tilde(&target.worktree.path);
        let (text, error) = match result {
            Ok(()) => (format!("Removed {path}"), false),
            Err(e) => (format!("Couldn't remove {path}: {}", e.brief()), true),
        };
        self.open_seq += 1;
        self.set_status(text, error, false, cx);
    }
}

/// Cancel (focused first) and the confirm, which shows progress while removal runs.
fn removal_footer(r: &Removal, main: WeakEntity<MainWindow>) -> impl IntoElement {
    let cancel_main = main.clone();
    let (bg, fg, hover) = if r.confirm.destructive {
        (theme::warn_text(), theme::bg(), theme::warn_text())
    } else {
        (theme::accent(), theme::on_accent(), theme::accent_hover())
    };
    let button = |id: &'static str| {
        Button::new(id)
            .disabled(r.running)
            .h(px(30.))
            .px(px(14.))
            .rounded(px(7.))
            .border_1()
            .border_color(theme::border_strong())
            .focus_visible(|s| s.border_color(theme::link()))
            .text_size(px(13.))
            .when(!r.running, |b| b.cursor_pointer())
    };
    div()
        .flex()
        .justify_end()
        .gap(px(8.))
        .px(px(16.))
        .child(
            button("remove-cancel")
                .track_focus(&r.cancel)
                .accessibility_label("Cancel")
                .bg(theme::raised())
                .text_color(theme::text())
                .hover(|s| s.bg(theme::border()))
                .child("Cancel")
                .on_click(move |_, window, cx| {
                    if matches!(
                        cancel_main.update(cx, |this, cx| this.cancel_removal(cx)),
                        Ok(true)
                    ) {
                        window.close_dialog(cx);
                    }
                }),
        )
        .child(
            button("remove-confirm")
                .track_focus(&r.ok)
                .accessibility_label(if r.running {
                    "Removing…"
                } else {
                    r.confirm.ok
                })
                .bg(bg)
                .text_color(fg)
                .font_weight(FontWeight::SEMIBOLD)
                .hover(move |s| s.bg(hover))
                .child(if r.running {
                    "Removing…"
                } else {
                    r.confirm.ok
                })
                .on_click(move |_, window, cx| {
                    let _ = main.update(cx, |this, cx| this.start_removal(window, cx));
                }),
        )
}
