use std::path::Path;

use gpui_kit::TestAppContext;
use grove_core::{Outcome, Presence, RowId, Settings};
use tempfile::TempDir;

use super::super::*;
use super::common::*;
use crate::test_support::{canned, fake_launcher, fake_presence, settings_file};

#[gpui_kit::test]
fn remove_is_offered_on_linked_worktrees_only(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    fake_launcher(cx, Ok(Outcome::Opened));
    let mut repo = canned();
    repo.worktrees[2].locked = true; // api-y
    repo.worktrees[3].presence = Presence::Missing; // api-z
    fake_presence(cx, "/c/wt/api-z", Presence::Missing);
    let (w, main) = launch_repo(cx, &file, repo);
    let row = |cx: &mut TestAppContext, p: &str| row_of(cx, &main, Path::new(p));

    let ix = row(cx, "/c/api");
    click(cx, w, ("tree-row", ix));
    assert!(!found(cx, w, "details-remove"));
    right_click(cx, w, ("tree-row", ix));
    assert!(!found(cx, w, "tree-menu"));

    let ix = row(cx, "/c/wt/api-x");
    click(cx, w, ("tree-row", ix));
    assert_eq!(label_of(cx, w, "details-remove"), "Remove worktree…");
    assert!(!found(cx, w, "details-remove-reason"));
    right_click(cx, w, ("tree-row", ix));
    assert_eq!(label_of(cx, w, "tree-menu-remove"), "Remove worktree…");
    keys(cx, w, &["escape"]);
    assert!(!found(cx, w, "tree-menu"));
    // A click outside closes it too, handing focus back to the tree.
    right_click(cx, w, ("tree-row", ix));
    click(cx, w, "details-remove");
    assert!(!found(cx, w, "tree-menu"));
    assert!(focused(cx, w, "tree-view"));
    assert!(main.read_with(cx, |m, _| m.removal.is_none()));

    let ix = row(cx, "/c/wt/api-y");
    click(cx, w, ("tree-row", ix));
    let locked = "Can't remove: it's locked (run git worktree unlock first)";
    assert_eq!(label_of(cx, w, "details-remove-reason"), locked);
    // Disabled: clicks do nothing.
    click(cx, w, "details-remove");
    right_click(cx, w, ("tree-row", ix));
    assert_eq!(label_of(cx, w, "tree-menu-reason"), locked);
    click(cx, w, "tree-menu-remove");
    assert!(main.read_with(cx, |m, _| m.removal.is_none()));
    assert_eq!(status(cx, &main), None);
    keys(cx, w, &["escape"]);
    assert!(!found(cx, w, "tree-menu"));

    let ix = row(cx, "/c/wt/api-z");
    click(cx, w, ("tree-row", ix));
    assert_eq!(label_of(cx, w, "details-remove"), "Remove missing entry…");
    right_click(cx, w, ("tree-row", ix));
    assert_eq!(label_of(cx, w, "tree-menu-remove"), "Remove missing entry…");
}

#[gpui_kit::test]
fn clean_remove_confirms_then_removes_without_force(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let removals = fake_remover(cx, None);
    let (w, main, file, wt) = removal_fixture(cx, &tmp, Settings::default());
    let a = wt.join("a");
    cx.update(|cx| shell::record_open(&a, cx));
    cx.run_until_parked();
    assert!(file.load().unwrap().last_opened.contains_key(&a));

    let ix = row_of(cx, &main, &a);
    click(cx, w, ("tree-row", ix));
    click(cx, w, "details-remove");
    let message = label_of(cx, w, "remove-message");
    assert!(
        message.starts_with(&format!("Remove {}?", a.display())),
        "{message}"
    );
    assert!(message.contains("Branch a is kept."), "{message}");
    assert!(!message.contains("permanently"), "{message}");
    assert_eq!(label_of(cx, w, "remove-confirm"), "Remove");
    assert!(focused(cx, w, "remove-cancel"));

    click(cx, w, "remove-confirm");
    assert_eq!(*removals.lock().unwrap(), [(a.clone(), false)]);
    assert!(!found(cx, w, "remove-message"));
    assert!(!a.exists());
    let s = status(cx, &main).unwrap();
    assert_eq!(s.text, format!("Removed {}", tilde(&a)));
    assert!(!s.error);
    // Its rows are gone; the next row (b's branch) takes the selection.
    assert!(main.read_with(cx, |m, cx| {
        m.tree
            .read(cx)
            .tree()
            .index_of(&RowId::Worktree(a.clone()))
            .is_none()
    }));
    assert_eq!(selected_row(cx, &main), Some(RowId::Branch("b".into())));
    assert!(!file.load().unwrap().last_opened.contains_key(&a));
}

