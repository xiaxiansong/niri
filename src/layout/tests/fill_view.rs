use insta::assert_snapshot;

use super::*;

fn window(id: usize, width: i32) -> Op {
    let mut params = TestWindowParams::new(id);
    params.bbox = Rectangle::from_size(Size::from((width, 200)));
    Op::AddWindow { params }
}

fn floating_window(id: usize, width: i32) -> Op {
    let mut params = TestWindowParams::new(id);
    params.bbox = Rectangle::from_size(Size::from((width, 200)));
    params.is_floating = true;
    Op::AddWindow { params }
}

/// View position of the first workspace on the first output.
fn first_ws_view_pos(layout: &Layout<TestWindow>) -> f64 {
    let (_, _, ws) = layout.workspaces().next().unwrap();
    ws.scrolling().view_pos()
}

fn make_options(fill: bool) -> Options {
    let mut options = Options::default();
    options.layout.fill_view_after_removal = fill;
    options
}

/// Runs the ops with fill-view-after-removal off and on, and returns the resulting view positions
/// of the first workspace after completing the animations.
fn off_on(options: Options, ops: &[Op]) -> String {
    let run = |fill| {
        let mut options = options.clone();
        options.layout.fill_view_after_removal = fill;
        let mut layout = check_ops_with_options(options, ops.iter().cloned());
        Op::CompleteAnimations.apply(&mut layout);
        first_ws_view_pos(&layout)
    };

    format!("off: {}, on: {}", run(false), run(true))
}

fn assert_active_column_visible(layout: &Layout<TestWindow>) {
    let (_, _, ws) = layout.workspaces().next().unwrap();
    let scrolling = ws.scrolling();
    let id = scrolling
        .active_column()
        .unwrap()
        .tiles()
        .next()
        .unwrap()
        .0
        .window()
        .id();
    let (tile, pos, _) = ws
        .tiles_with_render_positions()
        .find(|(tile, _, _)| tile.window().id() == id)
        .unwrap();
    let view_width = ws.view_size().w;
    assert!(
        0. <= pos.x && pos.x + tile.tile_size().w <= view_width,
        "active column must be visible: x {}, width {}",
        pos.x,
        tile.tile_size().w,
    );
}

/// Four 500 px wide columns at x = 0, 516, 1032, 1548, with the view at the right end (784) and
/// the third column focused.
fn four_columns_view_at_end() -> Vec<Op> {
    vec![
        Op::AddOutput(1),
        window(1, 500),
        window(2, 500),
        window(3, 500),
        window(4, 500),
        // Clear activate_prev_column_on_removal.
        Op::FocusColumnFirst,
        Op::FocusColumnLast,
        Op::FocusColumnLeft,
        Op::CompleteAnimations,
    ]
}

#[test]
fn close_rightmost_column() {
    let mut ops = four_columns_view_at_end();
    ops.extend([Op::FocusColumnLast, Op::CloseWindow(4)]);

    // The right snap is now at 1032 + 500 + 16 - 1280 = 268.
    assert_snapshot!(off_on(Options::default(), &ops), @"off: 784, on: 268");

    let mut layout = check_ops_with_options(make_options(true), ops);
    Op::CompleteAnimations.apply(&mut layout);
    assert_active_column_visible(&layout);

    // Same when the focused column was the left neighbor.
    let mut ops = four_columns_view_at_end();
    ops.push(Op::CloseWindow(4));
    assert_snapshot!(off_on(Options::default(), &ops), @"off: 784, on: 268");
}

#[test]
fn close_middle_column_with_view_at_end() {
    // Five 400 px wide columns at x = 0, 416, 832, 1248, 1664.
    let ops = [
        Op::AddOutput(1),
        window(1, 400),
        window(2, 400),
        window(3, 400),
        window(4, 400),
        window(5, 400),
        Op::FocusColumnFirst,
        Op::FocusColumnLast,
        Op::FocusColumnLeft,
        Op::FocusColumnLeft,
        Op::CompleteAnimations,
        Op::CloseWindow(4),
    ];

    // The right snap is now at 1248 + 400 + 16 - 1280 = 384.
    assert_snapshot!(off_on(Options::default(), &ops), @"off: 800, on: 384");

    let mut layout = check_ops_with_options(make_options(true), ops);
    Op::CompleteAnimations.apply(&mut layout);
    assert_active_column_visible(&layout);
}

