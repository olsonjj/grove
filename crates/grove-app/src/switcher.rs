//! Quick switcher (⌥⌘K anywhere, ⌘K in the main window): search every worktree, open it.

use std::ops::Range;
use std::time::{Duration, Instant};

use gpui_kit::component::input::{Enter, Escape, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext, Bounds, ClipboardItem, Context, DisplayId,
    Entity, FontWeight, Global, HighlightStyle, InteractiveElement, IntoElement, KeyDownEvent,
    KeyUpEvent, ParentElement, Render, ScrollHandle, SharedString, StatefulInteractiveElement,
    Styled, StyledText, Subscription, TestSupportExt, Window, WindowId, WindowOptions, div, point,
    px, size,
};
use grove_core::{Badge, DisplayPath, Hit, Presence, Role, home_dir, search};

use crate::icons::{self, icon};
use crate::launcher::{self, Notice};
use crate::main_window::tilde;
use crate::shell::{self, REOPEN_GUARD};
use crate::windows::{self, close_on_deactivate, panel_options};
use crate::{macos, theme};

const SIZE: (f32, f32) = (640., 440.);
/// Long enough to read "Copied path …" before focus goes back.
const COPY_CLOSE_DELAY: Duration = Duration::from_millis(700);

#[derive(Default)]
struct Current {
    window: Option<AnyWindowHandle>,
    /// Closed by Esc, a toggle or an action, rather than by a click elsewhere.
    dismissed: bool,
    /// When a click elsewhere last closed it.
    closed_at: Option<Instant>,
}

impl Global for Current {}

pub fn window(cx: &App) -> Option<AnyWindowHandle> {
    cx.try_global::<Current>().and_then(|c| c.window)
}

/// Closes the switcher if it's open, else opens it. Returns whether it opened.
pub fn toggle(at: Instant, cx: &mut App) -> bool {
    if let Some(open) = window(cx).filter(|w| cx.windows().contains(w)) {
        dismiss(open, cx);
        return false;
    }
    // Clicking Jump to… while the switcher is open first closes it on deactivation; that
    // click mustn't reopen it.
    let closed_at = cx.default_global::<Current>().closed_at;
    if closed_at.is_some_and(|t| at.saturating_duration_since(t) < REOPEN_GUARD) {
        return false;
    }
    let window = open(cx);
    // open_window draws the first frame before returning.
    eprintln!(
        "[grove] switcher: hotkey → first frame {:.1} ms",
        windows::ms(at)
    );
    cx.set_global(Current {
        window,
        ..Default::default()
    });
    window.is_some()
}

/// Closes the switcher on the user's say-so, so focus goes back to the previous app.
pub fn dismiss(window: AnyWindowHandle, cx: &mut App) {
    let current = cx.default_global::<Current>();
    if current.window == Some(window) {
        current.dismissed = true;
    }
    // Deferred: from the switcher's own key handlers, its window is mid-update.
    cx.defer(move |cx| shell::close(window, cx));
}

/// Forgets the switcher once window `id` closed. If it was the switcher: whether it was
/// dismissed rather than closed by a click elsewhere.
pub fn closed(id: WindowId, cx: &mut App) -> Option<bool> {
    let current = cx.default_global::<Current>();
    current.window.filter(|w| w.window_id() == id)?;
    current.window = None;
    if !current.dismissed {
        current.closed_at = Some(Instant::now());
    }
    Some(current.dismissed)
}

/// A fresh window per open: with the renderer warmed up at launch (`windows::warm_up`),
/// creating one takes a few ms, and the search starts from current settings and snapshot.
/// Opens on the display with the mouse, as Spotlight does.
fn open(cx: &mut App) -> Option<AnyWindowHandle> {
    let (w, h) = SIZE;
    let display = macos::display_under_mouse()
        .map(DisplayId::from)
        .and_then(|id| cx.find_display(id))
        .or_else(|| cx.primary_display());
    // Window bounds are relative to their display.
    let bounds = match &display {
        Some(display) => {
            let screen = display.bounds().size;
            let origin = point((screen.width - px(w)) / 2., screen.height / 5.);
            Bounds::new(origin, size(px(w), px(h)))
        }
        None => Bounds::centered(None, size(px(w), px(h)), cx),
    };
    let options = panel_options(bounds, display.map(|d| d.id()));
    open_with(options, cx)
        .map(|(handle, _)| handle)
        .inspect_err(|e| eprintln!("[grove] switcher: {e:#}"))
        .ok()
}

