//! Right pane: everything about the tree's selection, plus open/copy/reveal actions.

use gpui_kit::base::Button;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, App, AppContext, ClipboardItem, Context, Entity, EventEmitter, FocusHandle,
    FontWeight, InteractiveElement, IntoElement, KeyBinding, KeyDownEvent, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, TestSupportExt, Window,
    anchored, deferred, div, px,
};
use grove_core::{
    ChangedFile, Commit, Details, Role, Worktree, cd_command, recent_commits, relative_time,
    unix_now,
};

use grove_core::git::GitError;

use crate::icons::{self, icon};
use crate::launcher::Choice;
use crate::main_window::tilde;
use crate::removal::{self, RemoveRequested};
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

/// Tab out of an open menu (▾, a repo's context menu) closes it; this context outranks the window's own Tab.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", LeaveMenu, Some(MENU_CONTEXT)),
        KeyBinding::new("shift-tab", LeaveMenuBack, Some(MENU_CONTEXT)),
    ]);
}

pub struct DetailsView {
    tree: Entity<TreeView>,
    details: Option<Details>,
    commits: Commits,
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
    /// The worktree as it was when read; a changed status or HEAD rereads.
    source: Option<Worktree>,
    result: Option<Result<Vec<Commit>, GitError>>,
    /// Bumped per read so a slow read for an old selection can't land.
    seq: u64,
}

impl EventEmitter<OpenRequested> for DetailsView {}
impl EventEmitter<Notice> for DetailsView {}
impl EventEmitter<RemoveRequested> for DetailsView {}

impl DetailsView {
    pub fn new(tree: Entity<TreeView>, cx: &mut Context<Self>) -> Self {
        let sub = cx.observe(&tree, |this, _, cx| this.sync(cx));
        let mut this = Self {
            tree,
            details: None,
            commits: Commits::default(),
            menu: None,
            carets: [cx.focus_handle(), cx.focus_handle()],
            items: Vec::new(),
            menu_focus: cx.focus_handle(),
            _sub: sub,
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
        self.worktree().map(status_fields).unwrap_or_default()
    }

    /// `None` while reading.
    #[cfg(test)]
    pub fn commits(&self) -> Option<&Result<Vec<Commit>, GitError>> {
        self.commits.result.as_ref()
    }

    fn sync(&mut self, cx: &mut Context<Self>) {
        let tree = self.tree.read(cx);
        let details = tree
            .selected()
            .zip(tree.repo())
            .and_then(|(id, repo)| repo.details(id));
        if details == self.details {
            return;
        }
        let path = |d: &Option<Details>| {
            d.as_ref()
                .and_then(|d| d.worktree.as_ref())
                .map(|w| w.path.clone())
        };
        if path(&details) != path(&self.details) {
            self.menu = None;
        }
        self.details = details;
        self.read_commits(cx);
        cx.notify();
    }

    fn worktree(&self) -> Option<&Worktree> {
        self.details.as_ref().and_then(|d| d.worktree.as_ref())
    }

    fn read_commits(&mut self, cx: &mut Context<Self>) {
        let wt = self
            .worktree()
            .filter(|w| w.presence.is_openable())
            .cloned();
        if wt == self.commits.source {
            return;
        }
        if wt.as_ref().map(|w| &w.path) != self.commits.source.as_ref().map(|w| &w.path) {
            self.commits.result = None;
        }
        self.commits.seq += 1;
        let seq = self.commits.seq;
        let path = wt.as_ref().map(|w| w.path.clone());
        self.commits.source = wt;
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

/// Label/value rows of the STATUS section.
fn status_fields(wt: &Worktree) -> Vec<(&'static str, String)> {
    let head = match (&wt.branch, &wt.head) {
        (Some(b), _) => b.clone(),
        (None, Some(sha)) => format!("detached @ {}", sha.get(..7).unwrap_or(sha)),
        (None, None) => "unknown".into(),
    };
    let mut out = vec![("Branch", head)];
    match (&wt.status, wt.presence.is_openable()) {
        (_, false) => out.push(("Working tree", "not on disk".into())),
        (None, true) => out.push(("Working tree", "reading…".into())),
        (Some(Err(e)), true) => out.push(("Working tree", format!("unknown ({e})"))),
        (Some(Ok(s)), true) => out.extend([
            ("Upstream", s.upstream.name().unwrap_or("none").to_string()),
            ("Sync", s.upstream.sync_words()),
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

        out.push(
            section("STATUS")
                .children(status_fields(wt).into_iter().map(|(l, v)| field(l, v)))
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let base = div()
            .id("details")
            .w(px(320.))
            .min_w(px(260.))
            .flex_shrink(1.)
            .h_full()
            .overflow_y_scroll()
            .border_l_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(18.));
        let Some(details) = &self.details else {
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
            );
        let sections = match &details.worktree {
            Some(wt) => self.worktree_sections(wt, cx),
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