#[test]
fn close_column_left_of_active() {
    let ops = [
        Op::AddOutput(1),
        window(1, 500),
        window(2, 500),
        window(3, 500),
        window(4, 500),
        Op::FocusColumnFirst,
        Op::FocusColumnRight,
        Op::CompleteAnimations,
        Op::CloseWindow(1),
    ];

    // The view follows the active column, which exposes empty space on the left.
    assert_snapshot!(off_on(Options::default(), &ops), @"off: -532, on: -16");
}

#[test]
fn remaining_columns_fit() {
    // Three 500 px wide columns at x = 0, 516, 1032, the view at the right end.
    let setup = [
        Op::AddOutput(1),
        window(1, 500),
        window(2, 500),
        window(3, 500),
        Op::FocusColumnFirst,
        Op::FocusColumnLast,
        Op::CompleteAnimations,
    ];

    // Close a column to the left of the active one.
    let mut ops = setup.to_vec();
    ops.push(Op::CloseWindow(2));
    assert_snapshot!(off_on(Options::default(), &ops), @"off: -248, on: -16");

    // Close the active rightmost column.
    let mut ops = setup.to_vec();
    ops.push(Op::CloseWindow(3));
    assert_snapshot!(off_on(Options::default(), &ops), @"off: 268, on: -16");
}

#[test]
fn centering_is_not_affected() {
    let ops = [
        Op::AddOutput(1),
        window(1, 500),
        window(2, 500),
        Op::FocusColumnFirst,
        Op::FocusColumnLast,
        Op::CompleteAnimations,
        Op::CloseWindow(1),
    ];

    let mut options = Options::default();
    options.layout.always_center_single_column = true;
    assert_snapshot!(off_on(options, &ops), @"off: -764, on: -764");

    let mut ops = four_columns_view_at_end();
    ops.push(Op::CloseWindow(4));

    let mut options = Options::default();
    options.layout.center_focused_column = CenterFocusedColumn::Always;
    assert_snapshot!(off_on(options, &ops), @"off: 642, on: 642");
}

#[test]
fn close_last_window() {
    let ops = [Op::AddOutput(1), window(1, 500), Op::CloseWindow(1)];
    assert_snapshot!(off_on(Options::default(), &ops), @"off: -16, on: -16");
}

#[test]
fn close_new_window_activates_prev_column() {
    // A new window opens to the right of the first column and is closed right away.
    let ops = [
        Op::AddOutput(1),
        window(1, 500),
        window(2, 500),
        window(3, 500),
        Op::FocusColumnFirst,
        Op::CompleteAnimations,
        window(4, 500),
        Op::CloseWindow(4),
    ];
    assert_snapshot!(off_on(Options::default(), &ops), @"off: -16, on: -16");

    // A new window opens at the right end and is closed right away.
    let ops = [
        Op::AddOutput(1),
        window(1, 500),
        window(2, 500),
        window(3, 500),
        Op::FocusColumnFirst,
        Op::FocusColumnLast,
        Op::CompleteAnimations,
        window(4, 500),
        Op::CloseWindow(4),
    ];
    assert_snapshot!(off_on(Options::default(), &ops), @"off: 268, on: 268");
}

#[test]
fn close_during_view_animation() {
    let mut ops = four_columns_view_at_end();
    ops.extend([
        Op::FocusColumnFirst,
        Op::CompleteAnimations,
        // Start animating towards the right end.
        Op::FocusColumnLast,
        Op::CloseWindow(4),
    ]);

    // The animation target is clamped.
    assert_snapshot!(off_on(Options::default(), &ops), @"off: 784, on: 268");

    // The view doesn't jump.
    let layout = check_ops_with_options(make_options(true), ops);
    let (_, _, ws) = layout.workspaces().next().unwrap();
    assert!(ws.scrolling().view_offset().is_animation_ongoing());
    assert_snapshot!(first_ws_view_pos(&layout), @"-16");
}

#[test]
fn close_during_view_gesture() {
    let mut ops = four_columns_view_at_end();
    ops.extend([
        Op::ViewOffsetGestureBegin {
            output_idx: 1,
            workspace_idx: None,
            is_touchpad: true,
        },
        Op::ViewOffsetGestureUpdate {
            delta: -100.,
            timestamp: Duration::from_millis(10),
            is_touchpad: true,
        },
        Op::CloseWindow(4),
        Op::ViewOffsetGestureUpdate {
            delta: -10.,
            timestamp: Duration::from_millis(20),
            is_touchpad: true,
        },
    ]);

    // The view doesn't move on its own during the gesture.
    assert_snapshot!(off_on(Options::default(), &ops), @"off: 666.6666666666667, on: 666.6666666666667");

    // Don't assert on the snapping result, just check that it doesn't panic.
    ops.push(Op::ViewOffsetGestureEnd {
        is_touchpad: Some(true),
    });
    off_on(Options::default(), &ops);
}

