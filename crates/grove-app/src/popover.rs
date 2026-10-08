//! Menu bar popover: every worktree by repo with its state and open buttons.

use std::path::Path;
use std::time::Duration;

use gpui_kit::base::Button;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext, Bounds, ClickEvent, Context, DisplayId,
    ElementId, Entity, FocusHandle, FontWeight, HighlightStyle, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled,
    StyledText, Subscription, TestSupportExt, Window, WindowOptions, div, point, px, size,
};
use grove_core::{MenuRow, Presence, Role, menu};

use crate::icons::{self, icon};
use crate::launcher::{self, Notice};
use crate::status_item::Anchor;
use crate::windows::{close_on_deactivate, panel_options};
use crate::{shell, theme};

const SIZE: (f32, f32) = (380., 560.);
/// Long enough to read the confirmation line before the popover closes.
const CLOSE_DELAY: Duration = Duration::from_millis(1200);

/// Opens below the status item, on the screen whose menu bar was clicked.
pub fn open(anchor: Anchor, cx: &mut App) -> Option<AnyWindowHandle> {
    let display = anchor
        .screen
        .and_then(|s| cx.find_display(DisplayId::from(s.id)));
    // Without the display, coordinates fall back to the primary screen's.
    let anchor = Anchor {
        screen: anchor.screen.filter(|_| display.is_some()),
        ..anchor
    };
    let (x, y) = place(anchor, SIZE.0);
    let bounds = Bounds::new(point(px(x), px(y)), size(px(SIZE.0), px(SIZE.1)));
    open_with(panel_options(bounds, display.map(|d| d.id())), cx)
        .map(|(handle, _)| handle)
        .inspect_err(|e| eprintln!("[grove] popover: {e:#}"))
        .ok()
}

/// Top-left of a `width`-wide popover in its screen's coordinates (window bounds are
/// relative to their display): centered under the status item, 8pt inside the screen.
fn place(anchor: Anchor, width: f32) -> (f32, f32) {
    let (sx, sy, sw) = anchor.screen.map_or((0., 0., f32::INFINITY), |s| {
        (s.x as f32, s.y as f32, s.width as f32)
    });
    let x = (anchor.x - sx + anchor.width / 2. - width / 2.)
        .min(sw - width - 8.)
        .max(8.);
    (x, anchor.y - sy + anchor.height + 4.)
}

fn open_with(
    options: WindowOptions,
    cx: &mut App,
) -> gpui_kit::Result<(AnyWindowHandle, Entity<Popover>)> {
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| {
            close_on_deactivate(window, cx);
            Popover::new(window, cx)
        })
    })
}

struct Popover {
    window: AnyWindowHandle,
    focus: FocusHandle,
    /// Confirmation or failure of the last action.
    status: Option<Notice>,
    /// Bumped per action so a slow launch can't overwrite a newer line or close.
    seq: u64,
    /// The line of a launch in flight; further opens wait for its outcome.
    launching: Option<String>,
    /// The last Fetch all report shown (`Fetcher::finished`).
    fetch_seen: u64,
    _subs: [Subscription; 2],
}

