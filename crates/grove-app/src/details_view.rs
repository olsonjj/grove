//! Right pane: everything about the tree's selection, plus open/copy/reveal actions.

use std::sync::Arc;

use gpui_kit::base::Button;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, App, AppContext, ClipboardItem, Context, Entity, EventEmitter, FocusHandle,
    FontWeight, InteractiveElement, IntoElement, KeyBinding, KeyDownEvent, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, TestSupportExt, Window,
    anchored, deferred, div, px,
};
use grove_core::{
    ChangedFile, Commit, Details, RepoSnapshot, Role, Snapshot, Worktree, cd_command,
    directory_size, recent_commits, relative_time, size_words, unix_now,
};

use grove_core::git::GitError;

use crate::branch::BranchMenu;
use crate::branch::{self, Offer};
use crate::icons::{self, icon};
use crate::launcher::Choice;
use crate::main_window::tilde;
use crate::pull;
use crate::removal::{self, RemoveRequested};
use crate::shell;
use crate::tree_view::{OpenRequested, TreeView};
use crate::{launcher, theme};

/// A confirmation or failure for the status bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
    pub error: bool,
}

gpui_kit::actions!(details, [LeaveMenu, LeaveMenuBack]);

pub(crate) const MENU_CONTEXT: &str = "OpenWithMenu";

/// What the trigger changes, for VoiceOver: the selected worktree's checked-out branch.
const SWITCH_LABEL: &str = "Switch the worktree's branch";

/// Tab out of an open menu (▾, a repo's context menu) closes it; this context outranks the window's own Tab.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", LeaveMenu, Some(MENU_CONTEXT)),
        KeyBinding::new("shift-tab", LeaveMenuBack, Some(MENU_CONTEXT)),
    ]);
}

pub struct DetailsView {
    tree: Entity<TreeView>,
    snapshot: Option<Arc<Snapshot>>,
    details: Option<Details>,
    commits: Commits,
    sizes: Sizes,
    /// The branch select at the pane's top, created lazily (its state needs a window).
    select: Option<Entity<SelectState<BranchMenu>>>,
    /// Subscriptions of `select`.
    _select_subs: Vec<Subscription>,
    /// The pane follows completed repo rereads, for the branch list.
    _repo_reads: Subscription,
    /// The branches read, per repo, re-read whenever the repo's snapshot is re-read, so a
    /// branch created in a terminal lands in the dropdown without any Grove action.
    branches: BranchList,
    /// The dropdown the pane last rendered, so `set_items` only runs on real changes.
    offers_shown: Option<Vec<Offer>>,
    /// The worktree identity and branch the select last showed, so the value updates only
    /// when it changes: (worktree path, committed branch; `None` is a detached worktree).
    branch_applied: Option<(std::path::PathBuf, Option<String>)>,
    /// The worktree being switched, while its switch runs; a selection change doesn't
    /// cancel it. One switch at a time per pane; the trigger reads "Switching…" until
    /// the write resolves.
    switching: Option<Switching>,
    /// The worktree being pulled, while its pull runs; a selection change doesn't cancel it.
    pulling: Option<std::path::PathBuf>,
    /// Which ▾ menu is open.
    menu: Option<Role>,
    /// The ▾ buttons (editor, terminal), for focus to return to.
    carets: [FocusHandle; 2],
    /// The open menu's items, for arrow keys; grown as needed.
    items: Vec<FocusHandle>,
    /// The open menu itself, focused when it has no items so Esc still closes it.
    menu_focus: FocusHandle,
    _sub: Subscription,
}

/// Commits are read per selection, not with every status read: only this pane shows them,
/// so reading them for every worktree on every refresh would double git's work for nothing.
#[derive(Default)]
struct Commits {
    /// The worktree identity and HEAD used for the read.
    source: Option<CommitSource>,
    result: Option<Result<Vec<Commit>, GitError>>,
    /// Bumped per read so a slow read for an old selection can't land.
    seq: u64,
}

/// Directory sizes are read per selection, like commits: only this pane shows one, and
/// walking a big worktree off every status refresh would spend real time for nothing.
/// Reads run single-flight per path (walks can't be cancelled): a key change during a
/// walk rides out the flight, shows its bytes, and follows with one trailing read.
/// Returning to a worktree mid-walk starts a second walk of the same key; the first
/// walk's landing is discarded by id, so the fresh walk wins.
#[derive(Debug, Default)]
struct Sizes {
    /// The size key the pane wants now.
    want: Option<SizeSource>,
    /// The walk in flight, its key and id: returning to a worktree mid-walk can start a
    /// second walk of the same key, and only the newer may land.
    flight: Option<(SizeSource, u64)>,
    /// The key and bytes of the last landed walk, shown for the same path while `want` is
    /// unmet — the stale bytes are the best known size until the fresh walk lands.
    shown: Option<(SizeSource, u64)>,
    /// Bumped per walk started; identifies landings.
    next: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SizeSource {
    path: std::path::PathBuf,
    /// The checked-out commit; a checkout changes the tree as surely as an edit does.
    head: Option<String>,
    /// The status read's newest changes mtime; a plain re-read reuses the shown size.
    changes_time: Option<i64>,
}

/// The branch list is read per repo, like commits: the dropdown is repo-wide, so the read
/// keys on the repo and re-reads whenever that repo's snapshot is re-read — the watcher's
/// ref and HEAD hits already produce those, which keeps a branch created in a terminal on
/// the list within the usual 2 s, without Grove's involvement.
#[derive(Default)]
struct BranchList {
    /// The selected repo's name.
    source: Option<String>,
    result: Option<Result<Vec<String>, GitError>>,
    /// Bumped per read so a slow read for an old selection can't land.
    seq: u64,
}

/// The switch write in flight: which worktree, to what branch, and the repo's git dir
/// for the single-repo refresh that follows a success.
#[derive(Debug)]
struct Switching {
    path: std::path::PathBuf,
    target: String,
    git_dir: std::path::PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommitSource {
    path: std::path::PathBuf,
    head: Option<String>,
}

impl EventEmitter<OpenRequested> for DetailsView {}
impl EventEmitter<Notice> for DetailsView {}
impl EventEmitter<RemoveRequested> for DetailsView {}

impl DetailsView {
    pub fn new(tree: Entity<TreeView>, cx: &mut Context<Self>) -> Self {
        let sub = cx.observe(&tree, |this, _, cx| this.sync(cx));
        let mut this = Self {
            tree,
            snapshot: None,
            details: None,
            commits: Commits::default(),
            sizes: Sizes::default(),
            select: None,
            _select_subs: Vec::new(),
            branches: BranchList::default(),
            offers_shown: None,
            branch_applied: None,
            switching: None,
            pulling: None,
            menu: None,
            carets: [cx.focus_handle(), cx.focus_handle()],
            items: Vec::new(),
            menu_focus: cx.focus_handle(),
            _sub: sub,
            _repo_reads: cx.observe_global::<shell::RepoReads>(|this, cx| this.repo_reread(cx)),
        };
        this.sync(cx);
        this
    }