#[gpui_kit::test]
fn dirty_remove_counts_changes_and_forces_after_cancel_and_esc(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let removals = fake_remover(cx, None);
    let (w, main, _file, wt) = removal_fixture(cx, &tmp, Settings::default());
    let b = wt.join("b");
    let ix = row_of(cx, &main, &b);
    let open_confirm = |cx: &mut TestAppContext| {
        right_click(cx, w, ("tree-row", ix));
        click(cx, w, "tree-menu-remove");
    };

    open_confirm(cx);
    let message = label_of(cx, w, "remove-message");
    assert!(
        message.contains("1 modified and 2 untracked files will be permanently deleted"),
        "{message}"
    );
    assert_eq!(label_of(cx, w, "remove-confirm"), "Discard and remove");
    assert!(focused(cx, w, "remove-cancel"));
    // Enter on the focused Cancel, Esc, and a Cancel click each leave it be.
    keys(cx, w, &["enter"]);
    assert!(!found(cx, w, "remove-message"));
    open_confirm(cx);
    keys(cx, w, &["escape"]);
    assert!(!found(cx, w, "remove-message"));
    open_confirm(cx);
    click(cx, w, "remove-cancel");
    assert!(!found(cx, w, "remove-message"));
    assert!(removals.lock().unwrap().is_empty());
    assert!(b.join("new/one").exists());
    assert_eq!(status(cx, &main), None);

    open_confirm(cx);
    click(cx, w, "remove-confirm");
    assert_eq!(*removals.lock().unwrap(), [(b.clone(), true)]);
    assert!(!b.exists());
    // The next row is c's branch.
    assert_eq!(selected_row(cx, &main), Some(RowId::Branch("c".into())));
}

#[gpui_kit::test]
fn last_row_removed_selects_the_previous(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    fake_remover(cx, None);
    let (w, main, _file, wt) = removal_fixture(cx, &tmp, Settings::default());
    let c = wt.join("c");
    let ix = row_of(cx, &main, &c);
    click(cx, w, ("tree-row", ix));
    click(cx, w, "details-remove");
    assert!(found(cx, w, "remove-message"));
    keys(cx, w, &["tab", "enter"]);
    assert!(!c.exists());
    assert_eq!(selected_row(cx, &main), Some(RowId::Worktree(wt.join("b"))));
}

#[gpui_kit::test]
fn failed_remove_shows_gits_error(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let removals = fake_remover(cx, Some("fatal: 'x' is locked\n"));
    let (w, main, _file, wt) = removal_fixture(cx, &tmp, Settings::default());
    let a = wt.join("a");
    let ix = row_of(cx, &main, &a);
    click(cx, w, ("tree-row", ix));
    click(cx, w, "details-remove");
    assert!(found(cx, w, "remove-message"));
    click(cx, w, "remove-confirm");
    assert_eq!(removals.lock().unwrap().len(), 1);
    assert!(!found(cx, w, "remove-message"));
    assert!(a.exists());
    let s = status(cx, &main).unwrap();
    assert_eq!(
        s.text,
        format!("Couldn't remove {}: 'x' is locked", tilde(&a))
    );
    assert!(s.error);
    assert_eq!(selected_row(cx, &main), Some(RowId::Worktree(a)));
}