impl Popover {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        // Refreshes and status reads landing update rows in place.
        let library = cx.observe_global::<shell::Library>(|_, cx| cx.notify());
        let fetcher = cx.observe_global::<shell::Fetcher>(|this, cx| this.fetch_changed(cx));
        // E.g. a launch warning that landed after the last popover closed.
        let status = shell::unseen_notice(cx).map(|s| Notice {
            text: s.text,
            warn: s.error,
        });
        if status.is_some() {
            shell::notice_shown(cx);
        }
        Self {
            window: window.window_handle(),
            focus,
            status,
            seq: 0,
            launching: None,
            fetch_seen: cx.try_global::<shell::Fetcher>().map_or(0, |f| f.finished),
            _subs: [library, fetcher],
        }
    }

    /// Closes the popover. Deferred: from its own handlers, its window is mid-update.
    fn close(&self, cx: &mut App) {
        let window = self.window;
        cx.defer(move |cx| shell::close(window, cx));
    }

    /// Closes after `CLOSE_DELAY` unless another action came first.
    fn close_soon(&self, cx: &mut Context<Self>) {
        let seq = self.seq;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(CLOSE_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                if this.seq == seq {
                    this.close(cx);
                }
            });
        })
        .detach();
    }

    fn show(&mut self, notice: Notice, cx: &mut Context<Self>) {
        self.status = Some(notice);
        cx.notify();
    }

    /// Shows each finished Fetch all's outcome once. The list itself shows the new counts,
    /// so the popover stays open.
    fn fetch_changed(&mut self, cx: &mut Context<Self>) {
        let fetcher = cx.global::<shell::Fetcher>();
        if fetcher.finished != self.fetch_seen {
            self.fetch_seen = fetcher.finished;
            if let Some(report) = &fetcher.report {
                let notice = Notice {
                    text: report.message(),
                    warn: !report.failures.is_empty(),
                };
                self.seq += 1;
                self.status = Some(notice);
            }
        }
        cx.notify();
    }

    /// Opens `path`; closes shortly after the launch went through, or stays open to show
    /// why it didn't. Every line also reaches the main window's status bar.
    fn open(&mut self, path: &Path, presence: Presence, role: Role, cx: &mut Context<Self>) {
        if let Some(line) = &self.launching {
            let notice = Notice {
                text: format!("{line} Still opening…"),
                warn: false,
            };
            return self.show(notice, cx);
        }
        self.seq += 1;
        let seq = self.seq;
        let opening = match launcher::open_worktree(path, presence, role, None, cx) {
            Ok(opening) => opening,
            Err(e) => {
                let notice = Notice {
                    text: e.message,
                    warn: true,
                };
                notice.post(e.settings, cx);
                shell::notice_shown(cx);
                return self.show(notice, cx);
            }
        };
        let notice = Notice {
            text: opening.message.clone(),
            warn: false,
        };
        notice.post(false, cx);
        shell::notice_shown(cx);
        self.launching = Some(notice.text.clone());
        self.show(notice, cx);
        cx.spawn(async move |this, cx| {
            let notice = opening.finish().await;
            cx.update(|cx| {
                if let Some(n) = &notice {
                    n.post(false, cx);
                }
                // Gone (closed on a click elsewhere): the post above reaches the main window.
                let _ = this.update(cx, |this, cx| {
                    let line = this.launching.take();
                    let warn = notice.as_ref().is_some_and(|n| n.warn);
                    if this.seq != seq && !warn {
                        return;
                    }
                    // No follow-up: replaces a "Still opening…" with the launch's own line.
                    let notice = notice.or(line.map(|text| Notice { text, warn: false }));
                    if let Some(n) = notice {
                        shell::notice_shown(cx);
                        this.show(n, cx);
                    }
                    if !warn {
                        this.close_soon(cx);
                    }
                });
            });
        })
        .detach();
    }

    fn search(&mut self, cx: &mut Context<Self>) {
        self.close(cx);
        shell::toggle_switcher(cx);
    }

    fn fetch(&mut self, cx: &mut Context<Self>) {
        // The Busy button stays clickable; leave the line and any auto-close alone.
        if cx.try_global::<shell::Fetcher>().is_some_and(|f| f.running) {
            return;
        }
        self.seq += 1;
        self.status = None;
        shell::fetch_all(cx);
        cx.notify();
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        let m = &ks.modifiers;
        match ks.key.as_str() {
            "escape" => self.close(cx),
            "tab" if m.shift => window.focus_prev(cx),
            "tab" => window.focus_next(cx),
            "k" if m.platform => self.search(cx),
            _ => return,
        }
        cx.stop_propagation();
    }
}