    #[cfg(test)]
    pub fn details(&self) -> Option<&Details> {
        self.details.as_ref()
    }

    #[cfg(test)]
    pub fn status_fields(&self) -> Vec<(&'static str, String)> {
        self.worktree()
            .map_or_else(Vec::new, |_| self.status_rows())
    }

    /// `None` while reading.
    #[cfg(test)]
    pub fn commits(&self) -> Option<&Result<Vec<Commit>, GitError>> {
        self.commits.result.as_ref()
    }

    /// The STATUS section's rows, Size included: what the section renders from, and
    /// what the tests assert.
    fn status_rows(&self) -> Vec<(&'static str, String)> {
        let Some(wt) = self.worktree() else {
            return Vec::new();
        };
        let mut fields = status_fields(wt);
        if wt.presence.is_openable()
            && let Some(Ok(_)) = wt.status.as_ref()
        {
            fields.push(("Size", self.size_line()));
        }
        fields
    }

    /// The switch write in flight: which worktree, to what branch.
    #[cfg(test)]
    pub fn switching(&self) -> Option<(&std::path::Path, &str)> {
        self.switching
            .as_ref()
            .map(|s| (s.path.as_path(), s.target.as_str()))
    }

    /// Puts a switch in flight for a test, without running a write — exactly the state
    /// the pick handler leaves after its synchronous part, so the trigger's refusal is
    /// testable without waiting on an async write.
    #[cfg(test)]
    pub fn hold_switch(
        &mut self,
        path: std::path::PathBuf,
        target: String,
        git_dir: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.switching = Some(Switching {
            git_dir,
            target,
            path: path.clone(),
        });
        // The trigger reads "Switching…" (no committed value) while the write runs.
        self.branch_applied = Some((path, None));
        let Some(select) = self.select.clone() else {
            return;
        };
        select.update(cx, |state, cx| state.set_selected_index(None, window, cx));
        cx.notify();
    }

    /// What the trigger reads right now: the committed branch, else the placeholder,
    /// by the same rule [`Self::branch_switch`] renders with.
    #[cfg(test)]
    pub fn branch_trigger_text(&self, cx: &App) -> Option<String> {
        let select = self.select.as_ref()?;
        match select.read(cx).selected_value() {
            Some(value) => Some(value.to_string()),
            None => {
                let displayed = self.details.as_ref().and_then(|d| d.worktree.clone());
                let switching_this = displayed.is_some_and(|wt| self.switching_is_path(&wt));
                Some(
                    if switching_this {
                        "Switching…"
                    } else {
                        "No branch"
                    }
                    .to_string(),
                )
            }
        }
    }

    fn sync(&mut self, cx: &mut Context<Self>) {
        let tree = self.tree.read(cx);
        let details = tree
            .selected()
            .zip(tree.repo())
            .and_then(|(id, repo)| repo.details(id));
        let snapshot = tree.snapshot().cloned();
        let same_snapshot = self
            .snapshot
            .as_ref()
            .zip(snapshot.as_ref())
            .is_some_and(|(old, new)| Arc::ptr_eq(old, new));
        if details == self.details && same_snapshot {
            return;
        }
        if details.as_ref().and_then(|d| d.worktree.as_ref())
            != self.details.as_ref().and_then(|d| d.worktree.as_ref())
        {
            self.menu = None;
        }
        self.snapshot = snapshot;
        self.details = details;
        self.read_commits(cx);
        self.read_sizes(cx);
        self.sync_branches(!same_snapshot, cx);
        cx.notify();
    }

    /// The branch list follows its repo: read per shown repo, re-read whenever the
    /// repo's snapshot is re-read, and whenever a repo reread announces itself (a branch
    /// created or deleted in a terminal changes the list but not the snapshot).
    fn sync_branches(&mut self, repo_reread: bool, cx: &mut Context<Self>) {
        let source = self.repo_shown().map(|r| r.repo.name.clone());
        if source != self.branches.source {
            self.branches.result = None;
            self.offers_shown = None;
        }
        self.branches.source = source;
        if self.branches.source.is_none() {
            return;
        }
        if repo_reread || self.branches.result.is_none() {
            self.read_branch_list(cx);
        }
    }

    /// A completed repo reread anywhere: for the shown repo, its branch list may have
    /// moved while the snapshot stayed, so read it again.
    fn repo_reread(&mut self, cx: &mut Context<Self>) {
        let Some(shown) = self.branches.source.clone() else {
            return;
        };
        let Some(reads) = cx.try_global::<shell::RepoReads>() else {
            return;
        };
        if reads.repo.as_deref() != Some(shown.as_str()) {
            return;
        }
        self.read_branch_list(cx);
    }