fn open_with(
    options: WindowOptions,
    cx: &mut App,
) -> gpui_kit::Result<(AnyWindowHandle, Entity<Switcher>)> {
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| {
            close_on_deactivate(window, cx);
            Switcher::new(window, cx)
        })
    })
}

struct Switcher {
    window: AnyWindowHandle,
    input: Entity<InputState>,
    query: String,
    hits: Vec<Hit>,
    selected: usize,
    scroll: ScrollHandle,
    /// Confirmation or failure of the last action.
    status: Option<Notice>,
    /// Bumped per action so a slow launch can't overwrite a newer line or close.
    seq: u64,
    /// The line of a launch in flight; further opens wait for its outcome.
    launching: Option<String>,
    /// Enter or Esc acted on and not yet released: its key repeats are ignored. (Their
    /// actions don't carry `is_held`.)
    held: Option<&'static str>,
    _subs: [Subscription; 2],
}

impl Switcher {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Jump to a branch or worktree…"));
        input.update(cx, |input, cx| input.focus(window, cx));
        let query = cx.subscribe(&input, |this, input, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                let query = input.read(cx).value().to_string();
                this.set_query(query, cx);
            }
        });
        // Status reads landing update badges in place; selection follows its worktree.
        let library = cx.observe_global::<shell::Library>(|this, cx| this.search(cx));
        let mut this = Self {
            window: window.window_handle(),
            input,
            query: String::new(),
            hits: Vec::new(),
            selected: 0,
            scroll: ScrollHandle::new(),
            status: None,
            seq: 0,
            launching: None,
            held: None,
            _subs: [query, library],
        };
        this.search(cx);
        this
    }

    fn search(&mut self, cx: &mut Context<Self>) {
        let library = shell::library(cx);
        let selected = self.hits.get(self.selected).map(|h| h.path.clone());
        self.hits = search(
            library.store.snapshot(),
            &library.settings.recents,
            &self.query,
        );
        self.selected = selected
            .and_then(|p| self.hits.iter().position(|h| h.path == p))
            .unwrap_or(0);
        cx.notify();
    }

    fn set_query(&mut self, query: String, cx: &mut Context<Self>) {
        self.query = query;
        self.status = None;
        self.hits.clear();
        self.search(cx);
        self.scroll_to_selected();
    }

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.selected = ix.min(self.hits.len().saturating_sub(1));
        self.scroll_to_selected();
        cx.notify();
    }

    fn scroll_to_selected(&self) {
        let headers = match self.hits.get(self.selected) {
            Some(hit) if self.query.trim().is_empty() && !hit.recent => {
                1 + usize::from(self.hits[0].recent)
            }
            Some(_) => 1,
            None => return,
        };
        self.scroll.scroll_to_item(self.selected + headers);
    }

    /// False for a repeat of `key` still held from the press that last acted.
    fn fresh_press(&mut self, key: &'static str) -> bool {
        self.held.replace(key) != Some(key)
    }

    fn on_move_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        self.select(self.selected.saturating_sub(1), cx);
        cx.stop_propagation();
    }

    fn on_move_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        self.select(self.selected + 1, cx);
        cx.stop_propagation();
    }

    fn on_enter(&mut self, action: &Enter, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if self.fresh_press("enter") {
            let role = if action.secondary {
                Role::Terminal
            } else {
                Role::Editor
            };
            self.open(role, cx);
        }
    }

    fn on_escape(&mut self, _: &Escape, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if self.fresh_press("escape") {
            dismiss(self.window, cx);
        }
    }

    /// Keys the input has no binding for.
    fn on_key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        let m = &ks.modifiers;
        match ks.key.as_str() {
            "k" if m.platform && !ev.is_held => dismiss(self.window, cx),
            "enter" if m.alt && !ev.is_held => self.copy_path(cx),
            "k" if m.platform => {}
            "enter" if m.alt => {}
            _ => return,
        }
        cx.stop_propagation();
    }

    fn on_key_up(&mut self, ev: &KeyUpEvent, _: &mut Window, _: &mut Context<Self>) {
        if self.held == Some(ev.keystroke.key.as_str()) {
            self.held = None;
        }
    }

    /// Shows `notice` here and in the main window's status bar, which outlives the switcher.
    fn show(&mut self, notice: Notice, settings: bool, cx: &mut Context<Self>) {
        notice.post(settings, cx);
        // Seen here; a main window opened later shouldn't replay it.
        shell::notice_shown(cx);
        self.status = Some(notice);
        cx.notify();
    }

    /// Opens the selected worktree; closes once the launch went through, or stays open to
    /// show why it didn't.
    fn open(&mut self, role: Role, cx: &mut Context<Self>) {
        if let Some(line) = &self.launching {
            self.status = Some(Notice {
                text: format!("{line} Still opening…"),
                warn: false,
            });
            return cx.notify();
        }
        let Some(hit) = self.hits.get(self.selected) else {
            return;
        };
        self.seq += 1;
        let seq = self.seq;
        let (path, presence) = (hit.path.clone(), hit.presence);
        let opening = match launcher::open_worktree(&path, presence, role, None, cx) {
            Ok(opening) => opening,
            Err(e) => {
                let notice = Notice {
                    text: e.message,
                    warn: true,
                };
                return self.show(notice, e.settings, cx);
            }
        };
        self.launching = Some(opening.message.clone());
        let notice = Notice {
            text: opening.message.clone(),
            warn: false,
        };
        self.show(notice, false, cx);
        let window = self.window;
        cx.spawn(async move |this, cx| {
            let notice = opening.finish().await;
            cx.update(|cx| {
                if let Some(n) = &notice {
                    n.post(false, cx);
                }
                let warn = notice.as_ref().is_some_and(|n| n.warn);
                let current = this.update(cx, |this, cx| {
                    this.launching = None;
                    if this.seq == seq && warn {
                        shell::notice_shown(cx);
                        this.status = notice;
                        cx.notify();
                    }
                    this.seq == seq
                });
                if current.unwrap_or(false) && !warn {
                    dismiss(window, cx);
                }
            });
        })
        .detach();
    }

    fn copy_path(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.hits.get(self.selected).map(|h| h.path.clone()) else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(path.display().to_string()));
        self.seq += 1;
        let seq = self.seq;
        let notice = Notice {
            text: format!("Copied path {}", tilde(&path)),
            warn: false,
        };
        self.show(notice, false, cx);
        let window = self.window;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPY_CLOSE_DELAY).await;
            cx.update(|cx| {
                if this
                    .read_with(cx, |this, _| this.seq == seq)
                    .unwrap_or(false)
                {
                    dismiss(window, cx);
                }
            });
        })
        .detach();
    }

    fn on_row_click(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.select(ix, cx);
        self.open(Role::Editor, cx);
    }
}