/// The editor's and terminal's names, for the open buttons' labels.
struct Apps {
    editor: String,
    terminal: String,
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Ready,
    /// Working; looks inert but stays focusable (a disabled `Button` drops focus, and with
    /// it the popover's keyboard handling). Its action must no-op meanwhile.
    Busy,
    /// Can't open, and why.
    Blocked(&'static str),
}

/// Square icon button with an accessible label (also its tooltip, or the blocked reason).
fn icon_button(
    id: impl Into<ElementId>,
    path: &'static str,
    label: SharedString,
    mode: Mode,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    let blocked = match mode {
        Mode::Blocked(reason) => Some(reason),
        _ => None,
    };
    let tip: SharedString = match blocked {
        Some(reason) => format!("Can't open: {reason}").into(),
        None => label.clone(),
    };
    Button::new(id)
        .accessibility_label(label)
        .disabled(blocked.is_some())
        .flex_none()
        .size(px(32.))
        .rounded(px(7.))
        .border_1()
        .border_color(theme::border_strong())
        .focus_visible(|s| s.border_color(theme::link()))
        .when_some(blocked, |b, reason| b.aria_description(reason))
        .when_else(
            mode == Mode::Ready,
            |b| {
                b.bg(theme::raised())
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::border()))
                    .child(icon(path, 14.).text_color(theme::text()))
            },
            |b| b.child(icon(path, 14.).text_color(theme::faint())),
        )
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .on_click(on_click)
}

/// Footer text button.
fn text_button(
    id: &'static str,
    label: &'static str,
    color: gpui_kit::Rgba,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    Button::new(id)
        .h(px(28.))
        .px(px(8.))
        .gap(px(6.))
        .rounded(px(6.))
        .border_1()
        .border_color(gpui_kit::transparent_black())
        .focus_visible(|s| s.border_color(theme::link()))
        .text_size(px(13.))
        .text_color(color)
        .cursor_pointer()
        .hover(|s| s.bg(theme::raised()))
        .child(label)
        .on_click(on_click)
}