    /// (Re)reads the shown repo's branch list off the UI thread.
    fn read_branch_list(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.repo_shown().map(|repo| repo.repo.path.clone()) else {
            return;
        };
        self.branches.seq += 1;
        let seq = self.branches.seq;
        let read = branch::list_runner(cx);
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { read(&path) }).await;
            let _ = this.update(cx, |this, cx| {
                if this.branches.seq == seq {
                    this.branches.result = Some(result);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The snapshot's repo holding the pane's selected worktree.
    fn repo_shown(&self) -> Option<&RepoSnapshot> {
        let wt = self.details.as_ref()?.worktree.as_ref()?;
        self.repo_of(Some(wt))
    }

    /// The snapshot's repo holding the pane's selected worktree.
    fn repo_of(&self, wt: Option<&std::path::Path>) -> Option<&RepoSnapshot> {
        let wt = wt?;
        self.snapshot
            .as_ref()?
            .repos
            .iter()
            .find(|repo| repo.worktrees.iter().any(|w| w.path == wt))
    }

    fn worktree(&self) -> Option<&Worktree> {
        let path = self.details.as_ref()?.worktree.as_ref()?;
        self.snapshot
            .as_ref()?
            .repos
            .iter()
            .flat_map(|repo| &repo.worktrees)
            .find(|worktree| &worktree.path == path)
    }

    fn read_commits(&mut self, cx: &mut Context<Self>) {
        let source = self
            .worktree()
            .filter(|w| w.presence.is_openable())
            .map(|worktree| CommitSource {
                path: worktree.path.clone(),
                head: worktree.head.clone(),
            });
        if source == self.commits.source {
            return;
        }
        if source.as_ref().map(|source| &source.path)
            != self.commits.source.as_ref().map(|source| &source.path)
        {
            self.commits.result = None;
        }
        self.commits.seq += 1;
        let seq = self.commits.seq;
        let path = source.as_ref().map(|source| source.path.clone());
        self.commits.source = source;
        let Some(path) = path else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { recent_commits(&path) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.commits.seq == seq {
                    this.commits.result = Some(result);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The size key the pane wants: which worktree, at which HEAD and changes mtime — a
    /// checkout or an edit changes the tree, so either re-reads. `None` (and so no read)
    /// while the status hasn't landed or failed: no Size row is shown then either, and
    /// waiting avoids a throwaway walk before the status's mtime is known.
    fn size_source(&self) -> Option<SizeSource> {
        let wt = self.worktree().filter(|w| w.presence.is_openable())?;
        let status = match wt.status.as_ref() {
            Some(Ok(s)) => s,
            _ => return None,
        };
        Some(SizeSource {
            path: wt.path.clone(),
            head: wt.head.clone(),
            changes_time: status.changes_time,
        })
    }

    /// (Re)reads the shown worktree's directory size off the UI thread.
    fn read_sizes(&mut self, cx: &mut Context<Self>) {
        let want = self.size_source();
        if want == self.sizes.want {
            return;
        }
        self.sizes.want = want;
        self.sync_sizes(cx);
    }

    /// Starts the wanted read unless that path's walk is already running; the walk's
    /// landing re-checks and re-reads once, so a key change during a walk costs one
    /// trailing read, never another concurrent walk.
    fn sync_sizes(&mut self, cx: &mut Context<Self>) {
        let Some(want) = self.sizes.want.clone() else {
            return;
        };
        if self
            .sizes
            .flight
            .as_ref()
            .is_some_and(|(f, _)| f.path == want.path)
        {
            return;
        }
        self.sizes.next += 1;
        let id = self.sizes.next;
        self.sizes.flight = Some((want.clone(), id));
        let path = want.path.clone();
        cx.spawn(async move |this, cx| {
            let bytes = cx
                .background_spawn(async move { directory_size(&path) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if !matches!(
                    &this.sizes.flight,
                    Some((f, fid)) if f == &want && *fid == id
                ) {
                    return; // A newer walk took over; its own landing lands.
                }
                this.sizes.flight = None;
                if this
                    .sizes
                    .want
                    .as_ref()
                    .is_some_and(|w| w.path == want.path)
                {
                    // The key moved during the walk: show these bytes anyway (closest
                    // known) and follow with one trailing read.
                    let moved = this.sizes.want.as_ref() != Some(&want);
                    this.sizes.shown = Some((want.clone(), bytes));
                    cx.notify();
                    if moved {
                        this.sync_sizes(cx);
                    }
                }
            });
        })
        .detach();
    }

    /// What Size shows while the wanted read hasn't landed: the pane's usual placeholder
    /// until this worktree's first walk lands; after that, the last known size keeps
    /// showing until a fresh walk lands — a landing for a moved key keeps the row alive
    /// across the trailing read, and repeated edits (each re-keying the walk) can't hold
    /// the row on the placeholder. A different worktree shows only its own walks.
    fn size_line(&self) -> String {
        match (&self.sizes.want, &self.sizes.shown) {
            (Some(want), Some((shown, bytes))) if shown.path == want.path => size_words(*bytes),
            _ => "reading…".into(),
        }
    }

    fn open(
        &mut self,
        role: Role,
        target: Option<Choice>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.menu = None;
        let Some(wt) = self.worktree() else {
            return;
        };
        let pick = target.is_some();
        let req = OpenRequested {
            path: wt.path.clone(),
            presence: wt.presence,
            role,
            target,
        };
        // A default that's gone falls back to the ▾ menu (the status bar says why).
        if !pick && launcher::ready(cx) && launcher::target_name(role, cx).is_none() {
            self.open_menu(role, window, cx);
        } else if pick {
            window.focus(&self.carets[role_ix(role)], cx);
        }
        cx.emit(req);
        cx.notify();
    }

    /// Opens `role`'s ▾ menu with its first item focused, so arrows and Enter work at once.
    fn open_menu(&mut self, role: Role, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = Some(role);
        let count = launcher::choices(role, cx).len();
        while self.items.len() < count {
            self.items.push(cx.focus_handle());
        }
        let first = if count == 0 {
            &self.menu_focus
        } else {
            &self.items[0]
        };
        window.focus(first, cx);
        cx.notify();
    }

    fn close_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(role) = self.menu.take() {
            window.focus(&self.carets[role_ix(role)], cx);
            cx.notify();
        }
    }

    fn on_menu_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let count = self
            .menu
            .map_or(0, |role| launcher::choices(role, cx).len());
        let at = self.items.iter().position(|h| h.is_focused(window));
        let next = match (ev.keystroke.key.as_str(), at) {
            ("escape", _) => {
                self.close_menu(window, cx);
                return cx.stop_propagation();
            }
            ("down", Some(i)) => (i + 1).min(count.saturating_sub(1)),
            ("up", Some(i)) => i.saturating_sub(1),
            ("down" | "up", None) => 0,
            _ => return,
        };
        if next < count {
            window.focus(&self.items[next], cx);
        }
        cx.stop_propagation();
    }

    fn notice(&self, text: String, error: bool, cx: &mut Context<Self>) {
        cx.emit(Notice { text, error });
    }

    fn copy_path(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.worktree().map(|w| w.path.clone()) else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(path.display().to_string()));
        self.notice(format!("Copied path {}", tilde(&path)), false, cx);
    }

    fn copy_cd(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.worktree().map(|w| w.path.clone()) else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(cd_command(&path)));
        self.notice(format!("Copied cd command for {}", tilde(&path)), false, cx);
    }

    fn reveal(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.worktree().map(|w| w.path.clone()) else {
            return;
        };
        match launcher::presence(&path, cx).reason() {
            Some(reason) => {
                self.notice(format!("Can't reveal {}: {reason}", tilde(&path)), true, cx);
            }
            None => {
                launcher::reveal(&path, cx);
                self.notice(format!("Revealed {} in Finder", tilde(&path)), false, cx);
            }
        }
    }

    /// The Sync row's Pull: fast-forwards the selected worktree, one pull at a time. Runs
    /// off the UI thread; a selection change mid-pull doesn't cancel it and its notice, and
    /// git's refusal still lands through the status bar. Pull waits out a running switch,
    /// as the switcher waits out a running pull (pulls and checkouts never collide).
    fn pull(&mut self, cx: &mut Context<Self>) {
        let Some(wt) = self.worktree().filter(|wt| pull::offer(wt).is_some()) else {
            return;
        };
        if self.pulling.is_some() || self.switching.is_some() {
            return; // The disabled button already refuses it; this is the belt and braces.
        }
        // `pull::offer` guarantees the worktree is on a branch, so its name is here.
        let Some(branch) = wt.branch.clone() else {
            return;
        };
        let path = wt.path.clone();
        self.pulling = Some(path.clone());
        let run = pull::runner(cx);
        let read_path = path.clone();
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { run(&path) }).await;
            let _ = this.update(cx, |this, cx| {
                this.pulling = None;
                let (text, error) = match result {
                    Ok(()) => {
                        // Rereads just this worktree so its badge and the sync line catch
                        // up; recent commits re-read on their own, as HEAD changed.
                        shell::reread_status(&read_path, cx);
                        (format!("Pulled {branch}"), false)
                    }
                    Err(e) => (format!("Couldn't pull {branch}: {}", e.brief()), true),
                };
                this.notice(text, error, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// The top branch line: the selected worktree's branch (or a placeholder), drawn by
    /// the component select. The dropdown lists the repo's local branches, searchable,
    /// alphabetically, with the current one check-marked in place and a branch another
    /// worktree holds grayed with that worktree's folder. A running switch reads
    /// "Switching…" and is disabled — the pane runs one write (switch or pull) at a
    /// time, and a selection change during one doesn't cancel it (its notice still
    /// lands), like pull's. A failed branch-list read keeps the worktree's own branch as
    /// the value and reports itself in the dropdown rather than reading as an empty
    /// repository.
    fn branch_switch(
        &mut self,
        wt: &Worktree,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let select = match self.select.as_ref() {
            Some(select) => select.clone(),
            None => {
                let state = cx.new(|cx| {
                    SelectState::new(BranchMenu::default(), None, window, cx).searchable(true)
                });
                let sub = cx.subscribe_in(
                    &state,
                    window,
                    |this, _, ev: &SelectEvent<BranchMenu>, window, cx| {
                        this.on_branch_pick(ev, window, cx);
                    },
                );
                self._select_subs.push(sub);
                self.select = Some(state.clone());
                state
            }
        };

        // The dropdown's items: the repo's read branch list joined with the snapshot's
        // worktrees; a failed read falls back to the worktree's own branch, so the
        // trigger keeps naming it while the list is unavailable. Derived per render;
        // `set_items` only on real changes.
        let offers = match &self.branches.result {
            Some(Ok(list)) => {
                let worktrees = self
                    .repo_of(Some(&wt.path))
                    .map_or(&[][..], |repo| repo.worktrees.as_slice());
                branch::items(&wt.path, list, worktrees)
            }
            Some(Err(_)) => self.fallback_offers(wt),
            _ => Vec::new(),
        };
        if self.offers_shown.as_ref() != Some(&offers) {
            let next = offers.clone();
            select.update(cx, |state, cx| {
                state.set_items(BranchMenu::new(next), window, cx)
            });
            self.offers_shown = Some(offers);
            // The value's position lives in the delegate, so changing the items means the
            // value needs committing again.
            self.branch_applied = None;
        }

        let (placeholder, disabled) = self.trigger_state(wt);
        if !self.switching_is(wt) {
            self.apply_branch(wt, window, cx);
        }
        let read_failed = matches!(&self.branches.result, Some(Err(_)));
        Select::new(&select)
            .id("details-branch")
            .placeholder(placeholder)
            .accessibility_label(SWITCH_LABEL)
            .when(disabled, |s| s.disabled(true))
            .empty(move |_, _| {
                // A failed read is its own reason, not an empty repo's "No branches".
                let text = if read_failed {
                    "Couldn't list branches"
                } else {
                    "No branches"
                };
                div()
                    .id("branch-empty")
                    .test_support()
                    .aria_label(text)
                    .flex()
                    .w_full()
                    .justify_center()
                    .py(px(10.))
                    .text_color(theme::muted())
                    .text_size(px(12.))
                    .child(text)
            })
            .text_size(px(13.))
            .into_any_element()
    }

    /// What the trigger reads when it has no committed value, and whether picks are on:
    /// "Switching…" while this worktree's switch runs, else the no-branch placeholder;
    /// picks are off while any write — switch or pull — runs (one write at a time per
    /// pane) and for a worktree that isn't on disk.
    fn trigger_state(&self, wt: &Worktree) -> (&'static str, bool) {
        let switching_this = self.switching_is(wt);
        (
            if switching_this {
                "Switching\u{2026}"
            } else {
                "No branch"
            },
            self.switching.is_some() || self.pulling.is_some() || !wt.presence.is_openable(),
        )
    }

    fn switching_is(&self, wt: &Worktree) -> bool {
        self.switching.as_ref().is_some_and(|s| s.path == wt.path)
    }

    /// The offers a failed branch-list read still allows: the worktree's own branch, so
    /// the trigger keeps it as the value while the list is unavailable.
    fn fallback_offers(&self, wt: &Worktree) -> Vec<Offer> {
        let Some(branch) = wt.branch.as_deref() else {
            return Vec::new();
        };
        let worktrees = self
            .repo_of(Some(&wt.path))
            .map_or(&[][..], |repo| repo.worktrees.as_slice());
        branch::items(&wt.path, &[branch.to_string()], worktrees)
    }

    /// Same, by the worktree's path alone.
    #[cfg(test)]
    fn switching_is_path(&self, path: &std::path::Path) -> bool {
        self.switching.as_ref().is_some_and(|s| s.path == path)
    }

    /// Seeds the state an edit leaves while a size walk rides out — `want` re-keyed on
    /// the status's fresh mtime, the last landed walk still `shown` — and starts the
    /// trailing walk, so the row's behavior across a pending read is testable without
    /// waiting on an async walk.
    #[cfg(test)]
    pub fn hold_size_walk(&mut self, cx: &mut Context<Self>) {
        let Some(mut want) = self.size_source() else {
            return;
        };
        // Far past any real mtime, so the key has moved however the status was read.
        want.changes_time = Some(unix_now() + 100_000);
        self.sizes.want = Some(want);
        self.sync_sizes(cx);
    }

    /// Commits the displayed worktree's branch into the select, when it changed; commit
    /// happens in render, where a window is at hand.
    fn apply_branch(&mut self, wt: &Worktree, window: &mut Window, cx: &mut Context<Self>) {
        let desired = (wt.path.clone(), wt.branch.clone());
        if self.branch_applied.as_ref() == Some(&desired) {
            return;
        }
        let Some(select) = self.select.clone() else {
            return;
        };
        let branch = desired.1.clone();
        select.update(cx, |state, cx| match branch {
            Some(b) => state.set_selected_value(&b, window, cx),
            None => state.set_selected_index(None, window, cx),
        });
        self.branch_applied = Some(desired);
    }

    /// Shows the displayed worktree's branch or placeholder again, after a refused pick.
    fn show_current_branch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.branch_applied = None;
        if let Some(wt) = self.worktree().cloned() {
            self.apply_branch(&wt, window, cx);
        }
        cx.notify();
    }

    /// A confirmed pick: the current branch is a harmless no-op switch; a branch another
    /// worktree holds is refused before git (the picker already grayed it; git's own
    /// refusal remains the backstop for races); anything else starts the write.
    fn on_branch_pick(
        &mut self,
        ev: &SelectEvent<BranchMenu>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(Some(target)) = ev else {
            return;
        };
        let target = target.clone();
        let Some(wt) = self.worktree().cloned() else {
            return;
        };
        if self.switching.is_some() || self.pulling.is_some() || !wt.presence.is_openable() {
            return self.show_current_branch(window, cx);
        }
        let Some(repo) = self.repo_of(Some(&wt.path)) else {
            return;
        };
        if branch::held_by(&repo.worktrees, &target, &wt.path).is_some() {
            return self.show_current_branch(window, cx);
        }
        let git_dir = repo.repo.git_dir.clone();
        let path = wt.path.clone();
        // The trigger reads "Switching…" (no committed value) while the write runs.
        // Guard on the select before committing switching state, so a missing select
        // falls back without leaving branch switching disabled.
        let Some(select) = self.select.clone() else {
            return self.show_current_branch(window, cx);
        };
        self.switching = Some(Switching {
            path: path.clone(),
            target: target.clone(),
            git_dir: git_dir.clone(),
        });
        self.branch_applied = Some((path.clone(), None));
        select.update(cx, |state, cx| state.set_selected_index(None, window, cx));
        cx.notify();
        let run = branch::switch_runner(cx);
        let run_path = path.clone();
        let run_target = target.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { run(&run_path, &run_target) })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(s) = this.switching.take() else {
                    return;
                };
                let (text, error) = match result {
                    Ok(()) => {
                        // The single-repo refresh: the worktree re-nests under its new
                        // branch node, badges and recent commits update, the cache saves.
                        shell::reread_repo(&s.git_dir, cx);
                        // The committed value was cleared for the run, so the marker
                        // goes with it: the next render re-applies from the worktree
                        // (which the reread brings to the new branch). Keeping the
                        // target here would leave a no-op switch's trigger on the
                        // placeholder, since the worktree's branch never changed.
                        this.branch_applied = None;
                        (format!("Switched to `{}`", s.target), false)
                    }
                    Err(e) => {
                        this.branch_applied = None;
                        (
                            format!("Couldn't switch to `{}`: {}", s.target, e.brief()),
                            true,
                        )
                    }
                };
                this.notice(text, error, cx);
                cx.notify();
            });
        })
        .detach();
    }
}

const fn role_ix(role: Role) -> usize {
    match role {
        Role::Editor => 0,
        Role::Terminal => 1,
    }
}

const fn role_key(role: Role) -> &'static str {
    match role {
        Role::Editor => "editor",
        Role::Terminal => "terminal",
    }
}

