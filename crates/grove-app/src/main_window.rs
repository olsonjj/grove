use std::path::Path;

use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyWindowHandle, App, AppContext, Bounds, Context, Entity, Focusable, InteractiveElement,
    IntoElement, ParentElement, Render, StatefulInteractiveElement, Styled, Subscription,
    TitlebarOptions, Window, WindowBounds, WindowKind, WindowOptions, div, px, size,
};

use crate::tree_view::{CollapseChanged, TreeView};
use crate::{shell, theme};

struct MainWindow {
    tree: Entity<TreeView>,
    _collapse: Subscription,
}

/// The saved repo if it still exists, else the first.
fn selected_repo(cx: &App) -> Option<String> {
    let library = shell::library(cx);
    let snapshot = &library.snapshot;
    library
        .settings
        .last_selected_repo
        .as_deref()
        .and_then(|name| snapshot.repo(name))
        .or_else(|| snapshot.repos.first())
        .map(|r| r.name().to_string())
}

impl MainWindow {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let tree = cx.new(TreeView::new);
        let collapse = cx.subscribe(&tree, |_, _, ev: &CollapseChanged, cx| {
            shell::set_collapsed(&ev.repo, &ev.prefix, ev.collapsed, cx);
        });
        window.focus(&tree.focus_handle(cx), cx);
        let mut this = Self {
            tree,
            _collapse: collapse,
        };
        this.show_repo(selected_repo(cx), cx);
        this
    }

    fn show_repo(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        let library = shell::library(cx);
        let repo = name
            .as_deref()
            .and_then(|n| library.snapshot.repo(n))
            .cloned();
        let collapsed = name
            .as_deref()
            .map(|n| library.settings.collapsed(n).to_vec())
            .unwrap_or_default();
        self.tree
            .update(cx, |tree, cx| tree.set_repo(repo, collapsed, cx));
    }
}

impl Render for MainWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let library = shell::library(cx);
        let snapshot = library.snapshot.clone();
        let selected = selected_repo(cx);

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
                .on_click(cx.listener(move |this, _, window, cx| {
                    shell::select_repo(&name, cx);
                    if this.tree.read(cx).repo_name() != Some(name.as_str()) {
                        this.show_repo(Some(name.clone()), cx);
                    }
                    window.focus(&this.tree.focus_handle(cx), cx);
                    cx.notify();
                }))
                .child(theme::mono(repo.display_name.clone()).truncate())
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

        let content = if snapshot.repos.is_empty() {
            div()
                .p(px(16.))
                .text_color(theme::muted())
                .child("No repos")
                .into_any_element()
        } else {
            self.tree.clone().into_any_element()
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

/// `/Users/me/code` → `~/code`.
pub fn tilde(path: &Path) -> String {
    grove_core::tilde(path, grove_core::home_dir().as_deref())
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
    open_with(options, cx)
        .map(|(handle, _)| handle)
        .inspect_err(|e| eprintln!("[grove] main: {e:#}"))
        .ok()
}

fn open_with(
    options: WindowOptions,
    cx: &mut App,
) -> gpui_kit::Result<(AnyWindowHandle, Entity<MainWindow>)> {
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| MainWindow::new(window, cx))
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{Point, TestAppContext};
    use grove_core::{RowId, RowKind, Settings, SettingsFile, Snapshot};
    use tempfile::TempDir;

    use super::*;
    use crate::shell::Library;
    use crate::tree_view::tests::canned;

    fn launch(
        cx: &mut TestAppContext,
        file: &SettingsFile,
    ) -> (AnyWindowHandle, Entity<MainWindow>) {
        let settings = file.load().unwrap();
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Library {
                settings_file: file.clone(),
                settings,
                snapshot: Arc::new(Snapshot {
                    code_dir: PathBuf::from("/c"),
                    code_dir_exists: true,
                    repos: vec![canned()],
                }),
                saves: Default::default(),
            });
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(960.), px(640.)),
                })),
                ..Default::default()
            };
            open_with(options, cx).unwrap()
        })
    }

    fn folder_collapsed(cx: &mut TestAppContext, main: &Entity<MainWindow>, prefix: &str) -> bool {
        main.read_with(cx, |m, cx| {
            m.tree.read(cx).tree().rows.iter().any(|r| {
                r.id == RowId::Folder(prefix.into())
                    && matches!(
                        r.kind,
                        RowKind::Folder {
                            collapsed: true,
                            ..
                        }
                    )
            })
        })
    }

    #[gpui_kit::test]
    fn collapse_by_key_is_saved_and_restored(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = SettingsFile::new(tmp.path().join("settings.json"));
        file.save(&Settings {
            last_selected_repo: Some("api".into()),
            ..Default::default()
        })
        .unwrap();

        let (w, main) = launch(cx, &file);
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            // main → main worktree → feature/, then collapse it.
            for k in ["down", "down", "left"] {
                window.press(k, cx);
            }
        })
        .unwrap();
        cx.run_until_parked();
        assert!(folder_collapsed(cx, &main, "feature/"));
        assert_eq!(file.load().unwrap().collapsed("api"), ["feature/"]);

        cx.update_window(w, |_, window, cx| {
            window.press("right", cx);
            window.press("left", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(file.load().unwrap().collapsed("api"), ["feature/"]);
        cx.update_window(w, |_, window, cx| window.press("right", cx))
            .unwrap();
        cx.run_until_parked();
        assert!(file.load().unwrap().collapsed("api").is_empty());
        cx.update_window(w, |_, window, cx| window.press("left", cx))
            .unwrap();
        cx.run_until_parked();
        cx.update(|cx| cx.remove_global::<Library>());

        // Fresh load from the same file restores it.
        let (_, main) = launch(cx, &file);
        assert!(folder_collapsed(cx, &main, "feature/"));
        assert!(!folder_collapsed(cx, &main, "fix/"));
    }
}