impl Popover {
    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let fetching = cx.try_global::<shell::Fetcher>().is_some_and(|f| f.running);
        let fetch_label = if fetching { "Fetching…" } else { "Fetch all" };
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.))
            .pl(px(16.))
            .pr(px(12.))
            .pt(px(12.))
            .pb(px(8.))
            .child(
                div()
                    .flex_1()
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Worktrees"),
            )
            .when(fetching, |d| {
                d.child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme::muted())
                        .child("Fetching…"),
                )
            })
            .child(icon_button(
                "popover-search",
                icons::SEARCH,
                "Search".into(),
                Mode::Ready,
                cx.listener(|this, _, _, cx| this.search(cx)),
            ))
            .child(icon_button(
                "popover-fetch",
                icons::REFRESH,
                fetch_label.into(),
                if fetching { Mode::Busy } else { Mode::Ready },
                cx.listener(|this, _, _, cx| this.fetch(cx)),
            ))
    }

    fn row(&self, ix: usize, row: &MenuRow, apps: &Apps, cx: &mut Context<Self>) -> AnyElement {
        let mode = row.presence.reason().map_or(Mode::Ready, Mode::Blocked);
        let button = |id: &'static str, path, app: &str, role: Role| {
            let label = format!("Open {} in {app}", row.name).into();
            let (wt, presence) = (row.path.clone(), row.presence);
            icon_button(
                (id, ix),
                path,
                label,
                mode,
                cx.listener(move |this, _, _, cx| this.open(&wt, presence, role, cx)),
            )
        };
        let mut a11y = format!("{}, {}", row.name, row.state.text);
        if row.last_opened {
            a11y.push_str(", last opened");
        }
        let state = StyledText::new(row.state.text.clone()).with_highlights(
            row.state.changed.clone().map(|r| {
                let color = Some(theme::accent().into());
                (
                    r,
                    HighlightStyle {
                        color,
                        ..Default::default()
                    },
                )
            }),
        );
        div()
            .id(("popover-row", ix))
            .role(gpui_kit::Role::ListItem)
            .aria_label(a11y)
            .flex()
            .items_center()
            .gap(px(4.))
            .pl(px(10.))
            .pr(px(6.))
            .py(px(6.))
            .rounded(px(8.))
            .when(row.last_opened, |d| d.bg(theme::selected()))
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
                            .items_baseline()
                            .gap(px(8.))
                            .child(
                                theme::mono(row.name.clone())
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(13.))
                                    .text_color(if row.presence == Presence::Unavailable {
                                        theme::faint()
                                    } else {
                                        theme::text()
                                    }),
                            )
                            .when(row.last_opened, |d| {
                                d.child(
                                    div()
                                        .id(("popover-last-opened", ix))
                                        .flex_none()
                                        .text_size(px(11.))
                                        .text_color(theme::faint())
                                        .child("last opened")
                                        .test_support(),
                                )
                            }),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(theme::muted())
                            .child(state),
                    ),
            )
            .child(button(
                "popover-editor",
                icons::EDITOR,
                &apps.editor,
                Role::Editor,
            ))
            .child(button(
                "popover-terminal",
                icons::TERMINAL,
                &apps.terminal,
                Role::Terminal,
            ))
            .test_support()
            .into_any_element()
    }

    fn list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let library = shell::library(cx);
        let groups = menu(
            library.store.snapshot(),
            &library.settings.pinned_repos,
            &library.settings.last_opened,
            library.settings.repo_sort,
            &library.settings.recents,
        );
        let app = |role, fallback: &str| {
            launcher::target_name(role, cx).unwrap_or_else(|| fallback.to_string())
        };
        let apps = Apps {
            editor: app(Role::Editor, "editor"),
            terminal: app(Role::Terminal, "terminal"),
        };
        let mut items: Vec<AnyElement> = Vec::new();
        let mut ix = 0;
        for group in &groups {
            items.push(
                div()
                    .px(px(8.))
                    .pt(px(12.))
                    .pb(px(4.))
                    .text_size(px(11.))
                    .text_color(theme::faint())
                    .child(group.repo.to_uppercase())
                    .into_any_element(),
            );
            for row in &group.rows {
                items.push(self.row(ix, row, &apps, cx));
                ix += 1;
            }
        }
        if groups.is_empty() {
            items.push(
                div()
                    .px(px(8.))
                    .py(px(24.))
                    .text_size(px(13.))
                    .text_color(theme::muted())
                    .child("No worktrees yet.")
                    .into_any_element(),
            );
        }
        div()
            .id("popover-rows")
            .role(gpui_kit::Role::List)
            .aria_label("Worktrees")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(8.))
            .pb(px(8.))
            .flex()
            .flex_col()
            .children(items)
    }

    fn footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.))
            .px(px(8.))
            .py(px(6.))
            .border_t_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .child(
                text_button(
                    "popover-open-grove",
                    "Open Grove",
                    theme::link(),
                    cx.listener(|this, _, _, cx| {
                        this.close(cx);
                        shell::show_main(cx);
                    }),
                )
                .child(
                    div()
                        .font_family(theme::MONO)
                        .text_size(px(11.))
                        .text_color(theme::faint())
                        .child("⌥⌘G"),
                ),
            )
            .child(div().flex_1())
            .child(text_button(
                "popover-settings",
                "Settings…",
                theme::muted(),
                cx.listener(|this, _, _, cx| {
                    this.close(cx);
                    shell::open_settings(cx);
                }),
            ))
            .child(text_button(
                "popover-quit",
                "Quit",
                theme::muted(),
                |_, _, cx| shell::quit(cx),
            ))
    }
}