#[test]
fn interactive_move_does_not_fill_view() {
    let mut ops = four_columns_view_at_end();
    ops.extend([
        Op::InteractiveMoveBegin {
            window: 4,
            output_idx: 1,
            px: 0.,
            py: 0.,
        },
        Op::InteractiveMoveUpdate {
            window: 4,
            dx: 1000.,
            dy: 1000.,
            output_idx: 1,
            px: 0.,
            py: 0.,
        },
    ]);

    // The window has left the scrolling layout, but the view stays during the move.
    assert_snapshot!(off_on(Options::default(), &ops), @"off: 784, on: 784");

    let layout = check_ops_with_options(make_options(true), ops);
    let (_, _, ws) = layout.workspaces().next().unwrap();
    assert!(!ws.has_window(&4));
}

#[test]
fn fullscreen_or_maximized_active_column() {
    // Committed fullscreen.
    let mut ops = four_columns_view_at_end();
    ops.extend([
        Op::FullscreenWindow(3),
        Op::Communicate(3),
        Op::CompleteAnimations,
        Op::CloseWindow(4),
    ]);
    assert_snapshot!(off_on(Options::default(), &ops), @"off: 1032, on: 1032");

    // Requested fullscreen, not committed yet. The view only moves once the window commits.
    let mut ops = four_columns_view_at_end();
    ops.extend([Op::FullscreenWindow(3), Op::CloseWindow(4)]);
    assert_snapshot!(off_on(Options::default(), &ops), @"off: 784, on: 784");

    // Requested unfullscreen, not committed yet.
    let mut ops = four_columns_view_at_end();
    ops.extend([
        Op::FullscreenWindow(3),
        Op::Communicate(3),
        Op::CompleteAnimations,
        Op::FullscreenWindow(3),
        Op::CloseWindow(4),
    ]);
    assert_snapshot!(off_on(Options::default(), &ops), @"off: 1032, on: 1032");

    // Maximized.
    let mut ops = four_columns_view_at_end();
    ops.extend([
        Op::MaximizeWindowToEdges { id: Some(3) },
        Op::Communicate(3),
        Op::CompleteAnimations,
        Op::CloseWindow(4),
    ]);
    assert_snapshot!(off_on(Options::default(), &ops), @"off: 1032, on: 1032");
}

#[test]
fn floating_removal_does_not_fill_view() {
    let setup = [
        Op::AddOutput(1),
        floating_window(5, 300),
        window(1, 500),
        window(2, 500),
        window(3, 500),
        window(4, 500),
        Op::FocusColumnFirst,
        Op::FocusColumnLast,
        // Shrink the last column to expose empty space on the right (resizing doesn't fill the
        // view).
        Op::SetForcedSize {
            id: 4,
            size: Some(Size::new(200, 200)),
        },
        Op::Communicate(4),
        Op::CompleteAnimations,
    ];

    let mut ops = setup.to_vec();
    ops.push(Op::CloseWindow(5));
    assert_snapshot!(off_on(Options::default(), &ops), @"off: 784, on: 784");

    let mut ops = setup.to_vec();
    ops.push(Op::MoveWindowToWorkspace {
        window_id: Some(5),
        workspace_idx: 1,
    });
    assert_snapshot!(off_on(Options::default(), &ops), @"off: 784, on: 784");
}

#[test]
fn close_on_inactive_workspace() {
    let mut ops = four_columns_view_at_end();
    ops.extend([
        Op::FocusWorkspaceDown,
        Op::CompleteAnimations,
        Op::CloseWindow(4),
    ]);
    assert_snapshot!(off_on(Options::default(), &ops), @"off: 784, on: 268");

    // The view animates even on an inactive workspace.
    let layout = check_ops_with_options(make_options(true), ops);
    let (_, _, ws) = layout.workspaces().next().unwrap();
    assert!(ws.scrolling().view_offset().is_animation_ongoing());
}