#[gpui_kit::test]
fn missing_worktree_removes_only_its_entry_after_a_plain_confirm(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let removals = fake_remover(cx, None);
    let (w, main, _file, wt) = removal_fixture_with(cx, &tmp, Settings::default(), |wt| {
        std::fs::remove_dir_all(wt.join("a")).unwrap();
        std::fs::remove_dir_all(wt.join("c")).unwrap();
    });
    let c = wt.join("c");
    let ix = row_of(cx, &main, &c);
    click(cx, w, ("tree-row", ix));
    assert_eq!(label_of(cx, w, "details-remove"), "Remove missing entry…");
    click(cx, w, "details-remove");
    let message = label_of(cx, w, "remove-message");
    assert!(message.contains("is already gone"), "{message}");
    assert_eq!(label_of(cx, w, "remove-confirm"), "Remove entry");
    click(cx, w, "remove-confirm");
    assert_eq!(*removals.lock().unwrap(), [(c.clone(), false)]);
    assert_eq!(
        status(cx, &main).unwrap().text,
        format!("Removed {}", tilde(&c))
    );
    // a, also missing, keeps its entry.
    let rows = main.read_with(cx, |m, cx| {
        let tree = m.tree.read(cx).tree();
        [wt.join("a"), c.clone()].map(|p| tree.index_of(&RowId::Worktree(p)).is_some())
    });
    assert_eq!(rows, [true, false]);
}

#[gpui_kit::test]
fn confirm_refreshes_instead_of_removing_when_the_worktree_changed(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let removals = fake_remover(cx, None);
    let (w, main, _file, wt) = removal_fixture(cx, &tmp, Settings::default());
    let a = wt.join("a");
    let ix = row_of(cx, &main, &a);
    click(cx, w, ("tree-row", ix));
    click(cx, w, "details-remove");
    assert_eq!(label_of(cx, w, "remove-confirm"), "Remove");

    std::fs::write(a.join("late"), "x").unwrap();
    click(cx, w, "remove-confirm");

    assert!(removals.lock().unwrap().is_empty());
    let message = label_of(cx, w, "remove-message");
    assert!(message.contains("1 untracked file will be"), "{message}");
    assert_eq!(label_of(cx, w, "remove-confirm"), "Discard and remove");
    // Back on Cancel, so a second Enter doesn't discard what it just revealed.
    assert!(focused(cx, w, "remove-cancel"));
    click(cx, w, "remove-confirm");
    assert_eq!(*removals.lock().unwrap(), [(a.clone(), true)]);
    assert!(!a.exists());
}

#[gpui_kit::test]
fn worktree_dirtied_after_a_clean_confirm_is_kept(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    // Dirtied after the confirm's last look, just before git runs.
    let removals = fake_remover_with(cx, None, |path| {
        std::fs::write(path.join("late"), "x").unwrap()
    });
    let (w, main, _file, wt) = removal_fixture(cx, &tmp, Settings::default());
    let a = wt.join("a");
    let ix = row_of(cx, &main, &a);
    click(cx, w, ("tree-row", ix));
    click(cx, w, "details-remove");
    assert!(found(cx, w, "remove-message"));
    click(cx, w, "remove-confirm");

    assert_eq!(*removals.lock().unwrap(), [(a.clone(), false)]);
    assert!(!found(cx, w, "remove-message"));
    assert!(a.join("late").exists());
    let s = status(cx, &main).unwrap();
    assert!(
        s.text
            .starts_with(&format!("Couldn't remove {}: ", tilde(&a)))
            && s.text.contains("contains modified or untracked files"),
        "{}",
        s.text
    );
    assert!(s.error);
}

#[gpui_kit::test]
fn submodules_refuse_removal_in_the_status_line(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let removals = fake_remover(cx, None);
    let (w, main, _file, wt) = removal_fixture(cx, &tmp, Settings::default());
    let a = wt.join("a");
    // What an initialized submodule leaves in the worktree's admin folder.
    let admin = tmp.path().join("code/api/.git/worktrees/a/modules");
    std::fs::create_dir_all(admin).unwrap();
    let ix = row_of(cx, &main, &a);
    click(cx, w, ("tree-row", ix));
    click(cx, w, "details-remove");
    assert!(!found(cx, w, "remove-message"));
    assert!(removals.lock().unwrap().is_empty());
    assert_eq!(
        status(cx, &main).unwrap().text,
        format!(
            "Can't remove {}: it contains submodules (remove it with git)",
            tilde(&a)
        )
    );
}