impl DetailsView {
    /// The Sync row: its value, plus Pull when the worktree is behind its upstream. A
    /// running pull reads "Pulling…" and refuses a second click, and Pull waits out a
    /// running switch: the pane runs one git write at a time, so pull's fast-forward and
    /// a concurrent checkout never collide on the same worktree.
    fn sync_field(&self, value: String, cx: &mut Context<Self>) -> impl IntoElement {
        let pulling = self
            .pulling
            .as_ref()
            .is_some_and(|p| self.details.as_ref().and_then(|d| d.worktree.as_ref()) == Some(p));
        let enabled = self.pulling.is_none() && self.switching.is_none();
        let label = if pulling { "Pulling…" } else { "Pull" };
        div()
            .flex()
            .gap(px(10.))
            .text_size(px(13.))
            .child(
                div()
                    .w(px(92.))
                    .flex_none()
                    .text_color(theme::muted())
                    .child(SYNC),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .min_w_0()
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_color(theme::text_soft())
                            .child(value),
                    )
                    .child(
                        Button::new("details-pull")
                            .accessibility_label(label)
                            .disabled(!enabled)
                            .h(px(24.))
                            .px(px(8.))
                            .rounded(px(6.))
                            .border_1()
                            .border_color(theme::border_strong())
                            .focus_visible(|s| s.border_color(theme::link()))
                            .bg(theme::raised())
                            .text_size(px(12.))
                            .when_else(
                                enabled,
                                |b| {
                                    b.text_color(theme::text())
                                        .cursor_pointer()
                                        .hover(|s| s.bg(theme::border()))
                                },
                                |b| b.text_color(theme::faint()),
                            )
                            .child(label)
                            .on_click(cx.listener(|this, _, _, cx| this.pull(cx))),
                    ),
            )
    }

    /// "Open in <default>" plus its ▾ menu of every detected app for `role`.
    fn open_split(&self, role: Role, openable: bool, cx: &mut Context<Self>) -> AnyElement {
        let key = role_key(role);
        let label = match launcher::target_name(role, cx) {
            Some(name) => format!("Open in {name}"),
            None => format!("Open in {key}"),
        };
        let primary = role == Role::Editor;
        let (bg, fg, hover) = match (primary, openable) {
            (_, false) => (theme::raised(), theme::faint(), theme::raised()),
            (true, true) => (theme::accent(), theme::on_accent(), theme::accent_hover()),
            (false, true) => (theme::raised(), theme::text(), theme::border()),
        };
        let part = |id: SharedString| {
            Button::new(id)
                .disabled(!openable)
                .h(px(32.))
                .text_size(px(13.))
                .bg(bg)
                .text_color(fg)
                .border_1()
                .when_else(
                    !primary || !openable,
                    |b| b.border_color(theme::border_strong()),
                    |b| b.border_color(gpui_kit::transparent_black()),
                )
                .focus_visible(|s| s.border_color(theme::link()))
                .when(openable, |d| d.cursor_pointer().hover(|s| s.bg(hover)))
        };
        let menu_open = openable && self.menu == Some(role);
        let main = part(format!("details-open-{key}").into())
            .flex_1()
            .min_w_0()
            .justify_start()
            .gap(px(8.))
            .px(px(12.))
            .rounded_l(px(8.))
            .aria_keyshortcuts(if primary { "Enter" } else { "Command+Enter" })
            .when(primary, |d| d.font_weight(FontWeight::SEMIBOLD))
            .child(
                icon(
                    if primary {
                        icons::EDITOR
                    } else {
                        icons::TERMINAL
                    },
                    14.,
                )
                .text_color(fg),
            )
            .child(div().min_w_0().truncate().child(label))
            .on_click(cx.listener(move |this, _, window, cx| this.open(role, None, window, cx)));
        let caret = part(format!("details-menu-{key}").into())
            .track_focus(&self.carets[role_ix(role)])
            .accessibility_label(format!("Open in another {key}"))
            .aria_expanded(menu_open)
            .flex_none()
            .w(px(30.))
            .rounded_r(px(8.))
            .border_l_1()
            .border_color(if primary && openable {
                theme::on_accent_divider()
            } else {
                theme::border_strong()
            })
            .child(icon(icons::CHEVRON_DOWN, 13.).text_color(fg))
            .on_click(cx.listener(move |this, _, window, cx| {
                if this.menu == Some(role) {
                    this.menu = None;
                    cx.notify();
                } else {
                    this.open_menu(role, window, cx);
                }
            }));
        div()
            .flex()
            .flex_col()
            .child(div().flex().child(main).child(caret))
            .when(menu_open, |d| d.child(self.menu_list(role, cx)))
            .into_any_element()
    }

    fn menu_list(&self, role: Role, cx: &mut Context<Self>) -> impl IntoElement {
        let key = role_key(role);
        let choices = launcher::choices(role, cx);
        let empty = choices.is_empty();
        let items = choices
            .into_iter()
            .enumerate()
            .map(|(i, (choice, is_default))| {
                Button::new((SharedString::from(format!("{key}-choice")), i))
                    .when_some(self.items.get(i), |b, h| b.track_focus(h))
                    .tab_stop(false)
                    .accessibility_label(if is_default {
                        format!("{}, default", choice.name)
                    } else {
                        choice.name.clone()
                    })
                    .h(px(28.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .rounded(px(5.))
                    .cursor_pointer()
                    .text_color(theme::text())
                    .hover(|s| s.bg(theme::selected()))
                    .focus(|s| s.bg(theme::selected()))
                    .child(choice.name.clone())
                    .when(is_default, |d| {
                        d.child(
                            div()
                                .text_size(px(11.))
                                .text_color(theme::faint())
                                .child("default"),
                        )
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open(role, Some(choice.clone()), window, cx)
                    }))
            });
        deferred(
            anchored().snap_to_window().child(
                div()
                    .id(SharedString::from(format!("{key}-menu")))
                    .role(gpui_kit::Role::Menu)
                    .track_focus(&self.menu_focus)
                    .key_context(MENU_CONTEXT)
                    .on_key_down(cx.listener(Self::on_menu_key))
                    .on_action(cx.listener(|this, _: &LeaveMenu, window, cx| {
                        this.close_menu(window, cx);
                        window.focus_next(cx);
                    }))
                    .on_action(cx.listener(|this, _: &LeaveMenuBack, window, cx| {
                        this.close_menu(window, cx);
                        window.focus_prev(cx);
                    }))
                    .mt(px(4.))
                    .min_w(px(200.))
                    .p(px(4.))
                    .flex()
                    .flex_col()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(theme::border_strong())
                    .bg(theme::raised())
                    .shadow_lg()
                    .occlude()
                    .text_size(px(13.))
                    .child(
                        div()
                            .px(px(10.))
                            .py(px(4.))
                            .text_size(px(11.))
                            .text_color(theme::faint())
                            .child("OPEN THIS TIME WITH"),
                    )
                    .when(empty, |d| {
                        d.child(
                            div()
                                .px(px(10.))
                                .py(px(4.))
                                .text_color(theme::muted())
                                .child(format!("No {key}s found")),
                        )
                    })
                    .children(items)
                    // Outside click closes the menu and is swallowed, like a native menu.
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.menu = None;
                        cx.stop_propagation();
                        cx.notify();
                    }))
                    .test_support(),
            ),
        )
        .with_priority(1)
    }
}