#[test]
fn interactive_resize_does_not_fill_view() {
    let mut ops = four_columns_view_at_end();
    ops.extend([
        Op::InteractiveResizeBegin {
            window: 3,
            edges: ResizeEdge::RIGHT,
        },
        Op::CloseWindow(4),
    ]);
    assert_snapshot!(off_on(Options::default(), &ops), @"off: 784, on: 784");
}

#[test]
fn struts_and_gaps() {
    let options = |gaps| {
        let mut options = Options::default();
        options.layout.gaps = gaps;
        options.layout.struts = Struts {
            left: FloatOrInt(50.),
            right: FloatOrInt(30.),
            top: FloatOrInt(0.),
            bottom: FloatOrInt(0.),
        };
        options
    };

    // Right snap: 1032 + 500 + 16 + 30 - 1280 = 298.
    let mut ops = four_columns_view_at_end();
    ops.push(Op::CloseWindow(4));
    assert_snapshot!(off_on(options(16.), &ops), @"off: 814, on: 298");

    // Right snap: 1000 + 500 + 0 + 30 - 1280 = 250.
    assert_snapshot!(off_on(options(0.), &ops), @"off: 750, on: 250");

    // Left snap: 0 - 16 - 50 = -66.
    let ops = [
        Op::AddOutput(1),
        window(1, 500),
        window(2, 500),
        window(3, 500),
        window(4, 500),
        Op::FocusColumnFirst,
        Op::FocusColumnRight,
        Op::CompleteAnimations,
        Op::CloseWindow(1),
    ];
    assert_snapshot!(off_on(options(16.), &ops), @"off: -582, on: -66");

    // Left snap: 0 - 0 - 50 = -50.
    assert_snapshot!(off_on(options(0.), &ops), @"off: -550, on: -50");

    // A column wider than the working area: x = 0, 516, 1782, 2298.
    // Right snap: 1782 + 500 + 16 + 30 - 1280 = 1048.
    let ops = [
        Op::AddOutput(1),
        window(1, 500),
        window(2, 1250),
        window(3, 500),
        window(4, 500),
        Op::FocusColumnFirst,
        Op::FocusColumnLast,
        Op::FocusColumnLeft,
        Op::CompleteAnimations,
        Op::CloseWindow(4),
    ];
    assert_snapshot!(off_on(options(16.), &ops), @"off: 1564, on: 1048");
}

#[test]
fn moving_out_fills_source_view() {
    let mut setup = four_columns_view_at_end();
    setup.extend([Op::AddOutput(2), Op::FocusColumnLast]);

    let check = |op: Op| {
        let mut ops = setup.clone();
        ops.push(op);
        off_on(Options::default(), &ops)
    };

    assert_snapshot!(check(Op::ToggleWindowFloating { id: None }), @"off: 784, on: 268");
    assert_snapshot!(check(Op::SetWindowFloating { id: Some(4), floating: true }), @"off: 784, on: 268");
    assert_snapshot!(check(Op::MoveWindowToWorkspaceDown(false)), @"off: 784, on: 268");
    assert_snapshot!(check(Op::MoveWindowToWorkspaceDown(true)), @"off: 784, on: 268");
    assert_snapshot!(check(Op::MoveColumnToWorkspaceDown(false)), @"off: 784, on: 268");
    assert_snapshot!(check(Op::MoveColumnToWorkspaceDown(true)), @"off: 784, on: 268");
    assert_snapshot!(check(Op::MoveWindowToOutput {
        window_id: None,
        output_id: 2,
        target_ws_idx: None,
    }), @"off: 784, on: 268");
    assert_snapshot!(check(Op::MoveColumnToOutput {
        output_id: 2,
        target_ws_idx: None,
        activate: false,
    }), @"off: 784, on: 268");
}

#[test]
fn unfullscreen_to_floating_fills_view() {
    let mut ops = four_columns_view_at_end();
    ops.extend([
        Op::FocusColumnLast,
        floating_window(5, 300),
        // Fullscreen moves the floating window into the scrolling layout, to the right of the
        // last column.
        Op::FullscreenWindow(5),
        Op::Communicate(5),
        Op::CompleteAnimations,
        Op::FocusColumnLeft,
        Op::CompleteAnimations,
        // Unfullscreen moves it back to floating.
        Op::SetFullscreenWindow {
            window: 5,
            is_fullscreen: false,
        },
        Op::Communicate(5),
    ]);
    assert_snapshot!(off_on(Options::default(), &ops), @"off: 1532, on: 784");
}