impl Render for Popover {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = self.status.clone().map(|n| {
            div()
                .id("popover-status")
                .flex_none()
                .px(px(16.))
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
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(12.))
            .border_1()
            .border_color(theme::border_strong())
            .bg(theme::raised())
            .text_color(theme::text())
            .font_family(theme::SANS)
            .child(self.header(cx))
            .child(self.list(cx))
            .children(status)
            .child(self.footer(cx))
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{Point, TestAppContext};
    use grove_core::{
        Change, ChangedFile, LaunchError, Outcome, RepoSnapshot, Settings, SettingsFile, Upstream,
        WorktreeStatus,
    };
    use tempfile::TempDir;

    use super::*;
    use crate::main_window::Status;
    use crate::shell::Library;
    use crate::test_support::{
        canned, code, fake_launcher, fake_presence, set_library, settings_file, tab_to,
        terminal_window,
    };

    fn open_popover(
        cx: &mut TestAppContext,
        file: &SettingsFile,
        repos: Vec<RepoSnapshot>,
    ) -> (AnyWindowHandle, Entity<Popover>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            set_library(file, repos, cx);
            let options = WindowOptions {
                window_bounds: Some(gpui_kit::WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(SIZE.0), px(SIZE.1)),
                })),
                ..Default::default()
            };
            open_with(options, cx).unwrap()
        })
    }

    fn click(cx: &mut TestAppContext, w: AnyWindowHandle, id: impl Into<ElementId>) {
        let id = id.into();
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        })
        .unwrap();
        cx.run_until_parked();
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

    fn label(cx: &mut TestAppContext, w: AnyWindowHandle, id: impl Into<ElementId>) -> String {
        let id = id.into();
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.find(id).label().unwrap_or_default().to_string()
        })
        .unwrap()
    }

    fn is_open(cx: &mut TestAppContext, w: AnyWindowHandle) -> bool {
        cx.update(|cx| cx.windows().contains(&w))
    }

    fn status(cx: &mut TestAppContext, p: &Entity<Popover>) -> Option<Notice> {
        p.read_with(cx, |p, _| p.status.clone())
    }

    fn posted(cx: &mut TestAppContext) -> Option<Status> {
        cx.update(|cx| {
            cx.try_global::<shell::Notices>()
                .and_then(|n| n.last.clone())
        })
    }

    fn recents(file: &SettingsFile) -> Vec<PathBuf> {
        file.load().unwrap().recents
    }

    /// Accessibility labels of the rendered rows, in order.
    fn rows(cx: &mut TestAppContext, w: AnyWindowHandle) -> Vec<String> {
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            (0usize..)
                .map_while(|ix| window.try_find(("popover-row", ix)))
                .map(|r| r.label().unwrap_or_default().to_string())
                .collect()
        })
        .unwrap()
    }

    /// The rows said to be last opened.
    fn highlighted(cx: &mut TestAppContext, w: AnyWindowHandle) -> Vec<String> {
        rows(cx, w)
            .into_iter()
            .filter(|r| r.ends_with(", last opened"))
            .collect()
    }

    /// Rows in display order: main, feature/x, feature/y, fix/z, bisect (detached).
    const API_X: usize = 1;
    const BISECT: usize = 4;

    #[test]
    fn placed_under_the_status_item_within_its_screen() {
        let at = |x, y, screen: (f64, f64, f64)| {
            let screen = Some(crate::macos::Screen {
                id: 1,
                x: screen.0,
                y: screen.1,
                width: screen.2,
                scale: 2.,
            });
            let anchor = Anchor {
                x,
                y,
                width: 30.,
                height: 24.,
                screen,
            };
            place(anchor, 380.)
        };
        let primary = (0., 0., 1512.);
        assert_eq!(at(700., 0., primary), (525., 28.));
        assert_eq!(at(1480., 0., primary), (1124., 28.));
        // An external display to the right, its top 200pt above the primary's.
        let right = (1512., -200., 2560.);
        assert_eq!(at(2500., -200., right), (813., 28.));
        assert_eq!(at(4000., -200., right), (2172., 28.));
        // One to the left, at negative x.
        let left = (-1920., 0., 1920.);
        assert_eq!(at(-1000., 0., left), (745., 28.));
        assert_eq!(at(-60., 0., left), (1532., 28.));
        assert_eq!(at(-1915., 0., left), (8., 28.));
    }

    #[gpui_kit::test]
    fn buttons_name_branch_and_app(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let (w, _) = open_popover(cx, &file, vec![canned()]);
        assert_eq!(
            label(cx, w, ("popover-editor", API_X)),
            "Open feature/x in VS Code"
        );
        assert_eq!(
            label(cx, w, ("popover-terminal", API_X)),
            "Open feature/x in Terminal"
        );
        assert_eq!(
            label(cx, w, ("popover-editor", BISECT)),
            "Open bisect in VS Code"
        );
        assert_eq!(label(cx, w, "popover-search"), "Search");
        assert_eq!(label(cx, w, "popover-fetch"), "Fetch all");
    }

    #[gpui_kit::test]
    fn editor_button_opens_confirms_then_closes(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, p) = open_popover(cx, &file, vec![canned()]);
        assert!(highlighted(cx, w).is_empty());

        click(cx, w, ("popover-editor", API_X));
        assert_eq!(*launches.lock().unwrap(), [code("/c/wt/api-x")]);
        assert_eq!(recents(&file), [PathBuf::from("/c/wt/api-x")]);
        let opened = file.load().unwrap().last_opened;
        assert_eq!(
            opened.keys().collect::<Vec<_>>(),
            [Path::new("/c/wt/api-x")]
        );
        assert_eq!(
            highlighted(cx, w),
            ["feature/x, checking status…, last opened"]
        );
        let line = Notice {
            text: "Opening /c/wt/api-x in VS Code…".into(),
            warn: false,
        };
        assert_eq!(status(cx, &p), Some(line.clone()));
        assert_eq!(posted(cx).map(|s| s.text), Some(line.text));
        // Seen here: a main window opened next doesn't replay it.
        assert_eq!(cx.update(|cx| shell::unseen_notice(cx)), None);

        assert!(is_open(cx, w));
        cx.executor().advance_clock(CLOSE_DELAY);
        cx.run_until_parked();
        assert!(!is_open(cx, w));
    }

    #[gpui_kit::test]
    fn terminal_button_opens_default_terminal(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, _) = open_popover(cx, &file, vec![canned()]);
        click(cx, w, ("popover-terminal", BISECT));
        let got = launches.lock().unwrap().clone();
        assert_eq!(got.len(), 1);
        terminal_window(&got[0], "/c/wt/bisect");
        assert_eq!(recents(&file), [PathBuf::from("/c/wt/bisect")]);
    }

    #[gpui_kit::test]
    fn most_recently_opened_is_highlighted(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let recents = vec![PathBuf::from("/c/wt/api-z"), PathBuf::from("/c/api")];
        let file = settings_file(
            &tmp,
            Settings {
                recents,
                ..Default::default()
            },
        );
        fake_launcher(cx, Ok(Outcome::Opened));
        let (w, _) = open_popover(cx, &file, vec![canned()]);
        assert_eq!(highlighted(cx, w), ["fix/z, checking status…, last opened"]);
        // Said on screen too: only fix/z (row 3) has the visible tag.
        let tags: Vec<usize> = cx
            .update_window(w, |_, window, cx| {
                window.render_frame(cx);
                (0usize..5)
                    .filter(|&ix| window.try_find(("popover-last-opened", ix)).is_some())
                    .collect()
            })
            .unwrap();
        assert_eq!(tags, [3]);
    }

    #[gpui_kit::test]
    fn launch_failure_stays_visible_and_reaches_status_bar(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let fail = LaunchError::Exit {
            program: "code".into(),
            status: "exit status: 1".into(),
            stderr: "bad option".into(),
        };
        fake_launcher(cx, Err(fail));
        let (w, p) = open_popover(cx, &file, vec![canned()]);
        click(cx, w, ("popover-editor", 0usize));
        let failed = Notice {
            text: "code failed: bad option".into(),
            warn: true,
        };
        assert_eq!(status(cx, &p), Some(failed));
        assert_eq!(
            posted(cx),
            Some(Status {
                text: "code failed: bad option".into(),
                error: true,
                settings: false,
            })
        );
        cx.executor().advance_clock(CLOSE_DELAY * 2);
        cx.run_until_parked();
        assert!(is_open(cx, w));
    }

    #[gpui_kit::test]
    fn missing_and_unavailable_rows_cannot_open(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        fake_presence(cx, "/c/wt/api-x", Presence::Missing);
        let mut repo = canned();
        repo.worktrees[1].presence = Presence::Missing; // /c/wt/api-x
        repo.worktrees[2].presence = Presence::Unavailable; // /c/wt/api-y
        let (w, _) = open_popover(cx, &file, vec![repo]);
        for ix in [API_X, API_X + 1] {
            click(cx, w, ("popover-editor", ix));
            click(cx, w, ("popover-terminal", ix));
        }
        assert!(launches.lock().unwrap().is_empty());
        assert!(recents(&file).is_empty());
        assert!(is_open(cx, w));
    }

    #[gpui_kit::test]
    fn folder_gone_since_scan_is_refused_with_reason(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, p) = open_popover(cx, &file, vec![canned()]);
        fake_presence(cx, "/c/wt/api-x", Presence::Missing);
        click(cx, w, ("popover-editor", API_X));
        assert!(launches.lock().unwrap().is_empty());
        let text = "Can't open /c/wt/api-x: folder is missing (run git worktree prune)";
        assert_eq!(status(cx, &p).map(|n| n.text).as_deref(), Some(text));
        assert_eq!(posted(cx).map(|s| s.text).as_deref(), Some(text));
        // The row now shows it missing.
        assert_eq!(
            rows(cx, w)[API_X],
            "feature/x, folder is missing (run git worktree prune)"
        );
    }

    #[gpui_kit::test]
    fn search_closes_popover_and_opens_switcher(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let (w, _) = open_popover(cx, &file, vec![canned()]);
        click(cx, w, "popover-search");
        assert!(!is_open(cx, w));
        let switcher = cx.update(|cx| crate::switcher::window(cx));
        assert!(switcher.is_some_and(|s| is_open(cx, s)));
    }

    #[gpui_kit::test]
    fn fetch_all_shows_progress_then_report(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let (w, p) = open_popover(cx, &file, vec![canned()]);
        cx.update(|cx| cx.default_global::<shell::Fetcher>().running = true);
        assert_eq!(label(cx, w, "popover-fetch"), "Fetching…");
        cx.update(|cx| cx.global_mut::<shell::Fetcher>().running = false);

        // `/c/api` isn't a repo, so its fetch fails.
        click(cx, w, "popover-fetch");
        let n = status(cx, &p).unwrap();
        assert!(n.text.starts_with("Fetch failed for api: "), "{}", n.text);
        assert!(n.warn);
        assert_eq!(label(cx, w, "popover-fetch"), "Fetch all");
    }

    #[gpui_kit::test]
    fn open_grove_closes_popover_and_shows_main_window(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let (w, _) = open_popover(cx, &file, vec![canned()]);
        click(cx, w, "popover-open-grove");
        assert!(!is_open(cx, w));
        assert_eq!(cx.update(|cx| cx.windows().len()), 1, "the main window");
    }

    #[gpui_kit::test]
    fn keyboard_tabs_to_a_button_and_escape_closes(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, _) = open_popover(cx, &file, vec![canned()]);
        // Search, Fetch all, then main's editor button.
        keys(cx, w, &["tab", "tab", "tab", "enter"]);
        assert_eq!(*launches.lock().unwrap(), [code("/c/api")]);

        let (w, _) = open_popover(cx, &file, vec![canned()]);
        keys(cx, w, &["escape"]);
        assert!(!is_open(cx, w));
    }

    #[gpui_kit::test]
    fn tab_reaches_header_rows_and_footer_and_wraps(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let (w, _) = open_popover(cx, &file, vec![canned()]);
        let ids: [ElementId; 8] = [
            "popover-search".into(),
            "popover-fetch".into(),
            ("popover-editor", API_X).into(),
            ("popover-terminal", API_X).into(),
            "popover-open-grove".into(),
            "popover-settings".into(),
            "popover-quit".into(),
            "popover-search".into(),
        ];
        for id in ids {
            tab_to(cx, w, id);
        }
    }

    #[gpui_kit::test]
    fn unseen_notice_shows_when_popover_opens(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let hint = Status {
            text: "Allow Grove to control Terminal".into(),
            error: true,
            settings: false,
        };
        cx.update(|cx| shell::post(hint, cx));
        let (_, p) = open_popover(cx, &file, vec![canned()]);
        assert_eq!(
            status(cx, &p),
            Some(Notice {
                text: "Allow Grove to control Terminal".into(),
                warn: true,
            })
        );
        assert_eq!(cx.update(|cx| shell::unseen_notice(cx)), None);
    }

    #[gpui_kit::test]
    fn rows_follow_status_reads(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let (w, _) = open_popover(cx, &file, vec![canned()]);
        assert_eq!(rows(cx, w)[API_X], "feature/x, checking status…");
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
            let upstream = Upstream::Tracking {
                name: "origin/feature/x".into(),
                ahead: 2,
                behind: 1,
            };
            store.apply_status(
                stamp,
                Path::new("/c/wt/api-x"),
                Ok(WorktreeStatus {
                    files,
                    upstream,
                    head: None,
                    head_time: None,
                    changes_time: None,
                }),
            );
        });
        cx.run_until_parked();
        assert_eq!(rows(cx, w)[API_X], "feature/x, 3 changed · ↑2 ↓1");
    }

    #[gpui_kit::test]
    fn keyboard_survives_fetch_all_running(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let (w, _) = open_popover(cx, &file, vec![canned()]);
        keys(cx, w, &["tab", "tab"]);
        cx.update(|cx| cx.default_global::<shell::Fetcher>().running = true);
        keys(cx, w, &["escape"]);
        assert!(!is_open(cx, w));
    }

    /// Clicks each id in one update, so a launch from the first is still in flight.
    fn click_all(cx: &mut TestAppContext, w: AnyWindowHandle, ids: Vec<ElementId>) {
        cx.update_window(w, |_, window, cx| {
            for id in ids {
                window.render_frame(cx);
                window.click(id, cx);
            }
        })
        .unwrap();
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn still_opening_gives_way_to_the_launch_line(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, p) = open_popover(cx, &file, vec![canned()]);
        let ids = vec![
            ("popover-editor", API_X).into(),
            ("popover-terminal", API_X).into(),
        ];
        click_all(cx, w, ids);
        assert_eq!(launches.lock().unwrap().len(), 1);
        let line = status(cx, &p).unwrap().text;
        assert_eq!(line, "Opening /c/wt/api-x in VS Code…");
        cx.executor().advance_clock(CLOSE_DELAY);
        cx.run_until_parked();
        assert!(!is_open(cx, w));
    }

    #[gpui_kit::test]
    fn launch_failure_after_a_later_action_still_shows(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let fail = LaunchError::Exit {
            program: "code".into(),
            status: "exit status: 1".into(),
            stderr: "bad option".into(),
        };
        fake_launcher(cx, Err(fail));
        let (w, p) = open_popover(cx, &file, vec![canned()]);
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.click(("popover-editor", 0usize), cx);
        })
        .unwrap();
        // Any later action (only a launch or a fetch has one) before the launch finishes.
        p.update(cx, |p, _| p.seq += 1);
        cx.run_until_parked();
        let failed = Notice {
            text: "code failed: bad option".into(),
            warn: true,
        };
        assert_eq!(status(cx, &p), Some(failed));
        assert!(is_open(cx, w));
    }

    #[gpui_kit::test]
    fn busy_fetch_button_keeps_the_line(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, p) = open_popover(cx, &file, vec![canned()]);
        click(cx, w, ("popover-editor", API_X));
        assert_eq!(launches.lock().unwrap().len(), 1);
        let line = status(cx, &p);
        cx.update(|cx| cx.default_global::<shell::Fetcher>().running = true);
        click(cx, w, "popover-fetch");
        assert_eq!(status(cx, &p), line);
        // Its auto-close still comes.
        cx.executor().advance_clock(CLOSE_DELAY);
        cx.run_until_parked();
        assert!(!is_open(cx, w));
    }

    #[gpui_kit::test]
    fn quit_writes_pending_settings_first(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let (w, _) = open_popover(cx, &file, vec![canned()]);
        cx.update(|cx| shell::record_open(Path::new("/c/wt/api-y"), cx));
        // Without parking: the background save hasn't run.
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.click("popover-quit", cx);
        })
        .unwrap();
        assert_eq!(recents(&file), [PathBuf::from("/c/wt/api-y")]);
    }
}
