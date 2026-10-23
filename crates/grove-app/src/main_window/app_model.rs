use gpui_kit::{App, Context};
use grove_core::repo_order;

use crate::shell;

use super::MainWindow;

/// The saved repo if it still exists, else the first.
pub(super) fn selected_repo(cx: &App) -> Option<String> {
    let library = shell::library(cx);
    let snapshot = library.store.snapshot();
    library
        .settings
        .last_selected_repo
        .as_deref()
        .and_then(|name| snapshot.repo(name))
        .or_else(|| {
            repo_order(
                snapshot,
                &library.settings.pinned_repos,
                &library.settings.hidden_repos,
                &library.settings.last_opened,
                library.settings.repo_sort,
            )
            .iter()
            .next()
        })
        .map(|r| r.name().to_string())
}

impl MainWindow {
    /// Pushes fresher snapshot data for the shown repo into the tree, or shows another if
    /// it's gone (e.g. outside a new code folder).
    pub(super) fn sync_repo(&mut self, cx: &mut Context<Self>) {
        if !cx.has_global::<shell::Library>() {
            return;
        }
        let mode = shell::library(cx).settings.repo_sort;
        self.repo_sort.update(cx, |m, cx| m.set_mode(mode, cx));
        self.sync_tree_order(cx);
        let name = self.tree.read(cx).repo_name().map(str::to_string);
        let snapshot = shell::library(cx).store.snapshot().clone();
        if name.as_deref().and_then(|n| snapshot.repo(n)).is_some() {
            self.tree
                .update(cx, |tree, cx| tree.update_snapshot(snapshot, cx));
        } else {
            self.show_repo(selected_repo(cx).as_deref(), cx);
        }
        cx.notify();
    }

    /// Shows each finished Fetch all's outcome once.
    pub(super) fn fetch_changed(&mut self, cx: &mut Context<Self>) {
        let fetcher = cx.global::<shell::Fetcher>();
        if fetcher.finished == self.fetch_seen {
            return cx.notify();
        }
        self.fetch_seen = fetcher.finished;
        let Some(report) = &fetcher.report else {
            return cx.notify();
        };
        let (text, error) = (report.message(), !report.failures.is_empty());
        self.open_seq += 1;
        self.set_status(text, error, false, cx);
    }

    pub(super) fn notice_posted(&mut self, cx: &mut Context<Self>) {
        let notices = cx.global::<shell::Notices>();
        if notices.posted == self.notices_seen {
            return;
        }
        self.notices_seen = notices.posted;
        if let Some(status) = notices.last.clone() {
            shell::notice_shown(cx);
            self.open_seq += 1;
            self.status = Some(status);
            cx.notify();
        }
    }

    /// The shown repo, so a reorder never moves the selection; else the saved one or the first.
    pub(super) fn selected(&self, cx: &App) -> Option<String> {
        self.tree
            .read(cx)
            .repo_name()
            .filter(|n| shell::library(cx).store.snapshot().repo(n).is_some())
            .map(str::to_string)
            .or_else(|| selected_repo(cx))
    }

    fn sync_tree_order(&self, cx: &mut Context<Self>) {
        let settings = &shell::library(cx).settings;
        let (sort, last_opened) = (settings.tree_sort, settings.last_opened.clone());
        self.tree
            .update(cx, |tree, cx| tree.set_order(sort, last_opened, cx));
    }

    pub(super) fn show_repo(&mut self, name: Option<&str>, cx: &mut Context<Self>) {
        self.sync_tree_order(cx);
        let library = shell::library(cx);
        let snapshot = library.store.snapshot().clone();
        let repo_name = name
            .filter(|n| snapshot.repo(n).is_some())
            .map(str::to_string);
        let collapsed = repo_name
            .as_deref()
            .map(|n| library.settings.collapsed(n).to_vec())
            .unwrap_or_default();
        self.tree.update(cx, |tree, cx| {
            tree.set_snapshot(Some(snapshot), repo_name, collapsed, cx)
        });
    }
}