fn section(title: &'static str) -> gpui_kit::Div {
    div().flex().flex_col().gap(px(6.)).child(
        div()
            .text_size(px(11.))
            .text_color(theme::faint())
            .child(title),
    )
}

fn field(label: &'static str, value: impl Into<SharedString>) -> impl IntoElement {
    div()
        .flex()
        .gap(px(10.))
        .text_size(px(13.))
        .child(
            div()
                .w(px(92.))
                .flex_none()
                .text_color(theme::muted())
                .child(label),
        )
        .child(
            div()
                .min_w_0()
                .text_color(theme::text_soft())
                .child(value.into()),
        )
}

fn link_button(
    id: &'static str,
    label: &'static str,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut gpui_kit::App) + 'static,
) -> impl IntoElement {
    Button::new(id)
        .flex_none()
        .h(px(26.))
        .px(px(10.))
        .rounded(px(6.))
        .border_1()
        .border_color(theme::border_strong())
        .focus_visible(|s| s.border_color(theme::link()))
        .bg(theme::raised())
        .text_size(px(12.))
        .text_color(theme::text())
        .cursor_pointer()
        .hover(|s| s.bg(theme::border()))
        .child(label)
        .on_click(on_click)
}

fn file_row(i: usize, f: &ChangedFile) -> impl IntoElement {
    let path = match &f.from {
        Some(from) => format!("{from} → {}", f.path),
        None => f.path.clone(),
    };
    div()
        .id(("changed-file", i))
        .flex()
        .gap(px(10.))
        .font_family(theme::MONO)
        .text_size(px(12.))
        .child(
            div()
                .w(px(18.))
                .flex_none()
                .text_color(theme::accent())
                .child(f.change.code()),
        )
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_color(theme::text_soft())
                .child(path),
        )
        .test_support()
}