impl Switcher {
    fn input_row(&self) -> impl IntoElement {
        div()
            .flex_none()
            .h(px(56.))
            .pl(px(18.))
            .pr(px(12.))
            .flex()
            .items_center()
            .gap(px(12.))
            .border_b_1()
            .border_color(theme::border())
            .child(icon(icons::SEARCH, 18.).text_color(theme::muted()))
            .child(
                Input::new(&self.input)
                    .appearance(false)
                    .flex_1()
                    .min_w_0()
                    .px_0()
                    .font_family(theme::MONO)
                    .text_size(px(17.)),
            )
            .child(
                div()
                    .flex_none()
                    .px(px(10.))
                    .py(px(4.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(theme::border_strong())
                    .text_size(px(12.))
                    .text_color(theme::text_soft())
                    .child("Esc"),
            )
    }

    fn row(&self, ix: usize, hit: &Hit, cx: &mut Context<Self>) -> AnyElement {
        let selected = ix == self.selected;
        let (title, title_ranges) = match &hit.branch {
            Some(branch) => (branch.clone(), &hit.matches.branch),
            None => (hit.folder.clone(), &hit.matches.folder),
        };
        let path = DisplayPath::new(&hit.path, home_dir().as_deref());
        // The folder's matches sit at the end of the shown path.
        let folder_ranges = if path.name == hit.folder {
            shift(&hit.matches.folder, path.parent.len())
        } else {
            Vec::new()
        };
        let badges = hit
            .badges
            .iter()
            .filter(|b| matches!(b, Badge::Changed(_) | Badge::Missing | Badge::Unavailable));
        let dim = hit.presence == Presence::Unavailable;
        let mut label = format!("{title}, {}, {}", hit.repo, path.full());
        for b in &hit.badges {
            label.push_str(", ");
            label.push_str(&b.description());
        }
        div()
            .id(("switcher-row", ix))
            .role(gpui_kit::Role::ListBoxOption)
            .aria_label(label)
            .aria_selected(selected)
            .flex_none()
            .min_h(px(48.))
            .px(px(12.))
            .py(px(4.))
            .flex()
            .items_center()
            .gap(px(12.))
            .rounded(px(8.))
            .cursor_pointer()
            .relative()
            .when_else(
                selected,
                |d| d.bg(theme::selected()).child(theme::selection_bar()),
                |d| d.hover(|s| s.bg(theme::raised())),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.on_row_click(ix, cx)))
            .child(icon(icons::FOLDER, 16.).text_color(if dim {
                theme::faint()
            } else {
                theme::link()
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .flex()
                            .min_w_0()
                            .whitespace_nowrap()
                            .font_family(theme::MONO)
                            .text_size(px(14.))
                            .text_color(if dim { theme::faint() } else { theme::text() })
                            .child(
                                div()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(highlighted(title, title_ranges)),
                            )
                            .when(hit.branch.is_none(), |d| {
                                d.child(
                                    div()
                                        .flex_none()
                                        .text_color(theme::faint())
                                        .child(" (detached)"),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .min_w_0()
                            .whitespace_nowrap()
                            .text_size(px(12.))
                            .text_color(theme::muted())
                            .child(
                                div()
                                    .flex_none()
                                    .font_family(theme::MONO)
                                    .child(highlighted(hit.repo.clone(), &hit.matches.repo)),
                            )
                            .child(div().flex_none().child("\u{a0}·\u{a0}"))
                            .child(
                                div()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis_start()
                                    .font_family(theme::MONO)
                                    .child(highlighted(path.full(), &folder_ranges)),
                            ),
                    ),
            )
            .children(badges.enumerate().map(|(i, b)| badge(ix, i, b)))
            .child(
                div()
                    .flex_none()
                    .w(px(14.))
                    .font_family(theme::MONO)
                    .text_size(px(12.))
                    .text_color(theme::text_soft())
                    .when(selected, |d| d.child("↵")),
            )
            .test_support()
            .into_any_element()
    }

    fn results(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let header = |label: &'static str| {
            div()
                .flex_none()
                .px(px(10.))
                .pt(px(8.))
                .pb(px(6.))
                .text_size(px(11.))
                .text_color(theme::faint())
                .child(label)
                .into_any_element()
        };
        let empty_query = self.query.trim().is_empty();
        let mut items: Vec<AnyElement> = Vec::with_capacity(self.hits.len() + 2);
        for (ix, hit) in self.hits.iter().enumerate() {
            // Keep in step with `scroll_to_selected`.
            let label = match (empty_query, ix) {
                (true, 0) if hit.recent => Some("RECENT"),
                (true, _) if !hit.recent && (ix == 0 || self.hits[ix - 1].recent) => {
                    Some("ALL WORKTREES")
                }
                (false, 0) => Some("WORKTREES"),
                _ => None,
            };
            items.extend(label.map(header));
            items.push(self.row(ix, hit, cx));
        }
        let empty = self.hits.is_empty().then(|| {
            let text = if empty_query {
                "No worktrees yet.".to_string()
            } else {
                format!(
                    "Nothing matches “{}”. Branches show up here once they're checked out in a worktree.",
                    self.query.trim()
                )
            };
            div()
                .id("nothing-matches")
                .px(px(16.))
                .py(px(40.))
                .flex()
                .justify_center()
                .text_size(px(14.))
                .text_color(theme::muted())
                .child(text)
                .test_support()
        });
        div()
            .id("switcher-rows")
            .role(gpui_kit::Role::ListBox)
            .aria_label("Worktrees")
            .track_scroll(&self.scroll)
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p(px(8.))
            .flex()
            .flex_col()
            .children(items)
            .children(empty)
    }

    fn footer(&self, cx: &App) -> impl IntoElement {
        let key = |keys: &'static str, label: String| {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    // System font: the mono face lacks ⌘/⌥, and its fallback sits off-baseline.
                    div()
                        .min_w(px(20.))
                        .flex()
                        .justify_center()
                        .text_color(theme::text())
                        .child(keys),
                )
                .child(label)
        };
        let app = |role, fallback: &str| {
            launcher::target_name(role, cx).unwrap_or_else(|| fallback.to_string())
        };
        div()
            .flex_none()
            .flex()
            .flex_wrap()
            .gap_x(px(22.))
            .gap_y(px(8.))
            .px(px(18.))
            .py(px(12.))
            .border_t_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .text_size(px(12.))
            .text_color(theme::muted())
            .child(key("↑↓", "Move".into()))
            .child(key("↵", format!("Open in {}", app(Role::Editor, "editor"))))
            .child(key(
                "⌘↵",
                format!("Open in {}", app(Role::Terminal, "terminal")),
            ))
            .child(key("⌥↵", "Copy path".into()))
    }
}

/// `text` with `ranges` in the accent color.
fn highlighted(text: String, ranges: &[Range<usize>]) -> StyledText {
    let style = HighlightStyle {
        color: Some(theme::accent().into()),
        font_weight: Some(FontWeight::SEMIBOLD),
        ..Default::default()
    };
    StyledText::new(text).with_highlights(ranges.iter().map(|r| (r.clone(), style)))
}

fn shift(ranges: &[Range<usize>], by: usize) -> Vec<Range<usize>> {
    ranges.iter().map(|r| r.start + by..r.end + by).collect()
}

fn badge(row: usize, i: usize, b: &Badge) -> impl IntoElement {
    let (bg, fg) = match b {
        Badge::Changed(_) => (theme::accent(), theme::on_accent()),
        Badge::Missing => (theme::warn_bg(), theme::warn_text()),
        _ => (theme::border(), theme::text_soft()),
    };
    let tip: SharedString = b.description().into();
    div()
        .id(SharedString::from(format!("switcher-badge-{row}-{i}")))
        .flex_none()
        .px(px(7.))
        .h(px(18.))
        .flex()
        .items_center()
        .rounded(px(9.))
        .bg(bg)
        .text_color(fg)
        .text_size(px(11.))
        .child(b.label())
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .test_support()
}

impl Render for Switcher {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = self.status.clone().map(|n| {
            div()
                .id("switcher-status")
                .flex_none()
                .px(px(18.))
                .py(px(10.))
                .border_t_1()
                .border_color(theme::border())
                .text_size(px(12.))
                .when_else(
                    n.warn,
                    |d| d.bg(theme::warn_bg()).text_color(theme::warn_text()),
                    |d| d.bg(theme::panel()).text_color(theme::text_soft()),
                )
                .child(n.text)
        });
        div()
            // Capture: ahead of the input's own handlers for the same keys.
            .capture_action(cx.listener(Self::on_move_up))
            .capture_action(cx.listener(Self::on_move_down))
            .capture_action(cx.listener(Self::on_enter))
            .capture_action(cx.listener(Self::on_escape))
            .capture_key_down(cx.listener(Self::on_key_down))
            .on_key_up(cx.listener(Self::on_key_up))
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(14.))
            .border_1()
            .border_color(theme::border_strong())
            .bg(theme::raised())
            .text_color(theme::text())
            .font_family(theme::SANS)
            .child(self.input_row())
            .child(self.results(cx))
            .children(status)
            .child(self.footer(cx))
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{Keystroke, PlatformInput, Point, TestAppContext, WindowBounds};
    use grove_core::{
        Change, ChangedFile, LaunchError, Outcome, RepoSnapshot, Settings, SettingsFile, Upstream,
        WorktreeStatus,
    };
    use tempfile::TempDir;

    use super::*;
    use crate::main_window::Status;
    use crate::shell::Library;
    use crate::test_support::{
        canned, code, fake_launcher, fake_presence, set_library, settings_file, terminal_window,
    };

    fn open_switcher(
        cx: &mut TestAppContext,
        file: &SettingsFile,
        repo: RepoSnapshot,
    ) -> (AnyWindowHandle, Entity<Switcher>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            set_library(file, vec![repo], cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(SIZE.0), px(SIZE.1)),
                })),
                ..Default::default()
            };
            open_with(options, cx).unwrap()
        })
    }

    fn keys(cx: &mut TestAppContext, w: AnyWindowHandle, keys: &[&str]) {
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            for k in keys {
                window.press(k, cx);
            }
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn typed(cx: &mut TestAppContext, w: AnyWindowHandle, text: &str) {
        cx.update_window(w, |_, window, cx| window.input(text, cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn is_open(cx: &mut TestAppContext, w: AnyWindowHandle) -> bool {
        cx.update(|cx| cx.windows().contains(&w))
    }

    fn shown(cx: &mut TestAppContext, s: &Entity<Switcher>) -> Vec<PathBuf> {
        s.read_with(cx, |s, _| s.hits.iter().map(|h| h.path.clone()).collect())
    }

    fn status(cx: &mut TestAppContext, s: &Entity<Switcher>) -> Option<Notice> {
        s.read_with(cx, |s, _| s.status.clone())
    }

    /// The main window's status bar line, as posted.
    fn posted(cx: &mut TestAppContext) -> Option<Status> {
        cx.update(|cx| {
            cx.try_global::<shell::Notices>()
                .and_then(|n| n.last.clone())
        })
    }

    fn recents(file: &SettingsFile) -> Vec<PathBuf> {
        file.load().unwrap().recents
    }

    fn paths(paths: &[&str]) -> Vec<PathBuf> {
        paths.iter().map(PathBuf::from).collect()
    }

    #[gpui_kit::test]
    fn typing_filters_and_enter_opens_in_editor(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, s) = open_switcher(cx, &file, canned());
        assert_eq!(shown(cx, &s).len(), 5);

        typed(cx, w, "api-y");
        assert_eq!(shown(cx, &s), paths(&["/c/wt/api-y"]));
        keys(cx, w, &["enter"]);
        assert_eq!(*launches.lock().unwrap(), [code("/c/wt/api-y")]);
        assert!(!is_open(cx, w));
        assert_eq!(recents(&file), paths(&["/c/wt/api-y"]));
        assert!(posted(cx).is_some_and(|s| s.text.starts_with("Opening ") && !s.error));
        // Seen in the switcher: a main window opened next doesn't replay it as in progress.
        assert_eq!(cx.update(|cx| shell::unseen_notice(cx)), None);
    }

    #[gpui_kit::test]
    fn alt_backspace_deletes_a_word(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let (w, s) = open_switcher(cx, &file, canned());
        typed(cx, w, "api x");
        keys(cx, w, &["alt-backspace"]);
        assert_eq!(s.read_with(cx, |s, _| s.query.clone()), "api ");
    }

    #[gpui_kit::test]
    fn click_elsewhere_close_guards_reopen_and_keeps_focus(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let (_, _) = open_switcher(cx, &file, canned());
        let closes: Rc<RefCell<Vec<Option<bool>>>> = Default::default();
        cx.update(|cx| {
            let closes = closes.clone();
            cx.on_window_closed(move |cx, id| closes.borrow_mut().push(closed(id, cx)))
                .detach();
        });
        let current = |cx: &mut TestAppContext| cx.update(|cx| window(cx));

        // The test platform doesn't deactivate windows; close as `close_on_deactivate` does.
        assert!(cx.update(|cx| toggle(Instant::now(), cx)));
        let w = current(cx).unwrap();
        cx.update_window(w, |_, window, _| window.remove_window())
            .unwrap();
        cx.run_until_parked();
        assert_eq!(*closes.borrow(), [Some(false)]);
        // The click on Jump to… that deactivated it lands right after.
        assert!(!cx.update(|cx| toggle(Instant::now(), cx)));
        assert!(cx.update(|cx| toggle(Instant::now() + REOPEN_GUARD, cx)));

        // Dismissed (Esc): reopens at once.
        let w = current(cx).unwrap();
        keys(cx, w, &["escape"]);
        assert_eq!(closes.borrow()[1], Some(true));
        assert!(cx.update(|cx| toggle(Instant::now(), cx)));
        // A toggle while open dismisses too.
        assert!(!cx.update(|cx| toggle(Instant::now(), cx)));
        cx.run_until_parked();
        assert_eq!(closes.borrow()[2], Some(true));
    }

    #[gpui_kit::test]
    fn empty_query_lists_recents_first_and_cmd_enter_opens_terminal(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(
            &tmp,
            Settings {
                recents: paths(&["/c/wt/api-z", "/c/wt/api-x"]),
                ..Default::default()
            },
        );
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, s) = open_switcher(cx, &file, canned());
        assert_eq!(
            shown(cx, &s),
            paths(&[
                "/c/wt/api-z",
                "/c/wt/api-x",
                "/c/api",
                "/c/wt/api-y",
                "/c/wt/bisect"
            ])
        );

        keys(cx, w, &["up", "down", "down", "up", "cmd-enter"]);
        let got = launches.lock().unwrap().clone();
        assert_eq!(got.len(), 1);
        terminal_window(&got[0], "/c/wt/api-x");
        assert!(!is_open(cx, w));
        assert_eq!(recents(&file), paths(&["/c/wt/api-x", "/c/wt/api-z"]));
    }

    #[gpui_kit::test]
    fn alt_enter_copies_path_then_closes(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, s) = open_switcher(cx, &file, canned());
        keys(cx, w, &["down", "alt-enter"]);
        assert_eq!(
            cx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
            Some("/c/wt/api-x")
        );
        assert_eq!(status(cx, &s).unwrap().text, "Copied path /c/wt/api-x");
        assert!(launches.lock().unwrap().is_empty());
        assert!(is_open(cx, w));

        cx.executor().advance_clock(COPY_CLOSE_DELAY);
        cx.run_until_parked();
        assert!(!is_open(cx, w));
        assert!(recents(&file).is_empty(), "copying isn't opening");
    }

    #[gpui_kit::test]
    fn escape_closes_without_launching(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, _) = open_switcher(cx, &file, canned());
        typed(cx, w, "x");
        keys(cx, w, &["escape"]);
        assert!(!is_open(cx, w));
        assert!(launches.lock().unwrap().is_empty());
    }

    #[gpui_kit::test]
    fn missing_worktree_is_refused_and_switcher_stays_open(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        fake_presence(cx, "/c/wt/api-x", Presence::Missing);
        let mut repo = canned();
        repo.worktrees[1].presence = Presence::Missing; // /c/wt/api-x
        let (w, s) = open_switcher(cx, &file, repo);
        typed(cx, w, "api-x");
        keys(cx, w, &["enter", "cmd-enter"]);
        assert!(launches.lock().unwrap().is_empty());
        assert_eq!(
            status(cx, &s),
            Some(Notice {
                text: "Can't open /c/wt/api-x: folder is missing (run git worktree prune)".into(),
                warn: true,
            })
        );
        assert!(is_open(cx, w));
        assert!(recents(&file).is_empty());
    }

    #[gpui_kit::test]
    fn launch_failure_shows_reason_and_stays_open(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let fail = LaunchError::Exit {
            program: "code".into(),
            status: "exit status: 1".into(),
            stderr: "bad option".into(),
        };
        let launches = fake_launcher(cx, Err(fail));
        let (w, s) = open_switcher(cx, &file, canned());
        keys(cx, w, &["enter"]);
        assert_eq!(launches.lock().unwrap().len(), 1);
        assert_eq!(
            status(cx, &s),
            Some(Notice {
                text: "code failed: bad option".into(),
                warn: true,
            })
        );
        assert!(is_open(cx, w));
        assert_eq!(
            posted(cx),
            Some(Status {
                text: "code failed: bad option".into(),
                error: true,
                settings: false,
            })
        );
    }

    #[gpui_kit::test]
    fn enter_is_ignored_while_launching_and_while_held(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let fail = LaunchError::Exit {
            program: "code".into(),
            status: "exit status: 1".into(),
            stderr: "bad option".into(),
        };
        let launches = fake_launcher(cx, Err(fail));
        let (w, s) = open_switcher(cx, &file, canned());
        // Both before the first launch's outcome lands.
        cx.update_window(w, |_, window, cx| {
            window.press("enter", cx);
            window.press("cmd-enter", cx);
        })
        .unwrap();
        assert_eq!(launches.lock().unwrap().len(), 1);
        let line = status(cx, &s).unwrap().text;
        assert!(line.starts_with("Opening ") && line.ends_with(" Still opening…"));
        cx.run_until_parked();

        // The failure re-enables Enter, but not its key repeat.
        let enter = Keystroke::parse("enter").unwrap();
        cx.update_window(w, |_, window, cx| {
            for is_held in [false, true, true] {
                let down = KeyDownEvent {
                    keystroke: enter.clone(),
                    is_held,
                    prefer_character_input: false,
                };
                window.dispatch_event(PlatformInput::KeyDown(down), cx);
            }
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(launches.lock().unwrap().len(), 2);
        cx.update_window(w, |_, window, cx| {
            let up = KeyUpEvent { keystroke: enter };
            window.dispatch_event(PlatformInput::KeyUp(up), cx);
        })
        .unwrap();
        keys(cx, w, &["enter"]);
        assert_eq!(launches.lock().unwrap().len(), 3);
    }

    #[gpui_kit::test]
    fn nothing_matches_shows_hint_and_backspace_edits(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, s) = open_switcher(cx, &file, canned());
        let hint = |cx: &mut TestAppContext| {
            cx.update_window(w, |_, window, cx| {
                window.render_frame(cx);
                window.try_find("nothing-matches").is_some()
            })
            .unwrap()
        };
        assert!(!hint(cx));
        typed(cx, w, "fyq");
        assert!(shown(cx, &s).is_empty());
        assert!(hint(cx));
        keys(cx, w, &["enter"]);
        assert!(launches.lock().unwrap().is_empty());

        keys(cx, w, &["backspace"]);
        assert_eq!(shown(cx, &s), paths(&["/c/wt/api-y"]));
        keys(cx, w, &["cmd-backspace"]);
        assert_eq!(shown(cx, &s).len(), 5);
    }

    #[gpui_kit::test]
    fn rows_show_change_count_and_follow_status_reads(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let (w, _) = open_switcher(cx, &file, canned());
        let badge = |cx: &mut TestAppContext| {
            cx.update_window(w, |_, window, cx| {
                window.render_frame(cx);
                window.try_find("switcher-badge-1-0").is_some()
            })
            .unwrap()
        };
        assert!(!badge(cx));
        cx.update(|cx| {
            let store = &mut cx.global_mut::<Library>().store;
            let stamp = store.status_reads().stamp();
            let files = (0..3)
                .map(|i| ChangedFile {
                    change: Change::Modified,
                    path: format!("f{i}"),
                    from: None,
                })
                .collect();
            store.apply_status(
                stamp,
                Path::new("/c/wt/api-x"),
                Ok(WorktreeStatus {
                    files,
                    upstream: Upstream::None,
                    head: None,
                    head_time: None,
                }),
            );
        });
        cx.run_until_parked();
        assert!(badge(cx));
    }
}
