use std::path::Path;

use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyWindowHandle, App, AppContext, Bounds, Context, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, TitlebarOptions,
    Window, WindowBounds, WindowKind, WindowOptions, div, px, size,
};
use grove_core::RepoSnapshot;

use crate::{shell, theme};

struct MainWindow;

impl Render for MainWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let library = shell::library(cx);
        let snapshot = library.snapshot.clone();
        let selected = library
            .settings
            .last_selected_repo
            .as_deref()
            .and_then(|name| snapshot.repo(name))
            .or_else(|| snapshot.repos.first())
            .map(|r| r.name().to_string());

        let rows = snapshot.repos.iter().enumerate().map(|(i, repo)| {
            let is_selected = selected.as_deref() == Some(repo.name());
            let name = repo.name().to_string();
            div()
                .id(("repo", i))
                .h(px(34.))
                .px(px(10.))
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.))
                .rounded(px(6.))
                .text_size(px(13.))
                .cursor_pointer()
                .when_else(
                    is_selected,
                    |d| d.bg(theme::selected()).text_color(theme::text()),
                    |d| {
                        d.text_color(theme::text_soft())
                            .hover(|s| s.bg(theme::raised()))
                    },
                )
                .on_click(cx.listener(move |_, _, _, cx| {
                    shell::select_repo(&name, cx);
                    cx.notify();
                }))
                .child(theme::mono(repo.name().to_string()).truncate())
                .child(
                    div()
                        .flex_none()
                        .text_size(px(11.))
                        .text_color(theme::link())
                        .child(format!("{} wt", repo.worktree_count())),
                )
        });

        let sidebar = div()
            .w(px(240.))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .bg(theme::sidebar())
            .border_r_1()
            .border_color(theme::border())
            .child(
                div()
                    .px(px(18.))
                    .pt(px(14.))
                    .pb(px(8.))
                    .text_size(px(11.))
                    .text_color(theme::faint())
                    .child(format!("REPOSITORIES · {}", snapshot.repos.len())),
            )
            .child(
                div()
                    .id("repo-list")
                    .flex_1()
                    .overflow_y_scroll()
                    .px(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .children(rows),
            )
            .child(
                div()
                    .m(px(10.))
                    .p(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::bg())
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme::faint())
                            .child("CODE FOLDER"),
                    )
                    .child(
                        theme::path_text(tilde(&snapshot.code_dir))
                            .text_size(px(13.))
                            .text_color(theme::text()),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(theme::faint())
                            .child("Grove only looks inside this folder."),
                    ),
            );

        let content = match selected.as_deref().and_then(|n| snapshot.repo(n)) {
            Some(repo) => repo_detail(repo).into_any_element(),
            None => div()
                .p(px(16.))
                .text_color(theme::muted())
                .child("No repos")
                .into_any_element(),
        };

        div()
            .size_full()
            .flex()
            .bg(theme::bg())
            .text_color(theme::text())
            .child(sidebar)
            .child(content)
    }
}

fn repo_detail(repo: &RepoSnapshot) -> impl IntoElement {
    let count = repo.worktree_count();
    let rows = repo.worktrees.iter().map(|wt| {
        let label: SharedString = match (&wt.branch, &wt.head) {
            (Some(b), _) => b.clone().into(),
            (None, Some(h)) => format!("detached @ {}", &h[..h.len().min(7)]).into(),
            (None, None) => "".into(),
        };
        div()
            .px(px(12.))
            .py(px(8.))
            .flex()
            .items_center()
            .gap(px(12.))
            .rounded(px(6.))
            .bg(theme::panel())
            .child(
                theme::mono(label)
                    .w(px(220.))
                    .flex_none()
                    .text_size(px(13.))
                    .truncate(),
            )
            .child(
                theme::path_text(tilde(&wt.path))
                    .flex_1()
                    .text_size(px(12.))
                    .text_color(theme::muted()),
            )
            .children(wt.is_main.then(|| {
                div()
                    .flex_none()
                    .text_size(px(11.))
                    .text_color(theme::faint())
                    .child("main worktree")
            }))
    });
    div()
        .id("repo-detail")
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_y_scroll()
        .p(px(16.))
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(px(10.))
                .pb(px(8.))
                .child(
                    theme::mono(repo.name().to_string())
                        .text_size(px(16.))
                        .font_weight(FontWeight::SEMIBOLD),
                )
                .child(div().text_size(px(13.)).text_color(theme::muted()).child(
                    match (count, repo.repo.bare) {
                        (1, false) => "1 worktree".to_string(),
                        (1, true) => "bare · 1 worktree".to_string(),
                        (n, false) => format!("{n} worktrees"),
                        (n, true) => format!("bare · {n} worktrees"),
                    },
                )),
        )
        .children(repo.error.as_ref().map(|e| {
            div()
                .p(px(10.))
                .rounded(px(6.))
                .bg(theme::warn_bg())
                .text_color(theme::warn_text())
                .text_size(px(13.))
                .child(e.to_string())
        }))
        .children(rows)
}

/// `/Users/me/code` → `~/code`.
pub fn tilde(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    match home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

pub fn open(cx: &mut App) -> Option<AnyWindowHandle> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(960.), px(640.)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some("Grove".into()),
            ..Default::default()
        }),
        focus: true,
        show: true,
        kind: WindowKind::Normal,
        ..Default::default()
    };
    let result = gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| MainWindow));
    result
        .map(|(handle, _)| handle)
        .inspect_err(|e| eprintln!("[grove] main: {e:#}"))
        .ok()
}