const SYNC: &str = "Sync";

/// Label/value rows of the STATUS section. The branch line itself is the pane's top
/// switcher, so an on-branch, on-disk worktree leaves its branch to that trigger; a
/// detached HEAD (and a worktree not on disk, where the trigger is off) keeps its row.
fn status_fields(wt: &Worktree) -> Vec<(&'static str, String)> {
    let head = match (&wt.branch, &wt.head) {
        (Some(b), _) => b.clone(),
        (None, Some(sha)) => format!("detached @ {}", sha.get(..7).unwrap_or(sha)),
        (None, None) => "unknown".into(),
    };
    let mut out = vec![];
    if wt.branch.is_none() || !wt.presence.is_openable() {
        out.push(("Branch", head));
    }
    match (&wt.status, wt.presence.is_openable()) {
        (_, false) => out.push(("Working tree", "not on disk".into())),
        (None, true) => out.push(("Working tree", "reading…".into())),
        (Some(Err(e)), true) => out.push(("Working tree", format!("unknown ({e})"))),
        (Some(Ok(s)), true) => out.extend([
            ("Upstream", s.upstream.name().unwrap_or("none").to_string()),
            (SYNC, s.upstream.sync_words()),
            ("Working tree", s.summary()),
        ]),
    }
    out
}

