//! Settings: code folder, default editor and terminal, and the general toggles. Edits a
//! draft; Save applies it to every surface, Cancel drops it.

use std::path::Path;

use gpui_kit::base::Button;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext, Bounds, ClickEvent, Context, ElementId, Entity,
    FocusHandle, FontWeight, Global, InteractiveElement, IntoElement, KeyDownEvent, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, TitlebarOptions, Window,
    WindowBounds, WindowKind, WindowOptions, div, px, size,
};
use grove_core::{
    LoginItemStatus, NoTarget, OpenTarget, Preferences, Role, Snapshot, TargetKind,
    TerminalOpensIn, command_line,
};

use crate::icons::{self, icon};
use crate::login_item::{self, LOGIN_ITEMS, LoginItems};
use crate::main_window::tilde;
use crate::theme::{self, ButtonKind, button};
use crate::{setup, shell};

const SIZE: (f32, f32) = (820., 760.);

/// The open settings window, so Settings… brings it forward instead of opening another.
struct Open(AnyWindowHandle);

impl Global for Open {}

pub fn open(cx: &mut App) {
    cx.activate(true);
    if let Some(w) = window(cx) {
        let _ = w.update(cx, |_, window, _| window.activate_window());
        return;
    }
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(SIZE.0), px(SIZE.1)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some("Grove Settings".into()),
            ..Default::default()
        }),
        focus: true,
        show: true,
        kind: WindowKind::Normal,
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    };
    match open_with(options, cx) {
        Ok((handle, _)) => cx.set_global(Open(handle)),
        Err(e) => eprintln!("[grove] settings: {e:#}"),
    }
}

pub fn window(cx: &App) -> Option<AnyWindowHandle> {
    cx.try_global::<Open>()
        .map(|o| o.0)
        .filter(|w| cx.windows().contains(w))
}

fn open_with(
    options: WindowOptions,
    cx: &mut App,
) -> gpui_kit::Result<(AnyWindowHandle, Entity<SettingsWindow>)> {
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| SettingsWindow::new(window, cx))
    })
}

pub struct SettingsWindow {
    draft: Preferences,
    /// Scan of a newly chosen code folder; swapped in on Save.
    scanned: Option<Snapshot>,
    /// macOS's login item status for this copy; `None` until read, or when unbundled.
    login_status: Option<LoginItemStatus>,
    focus: FocusHandle,
}

