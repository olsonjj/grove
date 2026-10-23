use gpui_kit::test::TestWindowExt;
use gpui_kit::{InputEvent, MouseButton, MouseUpEvent, TestAppContext, point, px, size};
use grove_core::Settings;
use tempfile::TempDir;

use super::super::layout::{
    DETAILS_PANE_MAX_WIDTH, DETAILS_PANE_MIN_WIDTH, DETAILS_PANE_WIDTH, SIDEBAR_W, TREE_MIN_WIDTH,
};
use super::super::*;
use super::common::*;
use crate::shell::Library;
use crate::test_support::{canned, settings_file};

#[gpui_kit::test]
fn resizable_pane_starts_at_320_and_sidebar_stays_put(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    let (w, _) = launch(cx, &file);
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        let details = window.find("details").bounds();
        assert_close(details.right() - details.left(), 320., "fresh pane width");
        // The repo sidebar keeps today's exact footprint:
        // fixed column, flush to the left edge.
        let sidebar = window.find("repo-list").bounds();
        assert_eq!(sidebar.left().as_f32(), 0.);
        assert_close(sidebar.right(), SIDEBAR_W - 1., "sidebar right edge");
    })
    .unwrap();
}

#[gpui_kit::test]
fn resizable_drag_trades_width_and_saves_on_release(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    // Wide enough for a 150 px drag not to reach the tree's own minimum.
    let (w, _) = launch_at(cx, &file, canned(), size(px(1500.), px(720.)));

    // Dragging left widens the pane at the tree's expense.
    drag_divider(cx, w, 150.);
    let pane = cx
        .update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.find("details").bounds()
        })
        .unwrap();
    assert_close(pane.right() - pane.left(), 470., "pane after drag");
    // The tree flexes to exactly what the pane took:
    // pane gained 150 px, tree lost them. Expected tree width comes
    // from the launch above: window width minus sidebar minus pane.
    let tree_after = tree_panel_width(cx, w);
    let tree_before = 1500. - SIDEBAR_W - DETAILS_PANE_WIDTH;
    assert_close(px(tree_after), tree_before - 150., "tree after drag");
    // The write happens once, at drag end, through the saved settings file.
    assert_eq!(
        file.load().unwrap().details_pane_width,
        Some(pane_width(cx, w).round() as i32)
    );
}

#[gpui_kit::test]
fn resizable_clamps_pane_to_260_and_720(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    // Wide enough that the pane's own max (720) and the tree's min (480) can
    // both bind, without one masking the other.
    let (w, _) = launch_at(cx, &file, canned(), size(px(1560.), px(720.)));

    // Far past either stop: the drag lands exactly on the clamp.
    drag_divider(cx, w, 1000.);
    assert_eq!(pane_width(cx, w).round() as i64, 720);
    drag_divider(cx, w, -2000.);
    assert_eq!(pane_width(cx, w).round() as i64, 260);
    // The tree keeps its own minimum in both cases, and the pane's buttons
    // render inside its bounds even at the 260 px minimum (story: nothing
    // overflows or overlaps at any width in the clamp range).
    assert!(tree_panel_width(cx, w) >= 480.);
    let pane = cx
        .update_window(w, |_, window, _| window.find("details").bounds())
        .unwrap();
    for what in [
        "details-open-editor",
        "details-open-terminal",
        "details-copy-path",
    ] {
        let button = cx
            .update_window(w, |_, window, _| window.find(what).bounds())
            .unwrap();
        assert!(
            button.left() >= pane.left() && button.right() <= pane.right(),
            "{what} must sit inside the clamped pane"
        );
    }
}

#[gpui_kit::test]
fn resizable_width_survives_a_selection_change(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    let (w, main) = launch_at(cx, &file, canned(), size(px(1500.), px(720.)));
    resize_pane(cx, &main, w, 470.);

    keys(cx, w, &["down"]);
    assert!((pane_width(cx, w) - 470.).abs() < 1.);
}

#[gpui_kit::test]
fn pane_saves_when_the_drag_ends_not_while_it_moves(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    let (w, _) = launch_at(cx, &file, canned(), size(px(1560.), px(720.)));
    let divider = cx
        .update_window(w, |_, window, _| window.find("details").bounds())
        .unwrap()
        .origin;
    let to = point(divider.x - px(150.), divider.y);

    // Press, move, and leave the drag open: no save may have landed while
    // the pointer moves (settings writes happen on background threads, so
    // parking first would flush a mid-move save if one existed).
    pressed_divider_drag(cx, w, divider, to);
    cx.run_until_parked();
    assert_eq!(file.load().unwrap().details_pane_width, None);
    assert_eq!(
        pane_width(cx, w).round() as i64,
        470,
        "pane grew while dragging"
    );

    // Release the still-open drag: the single write carries the final width.
    cx.update_window(w, |_, window, cx| {
        window.dispatch_event(
            MouseUpEvent {
                button: MouseButton::Left,
                position: to,
                modifiers: Default::default(),
                click_count: 1,
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(file.load().unwrap().details_pane_width, Some(470));
    assert_eq!(pane_width(cx, w).round() as i64, 470);
}

#[gpui_kit::test]
fn resizable_width_is_restored_on_relaunch(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let file = settings_file(&tmp, Settings::default());
    let (w, main) = launch_at(cx, &file, canned(), size(px(1500.), px(720.)));
    resize_pane(cx, &main, w, 470.);
    assert_eq!(file.load().unwrap().details_pane_width, Some(470));

    cx.update(|cx| cx.remove_global::<Library>());
    // Same window size as the first launch, so the saved width fits.
    let (w, _) = launch_at(cx, &file, canned(), size(px(1500.), px(720.)));
    assert!((pane_width(cx, w) - 470.).abs() < 1.);
}

#[gpui_kit::test]
fn saved_max_width_compressed_keep_tree_at_its_minimum(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    // Saved at the pane's maximum, in the wide window it was earned in.
    let file = settings_file(
        &tmp,
        Settings {
            details_pane_width: Some(DETAILS_PANE_MAX_WIDTH as i32),
            ..Settings::default()
        },
    );
    // The app's real default window (1180 px) is too narrow to honor the
    // saved 720: the tree holds its 480 minimum and the pane absorbs the
    // rest rather than overflowing the window.
    let (w, _) = launch_at(cx, &file, canned(), size(px(1180.), px(720.)));
    assert_close(
        px(pane_width(cx, w)),
        1180. - SIDEBAR_W - TREE_MIN_WIDTH,
        "pane at launch",
    );
    assert!(
        tree_panel_width(cx, w) >= TREE_MIN_WIDTH,
        "tree kept its minimum"
    );
    assert!(
        pane_width(cx, w) >= DETAILS_PANE_MIN_WIDTH,
        "pane kept its minimum"
    );
}