fn commit_row(i: usize, c: &Commit, now: i64) -> impl IntoElement {
    div()
        .id(("commit", i))
        .flex()
        .items_baseline()
        .gap(px(10.))
        .text_size(px(12.))
        .child(
            theme::mono(c.short_sha.clone())
                .flex_none()
                .text_color(theme::link()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(theme::text_soft())
                .child(c.subject.clone()),
        )
        .child(
            div()
                .flex_none()
                .text_color(theme::faint())
                .child(relative_time(c.time, now)),
        )
        .test_support()
}

impl DetailsView {
    fn worktree_sections(&self, wt: &Worktree, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let openable = wt.presence.is_openable();
        let mut out = Vec::new();

        out.push(
            section("PATH")
                .child(
                    theme::path_text(tilde(&wt.path))
                        .text_size(px(13.))
                        .text_color(theme::worktree_text()),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(6.))
                        .child(link_button(
                            "details-copy-path",
                            "Copy path",
                            cx.listener(|this, _, _, cx| this.copy_path(cx)),
                        ))
                        .child(link_button(
                            "details-copy-cd",
                            "Copy cd command",
                            cx.listener(|this, _, _, cx| this.copy_cd(cx)),
                        ))
                        .child(link_button(
                            "details-reveal",
                            "Reveal in Finder",
                            cx.listener(|this, _, _, cx| this.reveal(cx)),
                        )),
                )
                .into_any_element(),
        );

        out.push(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(self.open_split(Role::Editor, openable, cx))
                .child(self.open_split(Role::Terminal, openable, cx))
                .children(wt.presence.reason().map(|reason| {
                    div()
                        .p(px(8.))
                        .rounded(px(6.))
                        .bg(theme::warn_bg())
                        .text_color(theme::warn_text())
                        .text_size(px(12.))
                        .child(format!("Can't open: {reason}"))
                }))
                .into_any_element(),
        );

        let pull_offered = pull::offer(wt).is_some();
        out.push(
            section("STATUS")
                .children(self.status_rows().into_iter().map(|(l, v)| {
                    if pull_offered && l == SYNC {
                        self.sync_field(v, cx).into_any_element()
                    } else {
                        field(l, v).into_any_element()
                    }
                }))
                .into_any_element(),
        );

        let files = match &wt.status {
            Some(Ok(s)) if openable && !s.files.is_empty() => Some(&s.files),
            _ => None,
        };
        if let Some(files) = files {
            out.push(
                section("CHANGED FILES")
                    .children(files.iter().enumerate().map(|(i, f)| file_row(i, f)))
                    .into_any_element(),
            );
        }

        if openable {
            let now = unix_now();
            let body = match &self.commits.result {
                None => div()
                    .text_size(px(12.))
                    .text_color(theme::faint())
                    .child("Reading…")
                    .into_any_element(),
                Some(Err(e)) => div()
                    .text_size(px(12.))
                    .text_color(theme::warn_text())
                    .child(e.to_string())
                    .into_any_element(),
                Some(Ok(c)) if c.is_empty() => div()
                    .text_size(px(12.))
                    .text_color(theme::faint())
                    .child("No commits yet")
                    .into_any_element(),
                Some(Ok(c)) => div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .children(c.iter().enumerate().map(|(i, c)| commit_row(i, c, now)))
                    .into_any_element(),
            };
            out.push(section("RECENT COMMITS").child(body).into_any_element());
        }

        if let Some(offer) = removal::offer(wt) {
            let path = wt.path.clone();
            let enabled = offer.reason.is_none();
            out.push(
                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(6.))
                    .child(
                        Button::new("details-remove")
                            .accessibility_label(offer.label)
                            .when_some(offer.reason.clone(), |b, reason| b.aria_description(reason))
                            .disabled(!enabled)
                            .h(px(28.))
                            .px(px(10.))
                            .rounded(px(6.))
                            .border_1()
                            .border_color(theme::border_strong())
                            .focus_visible(|s| s.border_color(theme::link()))
                            .bg(theme::raised())
                            .text_size(px(12.))
                            .when_else(
                                enabled,
                                |b| {
                                    b.text_color(theme::warn_text())
                                        .cursor_pointer()
                                        .hover(|s| s.bg(theme::border()))
                                },
                                |b| b.text_color(theme::faint()),
                            )
                            .child(offer.label)
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(RemoveRequested { path: path.clone() })
                            })),
                    )
                    .children(offer.reason.map(|reason| {
                        div()
                            .id("details-remove-reason")
                            .aria_label(reason.clone())
                            .text_size(px(12.))
                            .text_color(theme::muted())
                            .child(reason)
                            .test_support()
                    }))
                    .into_any_element(),
            );
        }
        out
    }
}

impl Render for DetailsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The resizable panel owns the width; this view fills it.
        let base = div()
            .id("details")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .bg(theme::panel())
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(18.));
        let Some(details) = self.details.clone() else {
            return base
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(theme::muted())
                        .child("Select a branch or worktree"),
                )
                .test_support();
        };
        let header = div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                theme::mono(details.title.clone())
                    .id("details-title")
                    .text_size(px(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text())
                    .test_support(),
            )
            .child(
                div()
                    .id("details-kind")
                    .text_size(px(12.))
                    .text_color(theme::muted())
                    .child(details.kind.clone())
                    .test_support(),
            )
            .when_some(self.worktree().cloned(), |header, wt| {
                header.child(self.branch_switch(&wt, window, cx))
            });
        let sections = match self.worktree().cloned() {
            Some(ref wt) => self.worktree_sections(wt, cx),
            None => vec![
                div()
                    .text_size(px(13.))
                    .text_color(theme::muted())
                    .child("Select a branch or worktree to see its details.")
                    .into_any_element(),
            ],
        };
        base.child(header).children(sections).test_support()
    }
}