impl SettingsWindow {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let status = login_item::status_now(cx);
        cx.spawn(async move |this, cx| {
            let status = status.await;
            let _ = this.update(cx, |this, cx| {
                this.login_status = status;
                cx.notify();
            });
        })
        .detach();
        Self {
            draft: shell::library(cx).settings.preferences(),
            scanned: None,
            login_status: None,
            focus,
        }
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut prefs = self.draft.clone();
        // Only a folder chosen here changes it (the sidebar may have meanwhile).
        if self.scanned.is_none() {
            prefs.code_dir = shell::library(cx).settings.code_dir.clone();
        }
        shell::save_preferences(prefs, self.scanned.take(), cx);
        window.remove_window();
    }

    fn change_folder(&mut self, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        setup::change_code_dir(
            move |snapshot, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.draft.code_dir = Some(snapshot.code_dir.clone());
                    let current = shell::library(cx).settings.code_dir.as_ref();
                    this.scanned = (current != Some(&snapshot.code_dir)).then_some(snapshot);
                    cx.notify();
                });
            },
            cx,
        );
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        match ks.key.as_str() {
            "escape" => window.remove_window(),
            "tab" if ks.modifiers.shift => window.focus_prev(cx),
            "tab" => window.focus_next(cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    /// The draft's default for `role`, as Enter or ⌘Enter would use it after Save.
    fn default_for<'a>(
        &self,
        role: Role,
        targets: &'a [OpenTarget],
    ) -> Result<&'a OpenTarget, NoTarget> {
        let settings = grove_core::Settings {
            default_editor: self.draft.default_editor.clone(),
            default_terminal: self.draft.default_terminal.clone(),
            ..Default::default()
        };
        grove_core::default_target(role, &settings, targets)
    }

    fn opens_in(&self, kind: TargetKind) -> Option<TerminalOpensIn> {
        let TargetKind::Terminal(t) = kind else {
            return None;
        };
        Some(if t.supports_tabs() {
            self.draft.terminal_opens_in.unwrap_or(t.default_opens_in())
        } else {
            TerminalOpensIn::Window
        })
    }

    /// The folder Save would leave Grove in, and its repo count: the live one unless
    /// another was chosen here.
    fn shown_folder<'a>(&'a self, cx: &'a App) -> (Option<&'a Path>, usize) {
        match &self.scanned {
            Some(s) => (Some(&s.code_dir), s.repos.len()),
            None => {
                let library = shell::library(cx);
                let repos = library.store.snapshot().repos.len();
                (library.settings.code_dir.as_deref(), repos)
            }
        }
    }

    fn code_folder(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (folder, repos) = self.shown_folder(cx);
        let folder = folder.map_or_else(|| "No folder chosen".into(), tilde);
        div()
            .flex()
            .items_center()
            .gap(px(14.))
            .px(px(16.))
            .py(px(14.))
            .rounded(px(10.))
            .border_1()
            .border_color(theme::border_strong())
            .bg(theme::raised())
            .child(icon(icons::FOLDER, 18.).text_color(theme::link()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(label("CODE FOLDER"))
                    .child(
                        div()
                            .flex()
                            .min_w_0()
                            .gap(px(6.))
                            .child(theme::path_text(folder).text_size(px(14.)))
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(px(12.))
                                    .text_color(theme::muted())
                                    .child(format!(
                                        "· {repos} repo{} · Grove scans nothing outside it",
                                        if repos == 1 { "" } else { "s" }
                                    )),
                            ),
                    ),
            )
            .child(button(
                "settings-change-folder",
                "Change…",
                ButtonKind::Secondary,
                Some(cx.listener(|this, _: &ClickEvent, _, cx| this.change_folder(cx))),
            ))
    }

    /// Detected apps for `role`, the draft's default selected.
    fn choices(&self, role: Role, cx: &mut Context<Self>) -> AnyElement {
        let (title, id) = match role {
            Role::Editor => ("EDITOR", "settings-editor"),
            Role::Terminal => ("TERMINAL", "settings-terminal"),
        };
        let targets = cx
            .try_global::<crate::launcher::Launcher>()
            .and_then(|l| l.targets.clone());
        let mut col = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(label(title).mb(px(4.)));
        let Some(targets) = targets else {
            return col.child(note("Looking for apps…")).into_any_element();
        };
        let chosen = self.default_for(role, &targets);
        if let Err(e @ NoTarget::NoneDetected(_)) = &chosen {
            return col.child(note(e.to_string())).into_any_element();
        }
        if let Err(e) = &chosen {
            col = col.child(note(format!("{e}. Pick another.")).text_color(theme::warn_text()));
        }
        let selected = chosen.ok().map(|t| t.kind);
        let rows = targets.iter().filter(|t| t.role() == role).enumerate();
        for (ix, target) in rows {
            let kind = target.kind;
            let on = selected == Some(kind);
            let detail = match self.opens_in(kind) {
                Some(TerminalOpensIn::Tab) => "new tab".to_string(),
                Some(TerminalOpensIn::Window) => "new window".to_string(),
                None => command_line(target).unwrap_or_default(),
            };
            let a11y = format!(
                "{}, {detail}{}",
                target.name(),
                if on { ", selected" } else { "" }
            );
            col = col.child(
                choice((id, ix), on, a11y)
                    .child(
                        div()
                            .flex_none()
                            .size(px(14.))
                            .rounded(px(7.))
                            .border_1()
                            .border_color(if on {
                                theme::accent()
                            } else {
                                theme::border_strong()
                            })
                            .when(on, |d| d.border_4()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(14.))
                            .child(target.name().to_string()),
                    )
                    .child(
                        theme::mono(detail)
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(theme::muted()),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        let id = Some(kind.id().to_string());
                        match role {
                            Role::Editor => this.draft.default_editor = id,
                            Role::Terminal => this.draft.default_terminal = id,
                        }
                        cx.notify();
                    })),
            );
        }
        if let (Role::Terminal, Some(TargetKind::Terminal(t))) = (role, selected) {
            col = col.child(self.opens_in_picker(t, cx));
        }
        col.into_any_element()
    }

    /// New tab or new window, for the default terminal where it supports both.
    fn opens_in_picker(&self, t: grove_core::Terminal, cx: &mut Context<Self>) -> AnyElement {
        if !t.supports_tabs() {
            return note(format!("{} opens new windows only.", t.name())).into_any_element();
        }
        let current = self.opens_in(TargetKind::Terminal(t));
        let option = |id: &'static str, text: &'static str, value: TerminalOpensIn| {
            let on = current == Some(value);
            Button::new(id)
                .accessibility_label(format!(
                    "Open in {text}{}",
                    if on { ", selected" } else { "" }
                ))
                .selected(on)
                .flex_1()
                .h(px(30.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .border_1()
                .border_color(gpui_kit::transparent_black())
                .focus_visible(|s| s.border_color(theme::link()))
                .text_size(px(13.))
                .cursor_pointer()
                .when_else(
                    on,
                    |b| b.bg(theme::selected()).text_color(theme::text()),
                    |b| {
                        b.text_color(theme::muted())
                            .hover(|s| s.bg(theme::border()))
                    },
                )
                .child(text)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.draft.terminal_opens_in = Some(value);
                    cx.notify();
                }))
        };
        div()
            .mt(px(6.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(label("OPENS IN"))
            .child(
                div()
                    .flex()
                    .gap(px(4.))
                    .p(px(3.))
                    .rounded(px(8.))
                    .bg(theme::field())
                    .border_1()
                    .border_color(theme::border())
                    .child(option(
                        "settings-opens-tab",
                        "New tab",
                        TerminalOpensIn::Tab,
                    ))
                    .child(option(
                        "settings-opens-window",
                        "New window",
                        TerminalOpensIn::Window,
                    )),
            )
            .when(
                t == grove_core::Terminal::Terminal && current == Some(TerminalOpensIn::Tab),
                |d| {
                    d.child(note(
                        "Terminal tabs also need Accessibility permission (Grove presses ⌘T).",
                    ))
                },
            )
            .into_any_element()
    }

    fn general(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let bundle = cx.try_global::<LoginItems>().and_then(|i| i.bundle.clone());
        let owner = shell::library(cx).settings.login_item_path.clone();
        let hint = login_hint(
            bundle.as_deref(),
            owner.as_deref(),
            self.login_status,
            self.draft.launch_at_login,
        );
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(label("GENERAL").mb(px(4.)))
            .child(self.toggle(
                "settings-menu-bar-icon",
                "Show menu bar icon",
                "Off hides it; Open Grove (⌥⌘G) and the quick switcher (⌥⌘K) still work.",
                self.draft.show_menu_bar_icon,
                |p| &mut p.show_menu_bar_icon,
                None,
                cx,
            ))
            .child(self.toggle(
                "settings-background-fetch",
                "Background fetch",
                "Fetch all every 15 minutes while Grove runs. Uses the network.",
                self.draft.background_fetch,
                |p| &mut p.background_fetch,
                None,
                cx,
            ))
            .child(self.toggle(
                "settings-launch-at-login",
                "Launch at login",
                "Keeps ⌥⌘K and ⌥⌘G ready after you log in.",
                self.draft.launch_at_login,
                |p| &mut p.launch_at_login,
                hint,
                cx,
            ))
    }

    #[allow(clippy::too_many_arguments)]
    fn toggle(
        &self,
        id: &'static str,
        title: &'static str,
        detail: &'static str,
        on: bool,
        field: fn(&mut Preferences) -> &mut bool,
        hint: Option<String>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        choice(
            id,
            false,
            format!("{title}, {}", if on { "on" } else { "off" }),
        )
        .py(px(10.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().text_size(px(14.)).child(title))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme::muted())
                        .child(detail),
                )
                .when_some(hint, |d, hint| {
                    d.child(
                        div()
                            .text_size(px(12.))
                            .text_color(theme::warn_text())
                            .child(hint),
                    )
                }),
        )
        .child(switch(on))
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            let value = field(&mut this.draft);
            *value = !*value;
            cx.notify();
        }))
    }

    fn footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let targets = cx
            .try_global::<crate::launcher::Launcher>()
            .and_then(|l| l.targets.clone())
            .unwrap_or_default();
        let name = |role| {
            self.default_for(role, &targets)
                .map_or("—".to_string(), |t| t.name().to_string())
        };
        let summary = format!(
            "↵ opens in {} · ⌘↵ opens in {}",
            name(Role::Editor),
            name(Role::Terminal)
        );
        div()
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(10.))
            .px(px(32.))
            .py(px(16.))
            .border_t_1()
            .border_color(theme::border())
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.))
                    .text_color(theme::muted())
                    .child(summary),
            )
            .child(
                div()
                    .flex()
                    .gap(px(10.))
                    .child(button(
                        "settings-cancel",
                        "Cancel",
                        ButtonKind::Secondary,
                        Some(|_: &ClickEvent, window: &mut Window, _: &mut App| {
                            window.remove_window()
                        }),
                    ))
                    .child(button(
                        "settings-save",
                        "Save",
                        ButtonKind::Primary,
                        Some(cx.listener(|this, _: &ClickEvent, window, cx| this.save(window, cx))),
                    )),
            )
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::panel())
            .text_color(theme::text())
            .child(
                div()
                    .id("settings-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(24.))
                    .px(px(32.))
                    .pt(px(28.))
                    .pb(px(24.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .text_size(px(22.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Settings"),
                            )
                            .child(div().text_size(px(14.)).text_color(theme::muted()).child(
                                "Stored on this Mac only. Nothing changes until you \
                                         Save.",
                            )),
                    )
                    .child(self.code_folder(cx))
                    .child(
                        div()
                            .flex()
                            .gap(px(24.))
                            .child(self.choices(Role::Editor, cx))
                            .child(self.choices(Role::Terminal, cx)),
                    )
                    .child(self.general(cx)),
            )
            .child(self.footer(cx))
    }
}

/// Under Launch at login: what Save can't do on its own, or will also do.
fn login_hint(
    bundle: Option<&Path>,
    owner: Option<&Path>,
    status: Option<LoginItemStatus>,
    on: bool,
) -> Option<String> {
    let Some(bundle) = bundle else {
        return Some("Only Grove.app can launch at login; this copy isn't one.".into());
    };
    let mut lines = Vec::new();
    if on && status == Some(LoginItemStatus::RequiresApproval) {
        lines.push(format!(
            "Waiting for your OK: allow Grove in {LOGIN_ITEMS}."
        ));
    }
    if let Some(other) = owner.filter(|o| *o != bundle) {
        lines.push(format!(
            "Saving moves launch at login to this copy; the one at {} may stay a login item \
             too (remove it in {LOGIN_ITEMS}).",
            tilde(other)
        ));
    }
    (!lines.is_empty()).then(|| lines.join(" "))
}

fn label(text: &'static str) -> gpui_kit::Div {
    div()
        .text_size(px(11.))
        .text_color(theme::faint())
        .child(text)
}

fn note(text: impl Into<SharedString>) -> gpui_kit::Div {
    div()
        .text_size(px(12.))
        .text_color(theme::muted())
        .child(text.into())
}

/// A selectable row: radio choice or toggle.
fn choice(id: impl Into<ElementId>, selected: bool, a11y: String) -> Button {
    Button::new(id)
        .accessibility_label(a11y)
        .selected(selected)
        .min_h(px(44.))
        .px(px(12.))
        .flex()
        .items_center()
        .gap(px(12.))
        .rounded(px(8.))
        .border_1()
        .border_color(gpui_kit::transparent_black())
        .focus_visible(|s| s.border_color(theme::link()))
        .cursor_pointer()
        .when_else(
            selected,
            |b| b.bg(theme::selected()),
            |b| b.bg(theme::raised()).hover(|s| s.bg(theme::border())),
        )
}

fn switch(on: bool) -> impl IntoElement {
    div()
        .flex_none()
        .w(px(34.))
        .h(px(20.))
        .p(px(2.))
        .flex()
        .rounded(px(10.))
        .when_else(
            on,
            |d| d.bg(theme::accent()).justify_end(),
            |d| d.bg(theme::border_strong()),
        )
        .child(div().size(px(16.)).rounded(px(8.)).bg(theme::text()))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::{Arc, Mutex};

    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{Point, TestAppContext};
    use grove_core::{Outcome, Settings, SettingsFile, TerminalOpensIn, search};
    use tempfile::TempDir;

    use super::*;
    use crate::main_window::Status;
    use crate::test_support::{canned, fake_launcher, set_library, settings_file};

    /// Grove past setup, with the fake launcher's apps detected.
    fn running(cx: &mut TestAppContext, file: &SettingsFile) {
        fake_launcher(cx, Ok(Outcome::Opened));
        cx.update(|cx| {
            gpui_kit::init(cx);
            shell::init(cx);
            set_library(file, vec![canned()], cx);
            shell::start_bare(cx);
        });
    }

    fn open_settings(cx: &mut TestAppContext) -> AnyWindowHandle {
        open_settings_view(cx).0
    }

    /// Tall enough that every row is on screen without scrolling.
    fn open_settings_view(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<SettingsWindow>) {
        cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(SIZE.0), px(1200.)),
                })),
                ..Default::default()
            };
            let opened = open_with(options, cx).unwrap();
            cx.set_global(Open(opened.0));
            opened
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

    fn is_open(cx: &mut TestAppContext, w: AnyWindowHandle) -> bool {
        cx.update(|cx| cx.windows().contains(&w))
    }

    fn settings(cx: &mut TestAppContext) -> Settings {
        cx.update(|cx| shell::library(cx).settings.clone())
    }

    /// Editors: VS Code, Zed. Terminals: Terminal, Warp, Ghostty.
    fn edit(cx: &mut TestAppContext, w: AnyWindowHandle) {
        click(cx, w, ("settings-editor", 1usize));
        click(cx, w, ("settings-terminal", 1usize));
        click(cx, w, "settings-opens-window");
        click(cx, w, "settings-background-fetch");
        click(cx, w, "settings-menu-bar-icon");
    }

    #[gpui_kit::test]
    fn cancel_discards(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        running(cx, &file);
        let w = open_settings(cx);
        edit(cx, w);
        click(cx, w, "settings-cancel");
        assert!(!is_open(cx, w));
        assert_eq!(settings(cx), Settings::default());
        assert_eq!(file.load().unwrap(), Settings::default());
    }

    #[gpui_kit::test]
    fn escape_discards(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        running(cx, &file);
        let w = open_settings(cx);
        click(cx, w, "settings-background-fetch");
        cx.update_window(w, |_, window, cx| window.press("escape", cx))
            .unwrap();
        cx.run_until_parked();
        assert!(!is_open(cx, w));
        assert!(!settings(cx).background_fetch);
    }

    #[gpui_kit::test]
    fn save_applies_to_every_surface_and_is_stored(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        running(cx, &file);
        let w = open_settings(cx);
        edit(cx, w);
        // Opened elsewhere while the window was open.
        cx.update(|cx| shell::record_open(Path::new("/c/wt/api-x"), cx));
        click(cx, w, "settings-save");
        assert!(!is_open(cx, w));

        let s = settings(cx);
        assert_eq!(s.default_editor.as_deref(), Some("zed"));
        assert_eq!(s.default_terminal.as_deref(), Some("warp"));
        assert_eq!(s.terminal_opens_in, Some(TerminalOpensIn::Window));
        assert!(s.background_fetch);
        assert!(!s.show_menu_bar_icon);
        assert_eq!(s.recents, [PathBuf::from("/c/wt/api-x")]);
        assert_eq!(file.load().unwrap(), s);
        // What the popover, switcher and main window open with.
        let names = cx.update(|cx| {
            (
                crate::launcher::target_name(Role::Editor, cx),
                crate::launcher::target_name(Role::Terminal, cx),
            )
        });
        assert_eq!(names, (Some("Zed".into()), Some("Warp".into())));
    }

    #[gpui_kit::test]
    fn tab_or_window_only_where_the_terminal_has_both(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        running(cx, &file);
        cx.update(|cx| {
            let launcher = cx.global_mut::<crate::launcher::Launcher>();
            if let Some(targets) = &mut launcher.targets {
                targets.push(OpenTarget {
                    kind: TargetKind::Terminal(grove_core::Terminal::Kitty),
                    app: Some("/Applications/kitty.app".into()),
                    cli: None,
                });
            }
        });
        let w = open_settings(cx);
        let has = |cx: &mut TestAppContext, id: &'static str| {
            cx.update_window(w, |_, window, cx| {
                window.render_frame(cx);
                window.try_find(id).is_some()
            })
            .unwrap()
        };
        assert!(has(cx, "settings-opens-tab"));
        click(cx, w, ("settings-terminal", 3usize));
        assert!(!has(cx, "settings-opens-tab"));
        assert!(!has(cx, "settings-opens-window"));
    }

    /// A temp code folder holding one real repo, `web`: bare, so it has no worktree whose
    /// status read would run on threads the test scheduler doesn't own.
    fn code_folder() -> TempDir {
        let tmp = TempDir::new().unwrap();
        let ok = Command::new("git")
            .args(["init", "-q", "--bare", "web"])
            .current_dir(tmp.path())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .status()
            .unwrap()
            .success();
        assert!(ok);
        tmp
    }

    /// Answers Change…'s folder prompt with `dir`, then presses Continue.
    fn pick(cx: &mut TestAppContext, dir: &Path) {
        assert!(cx.did_prompt_for_paths());
        let dir = dir.to_path_buf();
        cx.simulate_path_prompt_response(move |_| Some(vec![dir]));
        cx.run_until_parked();
        let picker = cx.update(|cx| setup::changing(cx)).unwrap();
        click(cx, picker, "continue");
        assert!(!is_open(cx, picker));
    }

    fn repos(cx: &mut TestAppContext) -> Vec<String> {
        cx.update(|cx| {
            let snapshot = shell::library(cx).store.snapshot();
            snapshot
                .repos
                .iter()
                .map(|r| r.name().to_string())
                .collect()
        })
    }

    #[gpui_kit::test]
    fn new_code_folder_replaces_repos_on_save(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let code = code_folder();
        running(cx, &file);
        let w = open_settings(cx);
        click(cx, w, "settings-change-folder");
        pick(cx, code.path());
        assert_eq!(repos(cx), ["api"], "applied before Save");

        click(cx, w, "settings-save");
        assert_eq!(repos(cx), ["web"]);
        let s = settings(cx);
        assert_eq!(s.code_dir.as_deref(), Some(code.path()));
        assert_eq!(file.load().unwrap().code_dir.as_deref(), Some(code.path()));
        let hits = cx.update(|cx| {
            let library = shell::library(cx);
            search(library.store.snapshot(), &library.settings.recents, "api").len()
        });
        assert_eq!(hits, 0, "switcher still finds api");
    }

    #[gpui_kit::test]
    fn new_code_folder_is_dropped_on_cancel(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let code = code_folder();
        running(cx, &file);
        let w = open_settings(cx);
        click(cx, w, "settings-change-folder");
        pick(cx, code.path());
        click(cx, w, "settings-cancel");
        assert_eq!(repos(cx), ["api"]);
        assert_eq!(settings(cx).code_dir, None);
    }

    #[gpui_kit::test]
    fn sidebar_change_switches_right_away(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let code = code_folder();
        running(cx, &file);
        let main = cx.update(|cx| {
            shell::show_main(cx);
            cx.windows()[0]
        });
        click(cx, main, "change-code-folder");
        pick(cx, code.path());
        assert_eq!(repos(cx), ["web"]);
        assert_eq!(file.load().unwrap().code_dir.as_deref(), Some(code.path()));
    }

    #[gpui_kit::test]
    fn change_picker_cancel_keeps_the_folder(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let code = code_folder();
        running(cx, &file);
        cx.update(shell::choose_code_dir);
        let dir = code.path().to_path_buf();
        cx.simulate_path_prompt_response(move |_| Some(vec![dir]));
        cx.run_until_parked();
        let picker = cx.update(|cx| setup::changing(cx)).unwrap();
        click(cx, picker, "cancel");
        assert!(!is_open(cx, picker));
        assert_eq!(repos(cx), ["api"]);
    }

    #[gpui_kit::test]
    fn sidebar_change_while_settings_picks_switches_right_away(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let code = code_folder();
        running(cx, &file);
        let w = open_settings(cx);
        click(cx, w, "settings-change-folder");
        cx.update(shell::choose_code_dir);
        pick(cx, code.path());
        assert_eq!(repos(cx), ["web"]);
        click(cx, w, "settings-cancel");
        assert_eq!(settings(cx).code_dir.as_deref(), Some(code.path()));
    }

    #[gpui_kit::test]
    fn settings_change_while_sidebar_picks_waits_for_save(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let code = code_folder();
        running(cx, &file);
        cx.update(shell::choose_code_dir);
        let w = open_settings(cx);
        click(cx, w, "settings-change-folder");
        pick(cx, code.path());
        assert_eq!(repos(cx), ["api"]);
        click(cx, w, "settings-save");
        assert_eq!(repos(cx), ["web"]);
    }

    #[gpui_kit::test]
    fn shows_the_folder_the_sidebar_switched_to(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let code = code_folder();
        running(cx, &file);
        let (_, view) = open_settings_view(cx);
        cx.update(shell::choose_code_dir);
        pick(cx, code.path());
        let shown = cx.update(|cx| {
            let (folder, repos) = view.read(cx).shown_folder(cx);
            (folder.map(Path::to_path_buf), repos)
        });
        assert_eq!(shown, (Some(code.path().to_path_buf()), 1));
    }

    #[gpui_kit::test]
    fn opens_from_top_bar_status_link_and_popover(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        running(cx, &file);
        let main = cx.update(|cx| {
            shell::show_main(cx);
            cx.windows()[0]
        });
        let close = |cx: &mut TestAppContext| {
            let w = cx.update(|cx| window(cx)).expect("settings open");
            cx.update(|cx| shell::close(w, cx));
            cx.run_until_parked();
        };

        click(cx, main, "open-settings");
        close(cx);

        cx.update(|cx| {
            let status = Status {
                text: "Your default editor, Cursor, isn't installed anymore".into(),
                error: true,
                settings: true,
            };
            shell::post(status, cx);
        });
        cx.run_until_parked();
        click(cx, main, "status-settings");
        close(cx);

        let anchor = crate::status_item::Anchor {
            x: 600.,
            y: 0.,
            width: 24.,
            height: 24.,
            screen: None,
        };
        let popover = cx.update(|cx| {
            shell::handle(crate::PlatformEvent::StatusItemClicked { anchor }, cx);
            cx.windows().into_iter().last().unwrap()
        });
        click(cx, popover, "popover-settings");
        assert!(cx.update(|cx| window(cx)).is_some());
    }

    #[gpui_kit::test]
    fn settings_again_reuses_the_open_window(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        running(cx, &file);
        let first = cx.update(|cx| {
            open(cx);
            window(cx).unwrap()
        });
        cx.update(open);
        assert_eq!(cx.update(|cx| window(cx)), Some(first));
        assert_eq!(cx.update(|cx| cx.windows().len()), 1);
    }

    /// Fake login item service at `/Applications/Grove.app`, recording register calls.
    fn fake_login(cx: &mut TestAppContext, status: LoginItemStatus) -> Arc<Mutex<Vec<bool>>> {
        let calls: Arc<Mutex<Vec<bool>>> = Default::default();
        let sink = calls.clone();
        cx.update(|cx| {
            cx.set_global(LoginItems {
                bundle: Some("/Applications/Grove.app".into()),
                installed: true,
                status: Arc::new(move || status),
                set: Arc::new(move |on| {
                    sink.lock().unwrap().push(on);
                    Ok(if on {
                        LoginItemStatus::Enabled
                    } else {
                        LoginItemStatus::NotRegistered
                    })
                }),
            })
        });
        calls
    }

    #[gpui_kit::test]
    fn save_applies_launch_at_login(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        running(cx, &file);
        let calls = fake_login(cx, LoginItemStatus::Enabled);
        let w = open_settings(cx);
        click(cx, w, "settings-launch-at-login");
        click(cx, w, "settings-save");
        assert_eq!(*calls.lock().unwrap(), [false]);
        let s = file.load().unwrap();
        assert!(!s.launch_at_login);
        assert_eq!(
            s.login_item_path.as_deref(),
            Some(Path::new("/Applications/Grove.app"))
        );
    }

    #[test]
    fn login_hints() {
        let app = Some(Path::new("/Applications/Grove.app"));
        let other = Some(Path::new("/Volumes/X/Grove.app"));
        let waiting = Some(LoginItemStatus::RequiresApproval);
        assert_eq!(
            login_hint(app, app, Some(LoginItemStatus::Enabled), true),
            None
        );
        assert_eq!(login_hint(app, None, waiting, false), None);
        assert_eq!(
            login_hint(app, None, waiting, true).unwrap(),
            "Waiting for your OK: allow Grove in System Settings → General → Login Items."
        );
        let moved = login_hint(app, other, None, true).unwrap();
        assert!(
            moved.starts_with("Saving moves launch at login to this copy; the one at /Volumes/X/Grove.app may stay a login item"),
            "{moved}"
        );
        assert!(
            login_hint(None, None, None, true)
                .unwrap()
                .contains("Grove.app")
        );
    }
}
