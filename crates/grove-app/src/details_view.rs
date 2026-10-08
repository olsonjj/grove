//! Right pane: everything about the tree's selection, plus open/copy/reveal actions.

use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, AppContext, ClipboardItem, Context, Entity, EventEmitter, FontWeight,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, TestSupportExt, Window, anchored, deferred,
    div, px,
};
use grove_core::{
    ChangedFile, Commit, Details, Role, Worktree, cd_command, recent_commits, relative_time,
    unix_now,
};

use grove_core::git::GitError;

use crate::icons::{self, icon};
use crate::launcher::Choice;
use crate::main_window::tilde;
use crate::tree_view::{OpenRequested, TreeView};
use crate::{launcher, theme};

/// A confirmation or failure for the status bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
    pub error: bool,
}

pub struct DetailsView {
    tree: Entity<TreeView>,
    details: Option<Details>,
    commits: Commits,
    /// Which ▾ menu is open.
    menu: Option<Role>,
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

impl DetailsView {
    pub fn new(tree: Entity<TreeView>, cx: &mut Context<Self>) -> Self {
        let sub = cx.observe(&tree, |this, _, cx| this.sync(cx));
        let mut this = Self {
            tree,
            details: None,
            commits: Commits::default(),
            menu: None,
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

    fn open(&mut self, role: Role, target: Option<Choice>, cx: &mut Context<Self>) {
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
            self.menu = Some(role);
        }
        cx.emit(req);
        cx.notify();
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
            div()
                .id(id)
                .h(px(32.))
                .flex()
                .items_center()
                .bg(bg)
                .text_color(fg)
                .when(!primary || !openable, |d| {
                    d.border_1().border_color(theme::border_strong())
                })
                .when(openable, |d| d.cursor_pointer().hover(|s| s.bg(hover)))
        };
        let main = part(format!("details-open-{key}").into())
            .flex_1()
            .min_w_0()
            .gap(px(8.))
            .px(px(12.))
            .rounded_l(px(8.))
            .when(primary, |d| d.font_weight(FontWeight::SEMIBOLD))
            .child(icon(
                if primary {
                    icons::EDITOR
                } else {
                    icons::TERMINAL
                },
                14.,
            ))
            .child(div().min_w_0().truncate().child(label))
            .when(openable, |d| {
                d.on_click(cx.listener(move |this, _, _, cx| this.open(role, None, cx)))
            })
            .test_support();
        let caret = part(format!("details-menu-{key}").into())
            .flex_none()
            .w(px(30.))
            .justify_center()
            .rounded_r(px(8.))
            .border_l_1()
            .border_color(if primary && openable {
                theme::on_accent_divider()
            } else {
                theme::border_strong()
            })
            .child(icon(icons::CHEVRON_DOWN, 13.))
            .when(openable, |d| {
                d.on_click(cx.listener(move |this, _, _, cx| {
                    this.menu = (this.menu != Some(role)).then_some(role);
                    cx.notify();
                }))
            })
            .test_support();
        div()
            .flex()
            .flex_col()
            .child(div().flex().child(main).child(caret))
            .when(openable && self.menu == Some(role), |d| {
                d.child(self.menu_list(role, cx))
            })
            .into_any_element()
    }

    fn menu_list(&self, role: Role, cx: &mut Context<Self>) -> impl IntoElement {
        let key = role_key(role);
        let choices = launcher::choices(role, cx);
        let empty = choices.is_empty();
        let items =
            choices
                .into_iter()
                .enumerate()
                .map(|(i, (choice, is_default))| {
                    div()
                        .id((SharedString::from(format!("{key}-choice")), i))
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
                        .child(choice.name.clone())
                        .when(is_default, |d| {
                            d.child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(theme::faint())
                                    .child("default"),
                            )
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open(role, Some(choice.clone()), cx)
                        }))
                        .test_support()
                });
        deferred(
            anchored().snap_to_window().child(
                div()
                    .id(SharedString::from(format!("{key}-menu")))
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
    div()
        .id(id)
        .flex_none()
        .h(px(26.))
        .px(px(10.))
        .flex()
        .items_center()
        .rounded(px(6.))
        .border_1()
        .border_color(theme::border_strong())
        .bg(theme::raised())
        .text_size(px(12.))
        .text_color(theme::text())
        .cursor_pointer()
        .hover(|s| s.bg(theme::border()))
        .child(label)
        .on_click(on_click)
        .test_support()
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
