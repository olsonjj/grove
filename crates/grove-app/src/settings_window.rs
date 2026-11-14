//! Settings: code folder, default editor and terminal, custom commands, the general
//! toggles, and the theme choice. Edits a draft; Save applies it to every surface,
//! Cancel drops it — the theme excepted: a pick applies the theme live and Save
//! persists it. Any way the window closes re-applies the choice it opened with.

use std::path::Path;

use gpui_kit::base::Button;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::searchable_list::SearchableListDelegate as _;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext, Bounds, ClickEvent, Context, ElementId, Entity,
    FocusHandle, FontWeight, Global, InteractiveElement, IntoElement, KeyDownEvent, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Subscription, TestSupportExt,
    TitlebarOptions, Window, WindowBounds, WindowKind, WindowOptions, div, px, size,
};
use grove_core::{
    CustomCommand, LoginItemStatus, NoTarget, Opener, Preferences, Role, Settings, Snapshot,
    TargetKind, TerminalOpensIn, ThemeChoice, command_line,
};

use crate::icons::{self, icon};
use crate::login_item::{self, LOGIN_ITEMS, LoginItems};
use crate::main_window::tilde;
use crate::theme::{self, ButtonKind, button};
use crate::theme_picker::ThemeMenu;
use crate::{setup, shell, theme_picker, windows};

const SIZE: (f32, f32) = (820., 760.);
const VERSION: &str = concat!(
    "Grove ",
    env!("CARGO_PKG_VERSION"),
    " (build ",
    env!("GROVE_BUILD"),
    ")"
);

/// The open settings window and the theme choice it opened with, so every close
/// path re-applies that choice: they all funnel through window removal, and the
/// `closed` observer sees the removal wherever it came from.
struct Open {
    window: AnyWindowHandle,
    /// The persisted choice as stored when the window opened (not its resolution,
    /// so a later Follow System entry isn't pinned by a cancel — spec story 16): a
    /// FollowSystem entry re-resolves from the system's appearance at cancel time.
    entry_theme: Option<ThemeChoice>,
    /// Keeps the window-closed observer alive while the window is open (dropped
    /// with the global; the underscore keeps the never-read lint quiet).
    _closed: Subscription,
}

impl Global for Open {}

/// Marks `window` the open settings dialog and remembers the entry choice.
fn became_open(window: AnyWindowHandle, cx: &mut App) {
    let entry_theme = shell::library(cx).settings.theme.clone();
    let closed = cx.on_window_closed(|cx: &mut App, window_id| {
        let Some(open) = cx.try_global::<Open>() else {
            return;
        };
        if open.window.window_id() != window_id {
            return;
        }
        // Save rewrites the entry before removing itself, so this keeps what the
        // window saved; every other path keeps what it opened with.
        let entry = open.entry_theme.clone();
        cx.remove_global::<Open>();
        theme::apply_entry_choice(entry, cx);
    });
    cx.set_global(Open {
        window,
        entry_theme,
        _closed: closed,
    });
}

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
        Ok((handle, _)) => became_open(handle, cx),
        Err(e) => eprintln!("[grove] settings: {e:#}"),
    }
}

pub fn window(cx: &App) -> Option<AnyWindowHandle> {
    cx.try_global::<Open>()
        .map(|o| o.window)
        .filter(|w| cx.windows().contains(w))
}

fn open_with(
    options: WindowOptions,
    cx: &mut App,
) -> gpui_kit::Result<(AnyWindowHandle, Entity<SettingsWindow>)> {
    windows::open_window(options, cx, |window, cx| {
        cx.new(|cx| SettingsWindow::new(window, cx))
    })
}

pub struct SettingsWindow {
    draft: Preferences,
    /// Scan of a newly chosen code folder; swapped in on Save.
    scanned: Option<Snapshot>,
    /// macOS's login item status for this copy; `None` until read, or when unbundled.
    login_status: Option<LoginItemStatus>,
    /// The custom command being added or edited.
    form: Option<CommandForm>,
    /// The THEME dropdown's select: the registry's themes, the committed choice and
    /// the open popup live in the entity.
    theme_select: Entity<SelectState<ThemeMenu>>,
    focus: FocusHandle,
}

struct CommandForm {
    /// Index in the draft's commands; `None` adds one.
    index: Option<usize>,
    label: Entity<InputState>,
    template: Entity<InputState>,
    run_in_terminal: bool,
    problem: Option<&'static str>,
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
        // The THEME dropdown over the themes registered at launch; the stored choice
        // (Grove Dark when nothing is stored) starts committed, so the trigger reads
        // it and the list opens on it with its check.
        let display = theme_display(&shell::library(cx).settings.theme);
        let menu = ThemeMenu::new(theme_picker::sections(cx));
        let selected = menu.position(&display);
        let theme_select =
            cx.new(|cx| SelectState::new(menu, selected, window, cx).searchable(true));
        cx.subscribe_in(
            &theme_select,
            window,
            |this, _, ev: &SelectEvent<ThemeMenu>, _, cx| this.theme_picked(ev, cx),
        )
        .detach();
        Self {
            draft: shell::library(cx).settings.preferences(),
            scanned: None,
            login_status: None,
            form: None,
            theme_select,
            focus,
        }
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.finish_form(cx) {
            return;
        }
        let mut prefs = self.draft.clone();
        // Only a folder chosen here changes it (the sidebar may have meanwhile).
        if self.scanned.is_none() {
            prefs.code_dir = shell::library(cx).settings.code_dir.clone();
        }
        shell::save_preferences(prefs, self.scanned.take(), cx);
        // What this window leaves behind is what it saved: the close-revert keeps
        // the saved theme instead of the one the window opened with.
        if cx.has_global::<Open>() {
            cx.global_mut::<Open>().entry_theme = self.draft.theme.clone();
        }
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

    /// What decides the defaults once saved.
    fn draft_settings(&self) -> Settings {
        Settings {
            default_editor: self.draft.default_editor.clone(),
            default_terminal: self.draft.default_terminal.clone(),
            custom_commands: self.draft.custom_commands.clone(),
            ..Default::default()
        }
    }

    fn edit_command(&mut self, index: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let existing = index
            .and_then(|i| self.draft.custom_commands.get(i))
            .cloned();
        let input = |value: Option<&String>,
                     placeholder: &'static str,
                     window: &mut Window,
                     cx: &mut Context<Self>| {
            let value = value.cloned().unwrap_or_default();
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(value)
            })
        };
        let label = input(existing.as_ref().map(|c| &c.label), "Neovim", window, cx);
        let template = input(
            existing.as_ref().map(|c| &c.template),
            "nvim {path}",
            window,
            cx,
        );
        label.update(cx, |input, cx| input.focus(window, cx));
        self.form = Some(CommandForm {
            index,
            label,
            template,
            run_in_terminal: existing.is_none_or(|c| c.run_in_terminal),
            problem: None,
        });
        cx.notify();
    }

    /// Puts the form's command in the draft, keeping it the default if it was. False (and
    /// the form stays, saying why) when it's not valid.
    fn finish_form(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(form) = &mut self.form else {
            return true;
        };
        let cmd = CustomCommand {
            label: form.label.read(cx).value().trim().to_string(),
            template: form.template.read(cx).value().trim().to_string(),
            run_in_terminal: form.run_in_terminal,
        };
        let commands = &mut self.draft.custom_commands;
        let others: Vec<CustomCommand> = commands
            .iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != form.index)
            .map(|(_, c)| c.clone())
            .collect();
        if let Some(problem) = cmd.problem(&others) {
            form.problem = Some(problem);
            cx.notify();
            return false;
        }
        match form.index {
            Some(i) => {
                if self.draft.default_editor == Some(commands[i].id()) {
                    self.draft.default_editor = Some(cmd.id());
                }
                commands[i] = cmd;
            }
            None => commands.push(cmd),
        }
        self.form = None;
        cx.notify();
        true
    }

    /// Removing the default editor hands the default back to the first detected editor.
    fn remove_command(&mut self, index: usize, cx: &mut Context<Self>) {
        let cmd = self.draft.custom_commands.remove(index);
        if self.draft.default_editor == Some(cmd.id()) {
            self.draft.default_editor = None;
        }
        cx.notify();
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
        let palette = theme::palette(cx);
        let folder = folder.map_or_else(|| "No folder chosen".into(), tilde);
        div()
            .flex()
            .items_center()
            .gap(px(14.))
            .px(px(16.))
            .py(px(14.))
            .rounded(px(10.))
            .border_1()
            .border_color(palette.border_strong())
            .bg(palette.raised())
            .child(icon(icons::FOLDER, 18.).text_color(palette.link()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(label("CODE FOLDER", cx))
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
                                    .text_color(palette.muted())
                                    .child(format!(
                                        "· {repos} repo{} · Grove scans nothing outside it",
                                        if repos == 1 { "" } else { "s" }
                                    )),
                            ),
                    ),
            )
            .child(button(
                cx,
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
        let palette = theme::palette(cx);
        let targets = cx
            .try_global::<crate::launcher::Launcher>()
            .and_then(|l| l.targets.clone());
        let mut col = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(label(title, cx).mb(px(4.)));
        let Some(targets) = targets else {
            return col.child(note("Looking for apps…", cx)).into_any_element();
        };
        let settings = self.draft_settings();
        let chosen = grove_core::default_opener(role, &settings, &targets);
        let selected = chosen.as_ref().ok().map(Opener::id);
        let selected_kind = match chosen {
            Ok(Opener::App(t)) => Some(t.kind),
            _ => None,
        };
        match &chosen {
            Err(e @ NoTarget::NoneDetected(_)) => col = col.child(note(e.to_string(), cx)),
            Err(e) => {
                col = col
                    .child(note(format!("{e}. Pick another."), cx).text_color(palette.warn_text()));
            }
            Ok(_) => {}
        }
        let rows = targets.iter().filter(|t| t.role() == role).enumerate();
        for (ix, target) in rows {
            let kind = target.kind;
            let on = selected.as_deref() == Some(kind.id());
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
                choice((id, ix), on, a11y, cx)
                    .child(radio(on, cx))
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
                            .text_color(palette.muted()),
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
        if let (Role::Terminal, Some(TargetKind::Terminal(t))) = (role, selected_kind) {
            col = col.child(self.opens_in_picker(t, cx));
        }
        if role == Role::Editor {
            col = col.child(self.commands(selected.as_deref(), cx));
        }
        col.into_any_element()
    }

    /// Custom commands, each selectable as the default editor, and the add / edit form.
    fn commands(&self, selected: Option<&str>, cx: &mut Context<Self>) -> AnyElement {
        let palette = theme::palette(cx);
        let editing = self.form.is_some();
        let mut col = div()
            .mt(px(6.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(label("CUSTOM COMMANDS", cx));
        for (ix, cmd) in self.draft.custom_commands.iter().enumerate() {
            let on = selected == Some(cmd.id().as_str());
            let detail = if cmd.run_in_terminal {
                "in terminal"
            } else {
                "background"
            };
            let a11y = format!(
                "{}, {}, {detail}{}",
                cmd.label,
                cmd.template,
                if on { ", selected" } else { "" }
            );
            let row = choice(("settings-command", ix), on, a11y, cx)
                .flex_1()
                .min_w_0()
                .child(radio(on, cx))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(div().text_size(px(14.)).child(cmd.label.clone()))
                        .child(
                            theme::mono(format!("{} · {detail}", cmd.template))
                                .truncate()
                                .text_size(px(12.))
                                .text_color(palette.muted()),
                        ),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.draft.default_editor =
                        this.draft.custom_commands.get(ix).map(CustomCommand::id);
                    cx.notify();
                }));
            col = col.child(div().flex().items_center().gap(px(6.)).child(row).when(
                !editing,
                |d| {
                    d.child(button(
                        cx,
                        ("settings-command-edit", ix),
                        "Edit",
                        ButtonKind::Secondary,
                        Some(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.edit_command(Some(ix), window, cx)
                        })),
                    ))
                    .child(button(
                        cx,
                        ("settings-command-remove", ix),
                        "Remove",
                        ButtonKind::Secondary,
                        Some(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.remove_command(ix, cx);
                            window.focus(&this.focus, cx);
                        })),
                    ))
                },
            ));
        }
        match &self.form {
            Some(form) => col.child(self.command_form(form, cx)),
            None => col.child(div().flex().child(button(
                cx,
                "settings-command-add",
                "Add command…",
                ButtonKind::Secondary,
                Some(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.edit_command(None, window, cx)
                })),
            ))),
        }
        .into_any_element()
    }

    fn command_form(&self, form: &CommandForm, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = theme::palette(cx);
        let field = |id: &'static str, title: &'static str, state: &Entity<InputState>| {
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(label(title, cx))
                .child(
                    div()
                        .id(id)
                        .child(
                            Input::new(state)
                                .font_family(theme::MONO)
                                .text_size(px(13.)),
                        )
                        .test_support(),
                )
        };
        let on = form.run_in_terminal;
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(12.))
            .rounded(px(8.))
            .border_1()
            .border_color(palette.border_strong())
            .child(field("settings-command-label", "NAME", &form.label))
            .child(field(
                "settings-command-template",
                "COMMAND ({path} is the worktree, quoted)",
                &form.template,
            ))
            .child(
                choice(
                    "settings-command-terminal",
                    false,
                    format!("Run in terminal, {}", if on { "on" } else { "off" }),
                    cx,
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(div().text_size(px(14.)).child("Run in terminal"))
                        .child(note(
                            "On: a new tab in your terminal. Off: runs in the background.",
                            cx,
                        )),
                )
                .child(switch(on, cx))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    if let Some(form) = &mut this.form {
                        form.run_in_terminal = !form.run_in_terminal;
                    }
                    cx.notify();
                })),
            )
            .when_some(form.problem, |d, problem| {
                d.child(note(problem, cx).text_color(palette.warn_text()))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(button(
                        cx,
                        "settings-command-discard",
                        "Discard",
                        ButtonKind::Secondary,
                        Some(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.form = None;
                            window.focus(&this.focus, cx);
                            cx.notify();
                        })),
                    ))
                    .child(button(
                        cx,
                        "settings-command-done",
                        "Done",
                        ButtonKind::Primary,
                        Some(cx.listener(|this, _: &ClickEvent, window, cx| {
                            if this.finish_form(cx) {
                                window.focus(&this.focus, cx);
                            }
                        })),
                    )),
            )
    }

    /// A confirmed pick: the draft keeps the choice — Save persists it, unlike the
    /// other prefs — and the theme re-skins every visible window now (including
    /// this one, whose own chrome follows), through the same route the launch
    /// choice takes.
    fn theme_picked(&mut self, ev: &SelectEvent<ThemeMenu>, cx: &mut Context<Self>) {
        let SelectEvent::Confirm(Some(value)) = ev else {
            return;
        };
        let choice = theme_picker::choice_for(value);
        self.draft.theme = Some(choice.clone());
        // The resolved theme re-skins every visible window now — a Follow System
        // pick resolves from the system's current appearance (see theme::apply_choice).
        theme::apply_choice(cx, Some(&choice));
        cx.notify();
    }

    /// New tab or new window, for the default terminal where it supports both.
    fn opens_in_picker(&self, t: grove_core::Terminal, cx: &mut Context<Self>) -> AnyElement {
        if !t.supports_tabs() {
            return note(format!("{} opens new windows only.", t.name()), cx).into_any_element();
        }
        let palette = theme::palette(cx);
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
                .focus_visible(|s| s.border_color(palette.link()))
                .text_size(px(13.))
                .cursor_pointer()
                .when_else(
                    on,
                    |b| {
                        b.bg(palette.selected())
                            .border_color(palette.text_soft())
                            .text_color(palette.text())
                            .font_weight(FontWeight::SEMIBOLD)
                    },
                    |b| {
                        b.text_color(palette.muted())
                            .hover(|s| s.bg(palette.border()))
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
            .child(label("OPENS IN", cx))
            .child(
                div()
                    .flex()
                    .gap(px(4.))
                    .p(px(3.))
                    .rounded(px(8.))
                    .bg(palette.field())
                    .border_1()
                    .border_color(palette.border())
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
                        cx,
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
            .child(label("GENERAL", cx).mb(px(4.)))
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

    #[expect(clippy::too_many_arguments, reason = "one call per toggle row")]
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
        let palette = theme::palette(cx);
        choice(
            id,
            false,
            format!("{title}, {}", if on { "on" } else { "off" }),
            cx,
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
                        .text_color(palette.muted())
                        .child(detail),
                )
                .when_some(hint, |d, hint| {
                    d.child(
                        div()
                            .text_size(px(12.))
                            .text_color(palette.warn_text())
                            .child(hint),
                    )
                }),
        )
        .child(switch(on, cx))
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            let value = field(&mut this.draft);
            *value = !*value;
            cx.notify();
        }))
    }

    /// The THEME section: its own header above the version note and the choice
    /// dropdown, styled like the sections above it and the secondary buttons. The
    /// window keeps its fixed size; the list is the select's popup.
    fn theme_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = theme::palette(cx);
        let display = theme_display(&self.draft.theme);
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(label("THEME", cx).mb(px(4.)))
            .child(
                div().flex_none().w(px(300.)).child(
                    // The trigger is the component's own div, so the secondary-button
                    // look rides on the style refinement; it can't carry Grove's hover
                    // tint with it. Focus/open still show the component's ring.
                    Select::new(&self.theme_select)
                        .id("theme-picker")
                        .search_placeholder("Search themes…")
                        .placeholder(display.clone())
                        .accessibility_label(format!("Theme, {display}, opens theme list",))
                        .h(px(34.))
                        .px(px(14.))
                        .rounded(px(8.))
                        .border_1()
                        .border_color(palette.border_strong())
                        .bg(palette.raised())
                        .text_size(px(13.)),
                ),
            )
    }

    fn footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = theme::palette(cx);
        let targets = cx
            .try_global::<crate::launcher::Launcher>()
            .and_then(|l| l.targets.clone())
            .unwrap_or_default();
        let settings = self.draft_settings();
        let name = |role| {
            grove_core::default_opener(role, &settings, &targets)
                .map_or("—".to_string(), |o| o.name().to_string())
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
            .border_color(palette.border())
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.))
                    .text_color(palette.muted())
                    .child(summary),
            )
            .child(
                div()
                    .flex()
                    .gap(px(10.))
                    .child(button(
                        cx,
                        "settings-cancel",
                        "Cancel",
                        ButtonKind::Secondary,
                        Some(|_: &ClickEvent, window: &mut Window, _: &mut App| {
                            window.remove_window()
                        }),
                    ))
                    .child(button(
                        cx,
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
        let palette = theme::palette(cx);
        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .size_full()
            .flex()
            .flex_col()
            .bg(palette.panel())
            .text_color(palette.text())
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
                            .child(div().text_size(px(14.)).text_color(palette.muted()).child(
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
                    .child(self.general(cx))
                    .child(self.theme_section(cx))
                    .child(note(VERSION, cx)),
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

fn label(text: &'static str, cx: &App) -> gpui_kit::Div {
    div()
        .text_size(px(11.))
        .text_color(theme::palette(cx).faint())
        .child(text)
}

fn note(text: impl Into<SharedString>, cx: &App) -> gpui_kit::Div {
    div()
        .text_size(px(12.))
        .text_color(theme::palette(cx).muted())
        .child(text.into())
}

/// What the theme dropdown reads for a stored choice: the choice as stored — a
/// theme by name, "Follow System" for the FollowSystem kind; Grove Dark when
/// nothing is stored, the default the launch apply stood up.
fn theme_display(choice: &Option<ThemeChoice>) -> SharedString {
    match choice {
        Some(ThemeChoice::Name(name)) => name.clone().into(),
        Some(ThemeChoice::FollowSystem) => theme_picker::FOLLOW_SYSTEM.into(),
        None => theme::GROVE_DARK_NAME.into(),
    }
}

/// A selectable row: radio choice or toggle.
fn choice(id: impl Into<ElementId>, selected: bool, a11y: String, cx: &App) -> Button {
    let palette = theme::palette(cx);
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
        .focus_visible(|s| s.border_color(palette.link()))
        .cursor_pointer()
        .when_else(
            selected,
            |b| b.bg(palette.selected()),
            |b| b.bg(palette.raised()).hover(|s| s.bg(palette.border())),
        )
}

fn radio(on: bool, cx: &App) -> impl IntoElement {
    let palette = theme::palette(cx);
    div()
        .flex_none()
        .size(px(14.))
        .rounded(px(7.))
        .border_1()
        .border_color(if on {
            palette.accent()
        } else {
            palette.border_strong()
        })
        .when(on, |d| d.border_4())
}

fn switch(on: bool, cx: &App) -> impl IntoElement {
    let palette = theme::palette(cx);
    div()
        .flex_none()
        .w(px(34.))
        .h(px(20.))
        .p(px(2.))
        .flex()
        .rounded(px(10.))
        .when_else(
            on,
            |d| d.bg(palette.accent()).justify_end(),
            |d| d.bg(palette.border_strong()),
        )
        .child(div().size(px(16.)).rounded(px(8.)).bg(palette.text()))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::{Arc, Mutex};

    use gpui_kit::WindowAppearance;
    use gpui_kit::component::IndexPath;
    use gpui_kit::component::theme::ActiveTheme;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{ElementId, Point, TestAppContext};
    use grove_core::{Outcome, Settings, SettingsFile, TerminalOpensIn, ThemeChoice, search};

    use ThemeChoice::{FollowSystem, Name};
    use tempfile::TempDir;

    use super::*;
    use crate::main_window::Status;
    use crate::test_support::{
        canned, fake_launcher, indexed_labels, set_library, settings_file, tab_to,
    };

    /// Grove past setup, with the fake launcher's apps detected.
    fn running(cx: &mut TestAppContext, file: &SettingsFile) {
        fake_launcher(cx, Ok(Outcome::Opened));
        cx.update(|cx| {
            gpui_kit::init(cx);
            theme::init(cx);
            shell::init(cx);
            set_library(file, vec![canned()], cx);
            shell::start_bare(cx);
            // Launch applies the persisted choice once (main.rs); the seeded file
            // stands in for the settings file here.
            theme::apply_choice_from(&file, cx);
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
            became_open(opened.0, cx);
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
        cx.update(|cx| {
            shell::record_open(Path::new("/c/wt/api-x"), cx);
            shell::toggle_pin(Path::new("/c/api"), cx);
            shell::set_repo_sort(grove_core::SortMode::Name, cx);
            shell::set_tree_sort(grove_core::SortMode::Name, cx);
        });
        click(cx, w, "settings-save");
        assert!(!is_open(cx, w));

        let s = settings(cx);
        assert_eq!(s.default_editor.as_deref(), Some("zed"));
        assert_eq!(s.default_terminal.as_deref(), Some("warp"));
        assert_eq!(s.terminal_opens_in, Some(TerminalOpensIn::Window));
        assert!(s.background_fetch);
        assert!(!s.show_menu_bar_icon);
        assert_eq!(s.recents, [PathBuf::from("/c/wt/api-x")]);
        assert!(s.last_opened.contains_key(Path::new("/c/wt/api-x")));
        assert_eq!(s.pinned_repos, [PathBuf::from("/c/api")]);
        assert_eq!(s.repo_sort, grove_core::SortMode::Name);
        assert_eq!(s.tree_sort, grove_core::SortMode::Name);
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
                targets.push(grove_core::OpenTarget {
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

    fn typed(cx: &mut TestAppContext, w: AnyWindowHandle, text: &str) {
        cx.update_window(w, |_, window, cx| window.input(text, cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn found(cx: &mut TestAppContext, w: AnyWindowHandle, id: impl Into<ElementId>) -> bool {
        let id = id.into();
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some()
        })
        .unwrap()
    }

    /// Types into the open form's empty fields.
    fn fill(cx: &mut TestAppContext, w: AnyWindowHandle, name: &str, template: &str) {
        click(cx, w, "settings-command-label");
        typed(cx, w, name);
        click(cx, w, "settings-command-template");
        typed(cx, w, template);
    }

    fn nvim() -> CustomCommand {
        CustomCommand {
            label: "Neovim".into(),
            template: "nvim {path}".into(),
            run_in_terminal: true,
        }
    }

    #[gpui_kit::test]
    fn add_command_and_make_it_the_default_editor(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        running(cx, &file);
        let w = open_settings(cx);
        click(cx, w, "settings-command-add");
        fill(cx, w, "Neovim", "nvim {path}");
        click(cx, w, "settings-command-done");
        assert!(!found(cx, w, "settings-command-done"));
        click(cx, w, "settings-command-add");
        fill(cx, w, "Sublime", "subl -n {path}");
        click(cx, w, "settings-command-terminal");
        click(cx, w, "settings-command-done");
        click(cx, w, ("settings-command", 0usize));
        assert_eq!(
            settings(cx),
            Settings::default(),
            "nothing applied before Save"
        );

        click(cx, w, "settings-save");
        let s = settings(cx);
        let sublime = CustomCommand {
            label: "Sublime".into(),
            template: "subl -n {path}".into(),
            run_in_terminal: false,
        };
        assert_eq!(s.custom_commands, [nvim(), sublime]);
        assert_eq!(s.default_editor.as_deref(), Some("custom:Neovim"));
        assert_eq!(file.load().unwrap(), s);
        let name = cx.update(|cx| crate::launcher::target_name(Role::Editor, cx));
        assert_eq!(name.as_deref(), Some("Neovim"));
    }

    #[gpui_kit::test]
    fn cancel_discards_command_changes(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let saved = Settings {
            custom_commands: vec![nvim()],
            ..Default::default()
        };
        let file = settings_file(&tmp, saved.clone());
        running(cx, &file);
        let w = open_settings(cx);
        click(cx, w, ("settings-command-remove", 0usize));
        click(cx, w, "settings-command-add");
        fill(cx, w, "Helix", "hx {path}");
        click(cx, w, "settings-command-done");
        click(cx, w, "settings-cancel");
        assert_eq!(settings(cx), saved);
        assert_eq!(file.load().unwrap(), saved);
    }

    #[gpui_kit::test]
    fn invalid_commands_are_flagged_not_added(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        running(cx, &file);
        let (w, view) = open_settings_view(cx);
        let problem = |cx: &mut TestAppContext| {
            view.read_with(cx, |v, _| v.form.as_ref().and_then(|f| f.problem))
        };
        click(cx, w, "settings-command-add");
        click(cx, w, "settings-command-done");
        assert_eq!(problem(cx), Some("Give the command a name."));
        fill(cx, w, "  Neovim ", "nvim .");
        click(cx, w, "settings-command-done");
        assert!(problem(cx).unwrap().contains("{path}"));
        // Save refuses too, keeping the window open on the problem.
        click(cx, w, "settings-save");
        assert!(is_open(cx, w));
        assert!(settings(cx).custom_commands.is_empty());

        click(cx, w, "settings-command-discard");
        click(cx, w, "settings-save");
        assert!(!is_open(cx, w));
        assert!(settings(cx).custom_commands.is_empty());
    }

    #[gpui_kit::test]
    fn edit_keeps_the_default_and_remove_hands_it_back(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(
            &tmp,
            Settings {
                default_editor: Some("custom:Neovim".into()),
                custom_commands: vec![nvim()],
                ..Default::default()
            },
        );
        running(cx, &file);
        let w = open_settings(cx);
        click(cx, w, ("settings-command-edit", 0usize));
        // The name field has focus.
        cx.update_window(w, |_, window, cx| window.press("cmd-a", cx))
            .unwrap();
        typed(cx, w, "Neovim (tab)");
        click(cx, w, "settings-command-done");
        click(cx, w, "settings-save");
        let s = settings(cx);
        assert_eq!(s.custom_commands[0].label, "Neovim (tab)");
        assert_eq!(s.custom_commands[0].template, "nvim {path}");
        assert_eq!(s.default_editor.as_deref(), Some("custom:Neovim (tab)"));

        let w = open_settings(cx);
        click(cx, w, ("settings-command-remove", 0usize));
        click(cx, w, "settings-save");
        let s = settings(cx);
        assert!(s.custom_commands.is_empty());
        assert_eq!(s.default_editor, None);
        let name = cx.update(|cx| crate::launcher::target_name(Role::Editor, cx));
        assert_eq!(name.as_deref(), Some("VS Code"));
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

    #[gpui_kit::test]
    fn every_setting_is_reachable_by_keyboard(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        running(cx, &file);
        let w = open_settings(cx);
        let press = |cx: &mut TestAppContext, key: &str| {
            cx.update_window(w, |_, window, cx| window.press(key, cx))
                .unwrap();
            cx.run_until_parked();
        };
        for (id, key) in [
            (ElementId::from(("settings-editor", 1usize)), "space"),
            (("settings-terminal", 1usize).into(), "enter"),
            ("settings-opens-window".into(), "space"),
            ("settings-background-fetch".into(), "space"),
            ("settings-menu-bar-icon".into(), "enter"),
            ("settings-command-add".into(), "enter"),
            ("settings-command-discard".into(), "enter"),
            ("settings-change-folder".into(), "tab"),
            ("settings-cancel".into(), "tab"),
            ("settings-save".into(), "enter"),
        ] {
            tab_to(cx, w, id);
            press(cx, key);
        }
        assert!(!is_open(cx, w));
        let s = settings(cx);
        assert_eq!(s.default_editor.as_deref(), Some("zed"));
        assert_eq!(s.default_terminal.as_deref(), Some("warp"));
        assert_eq!(s.terminal_opens_in, Some(TerminalOpensIn::Window));
        assert!(s.background_fetch);
        assert!(!s.show_menu_bar_icon);
        assert!(s.custom_commands.is_empty());
    }

    fn press(cx: &mut TestAppContext, w: AnyWindowHandle, key: &'static str) {
        cx.update_window(w, |_, window, cx| window.press(key, cx))
            .unwrap();
        cx.run_until_parked();
    }

    /// The a11y label of the rendered element with `id`.
    fn label(cx: &mut TestAppContext, w: AnyWindowHandle, id: impl Into<ElementId>) -> String {
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.find(id.into()).label().unwrap().to_string()
        })
        .unwrap()
    }

    /// The active theme's name.
    fn applied(cx: &mut TestAppContext) -> String {
        cx.update(|cx| cx.theme().theme_name().to_string())
    }

    fn draft_theme(cx: &mut TestAppContext, view: &Entity<SettingsWindow>) -> Option<ThemeChoice> {
        view.read_with(cx, |v, _| v.draft.theme.clone())
    }

    /// The dropdown's row labels in display order, with the list open.
    fn theme_rows(cx: &mut TestAppContext, w: AnyWindowHandle) -> Vec<String> {
        indexed_labels(cx, w, "theme-row")
    }

    fn theme_sections(cx: &mut TestAppContext, w: AnyWindowHandle) -> Vec<String> {
        indexed_labels(cx, w, "theme-section")
    }

    /// Opens the theme list and picks by typing `query`, then ⏎ (the keyboard path).
    fn pick_theme(cx: &mut TestAppContext, w: AnyWindowHandle, query: &str) {
        click(cx, w, "theme-picker");
        typed(cx, w, query);
        press(cx, w, "enter");
    }

    /// The theme right after Grove Dark in the picker's display order — where one
    /// arrow-down steps from the committed choice.
    fn next_theme_after_grove_dark(cx: &mut TestAppContext) -> String {
        cx.update(|cx| {
            use gpui_kit::component::searchable_list::SearchableListItem as _;
            let menu = ThemeMenu::new(theme_picker::sections(cx));
            let mut names: Vec<SharedString> = Vec::new();
            for section in 0..menu.sections_count(cx) {
                for row in 0..menu.items_count(section) {
                    names.push(
                        menu.item(IndexPath::default().section(section).row(row))
                            .unwrap()
                            .title(),
                    );
                }
            }
            let grove = names.iter().position(|name| name == "Grove Dark").unwrap();
            names[grove + 1].to_string()
        })
    }

    #[gpui_kit::test]
    fn picking_a_theme_applies_it_live_and_stays_a_draft(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        running(cx, &file);
        let (w, view) = open_settings_view(cx);
        // Arrows step the keyboard cursor off the committed choice; ⏎ picks where
        // it lands (spec story 5).
        let next = next_theme_after_grove_dark(cx);
        click(cx, w, "theme-picker");
        press(cx, w, "down");
        press(cx, w, "enter");
        assert_eq!(applied(cx), next);
        assert_eq!(draft_theme(cx, &view), Some(Name(next)));
        // Unlike the other prefs, the pick leaves the stored choice alone.
        assert_eq!(settings(cx).theme, None);
        assert_eq!(file.load().unwrap().theme, None);
    }

    #[gpui_kit::test]
    fn the_list_is_grouped_searchable_and_labeled(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        running(cx, &file);
        let w = open_settings(cx);
        assert_eq!(
            label(cx, w, "theme-picker"),
            "Theme, Grove Dark, opens theme list"
        );
        click(cx, w, "theme-picker");
        // SYSTEM first with its single fixed row, then LIGHT (spec story 3, 35):
        // on this tall window the fold shows both groups, Follow System atop.
        assert_eq!(theme_sections(cx, w), ["SYSTEM", "LIGHT"]);
        let rows = theme_rows(cx, w);
        assert_eq!(
            rows.first().map(String::as_str),
            Some("Follow System, follows system appearance")
        );
        assert!(
            rows[1..].iter().all(|row| row.ends_with(", light")),
            "only light rows after Follow System: {rows:?}"
        );
        // A query landing in a single group shows that group alone: on the still
        // open list, the search starts empty ("latte" only matches Catppuccin Latte).
        typed(cx, w, "latte");
        assert_eq!(theme_sections(cx, w), ["LIGHT"]);
        let rows = theme_rows(cx, w);
        assert!(!rows.is_empty());
        assert!(
            rows.iter().all(|row| row.ends_with(", light")),
            "only light rows in the query's group: {rows:?}"
        );
        // The committed choice is checked wherever it shows (spec story 4).
        press(cx, w, "escape");
        click(cx, w, "theme-picker");
        typed(cx, w, "grove");
        assert_eq!(theme_rows(cx, w), ["Grove Dark, dark, selected"]);
        // Follow System is searchable with the themes (ticket 35).
        press(cx, w, "escape");
        click(cx, w, "theme-picker");
        typed(cx, w, "sys");
        assert_eq!(
            theme_rows(cx, w),
            ["Follow System, follows system appearance"]
        );
        // The search narrows case-insensitively on the name (spec story 2), and a
        // query into both groups shows LIGHT's rows before DARK's.
        press(cx, w, "escape");
        click(cx, w, "theme-picker");
        typed(cx, w, "GRU");
        assert_eq!(
            theme_rows(cx, w),
            ["Gruvbox Light, light", "Gruvbox Dark, dark"]
        );
        press(cx, w, "escape");
        click(cx, w, "theme-picker");
        typed(cx, w, "kibble");
        assert_eq!(theme_sections(cx, w), ["DARK"]);
        assert_eq!(
            theme_rows(cx, w),
            ["Kibble, dark"],
            "uncommitted rows carry no check; Grove Dark holds it"
        );
    }

    #[gpui_kit::test]
    fn cancel_reverts_to_the_entry_theme_after_several_picks(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(
            &tmp,
            Settings {
                theme: Some(Name("Kibble".into())),
                ..Settings::default()
            },
        );
        running(cx, &file);
        let w = open_settings(cx);
        for pick in ["flexoki dark", "ayu light"] {
            pick_theme(cx, w, pick);
        }
        assert_eq!(applied(cx), "Ayu Light");
        click(cx, w, "settings-cancel");
        assert!(!is_open(cx, w));
        // The entry theme is back, and the file was never touched (story 16).
        assert_eq!(applied(cx), "Kibble");
        assert_eq!(settings(cx).theme, Some(Name("Kibble".into())));
        assert_eq!(file.load().unwrap().theme, Some(Name("Kibble".into())));
        // Reopen: the dropdown still names the stored choice.
        let w = open_settings(cx);
        assert_eq!(
            label(cx, w, "theme-picker"),
            "Theme, Kibble, opens theme list"
        );
    }

    #[gpui_kit::test]
    fn save_persists_the_choice_and_it_survives_reopen(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        running(cx, &file);
        let w = open_settings(cx);
        pick_theme(cx, w, "kibble");
        click(cx, w, "settings-save");
        assert!(!is_open(cx, w));
        assert_eq!(applied(cx), "Kibble", "Save keeps the picked theme live");
        let s = settings(cx);
        assert_eq!(s.theme, Some(Name("Kibble".into())));
        assert_eq!(file.load().unwrap().theme, Some(Name("Kibble".into())));
        // Reopen shows the stored choice committed in the picker.
        let w = open_settings(cx);
        assert_eq!(
            label(cx, w, "theme-picker"),
            "Theme, Kibble, opens theme list"
        );
        click(cx, w, "theme-picker");
        typed(cx, w, "kibble");
        assert_eq!(
            theme_rows(cx, w),
            ["Kibble, dark, selected"],
            "the reopened picker checks Kibble"
        );
    }

    #[gpui_kit::test]
    fn escape_closes_the_open_list_and_the_next_one_cancels(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        running(cx, &file);
        let (w, view) = open_settings_view(cx);
        click(cx, w, "theme-picker");
        assert!(!theme_rows(cx, w).is_empty(), "the list is open");
        press(cx, w, "escape");
        assert!(
            is_open(cx, w),
            "the open list's escape doesn't cancel the dialog"
        );
        assert!(theme_rows(cx, w).is_empty(), "the list closed");
        assert_eq!(draft_theme(cx, &view), None, "nothing was picked");
        press(cx, w, "escape");
        assert!(!is_open(cx, w));
        assert_eq!(settings(cx).theme, None);
    }

    /// Any close path funnels through window removal; the removal observer is the
    /// path the traffic light and shell::close take.
    #[gpui_kit::test]
    fn closing_the_window_reverts_to_the_entry_theme(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(
            &tmp,
            Settings {
                theme: Some(Name("Kibble".into())),
                ..Settings::default()
            },
        );
        running(cx, &file);
        let w = open_settings(cx);
        pick_theme(cx, w, "ayu light");
        assert_eq!(applied(cx), "Ayu Light");
        cx.update(|cx| shell::close(w, cx));
        cx.run_until_parked();
        assert!(!is_open(cx, w));
        assert_eq!(applied(cx), "Kibble");
        assert_eq!(settings(cx).theme, Some(Name("Kibble".into())));
        assert_eq!(file.load().unwrap().theme, Some(Name("Kibble".into())));
    }

    #[gpui_kit::test]
    fn picking_a_theme_keeps_the_open_command_form(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        running(cx, &file);
        let (w, view) = open_settings_view(cx);
        click(cx, w, "settings-command-add");
        click(cx, w, "settings-command-label");
        typed(cx, w, "Neovim");
        pick_theme(cx, w, "ayu dark");
        assert_eq!(applied(cx), "Ayu Dark");
        let (open, label) = view.read_with(cx, |v, cx| {
            (
                v.form.is_some(),
                v.form.as_ref().map(|f| f.label.read(cx).value().clone()),
            )
        });
        assert!(open, "the form survives the pick");
        assert_eq!(label, Some("Neovim".into()), "the field keeps its text");
    }

    #[gpui_kit::test]
    fn picking_follow_system_resolves_and_persists(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        running(cx, &file);
        let (w, view) = open_settings_view(cx);
        pick_theme(cx, w, "system");
        // The pick resolves from the system's appearance (the test platform reports
        // light): Follow System's light resolution, same route a named pick takes
        // (spec story 12).
        assert_eq!(applied(cx), "Default Light");
        assert_eq!(draft_theme(cx, &view), Some(FollowSystem));
        // Unlike the other prefs, the pick leaves the stored choice alone.
        assert_eq!(settings(cx).theme, None);

        click(cx, w, "settings-save");
        assert!(!is_open(cx, w));
        assert_eq!(settings(cx).theme, Some(FollowSystem));
        assert_eq!(file.load().unwrap().theme, Some(FollowSystem));
        assert_eq!(
            applied(cx),
            "Default Light",
            "Save keeps the resolution live"
        );

        // Reopen shows the stored choice committed in the picker (story 4).
        let w = open_settings(cx);
        assert_eq!(
            label(cx, w, "theme-picker"),
            "Theme, Follow System, opens theme list"
        );
        click(cx, w, "theme-picker");
        typed(cx, w, "system");
        assert_eq!(
            theme_rows(cx, w),
            ["Follow System, follows system appearance, selected"]
        );
    }

    /// macOS's appearance changed while Settings is open: Grove rethemes live — the
    /// simulated notification routes the same core the per-window callbacks run
    /// (story 13) — and Cancel re-applies the entry choice AS STORED: Follow System
    /// re-resolved from the appearance it reads now, not the resolution the change
    /// left standing and not a theme name (story 16, this ticket's entry semantics).
    #[gpui_kit::test]
    fn cancel_after_an_appearance_change_re_applies_the_entry_choice(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(
            &tmp,
            Settings {
                theme: Some(FollowSystem),
                ..Default::default()
            },
        );
        running(cx, &file);
        let w = open_settings(cx);
        cx.update(|cx| assert!(theme::follow_appearance_changed(WindowAppearance::Dark, cx)));
        assert_eq!(applied(cx), "Grove Dark");
        click(cx, w, "settings-cancel");
        assert!(!is_open(cx, w));
        // The test platform reports light, so Cancel re-resolves light — proving the
        // re-resolve (a pinned resolution would keep Grove Dark).
        assert_eq!(applied(cx), "Default Light");
        assert_eq!(settings(cx).theme, Some(FollowSystem));
        assert_eq!(file.load().unwrap().theme, Some(FollowSystem));
    }

    /// A settings file persisted under Follow System opens on its light resolution
    /// here (the test platform reports light), and the picker opens committed on the
    /// Follow System row.
    #[gpui_kit::test]
    fn launch_with_follow_system_resolves_and_the_picker_agrees(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(
            &tmp,
            Settings {
                theme: Some(FollowSystem),
                ..Default::default()
            },
        );
        running(cx, &file);
        assert_eq!(applied(cx), "Default Light");
        let w = open_settings(cx);
        assert_eq!(
            label(cx, w, "theme-picker"),
            "Theme, Follow System, opens theme list"
        );
    }
}
