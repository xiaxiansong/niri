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
    off_on_with(options, ops, |options, fill| {
        options.layout.fill_view_after_removal = fill;
    })
}

/// Same as [`off_on()`], but for fill-view-after-resize.
fn resize_off_on(options: Options, ops: &[Op]) -> String {
    off_on_with(options, ops, |options, fill| {
        options.layout.fill_view_after_resize = fill;
    })
}

fn off_on_with(options: Options, ops: &[Op], set: fn(&mut Options, bool)) -> String {
    let run = |fill| {
        let mut options = options.clone();
        set(&mut options, fill);
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

/// Starts an interactive move of a window on output 1 and moves it past the threshold to the given
/// pointer position on the given output.
fn start_moving(window: usize, output_idx: usize, px: f64, py: f64) -> [Op; 2] {
    [
        Op::InteractiveMoveBegin {
            window,
            output_idx: 1,
            px: 0.,
            py: 0.,
        },
        Op::InteractiveMoveUpdate {
            window,
            dx: 1000.,
            dy: 1000.,
            output_idx,
            px,
            py,
        },
    ]
}

#[test]
fn interactive_move_fills_source_view_after_drop() {
    let mut setup = four_columns_view_at_end();
    setup.push(Op::AddOutput(2));

    // Drop onto the other output. The right snap is now at 1032 + 500 + 16 - 1280 = 268.
    let mut ops = setup.clone();
    ops.extend(start_moving(4, 2, 640., 360.));
    ops.push(Op::InteractiveMoveEnd { window: 4 });
    assert_snapshot!(off_on(Options::default(), &ops), @"off: 784, on: 268");

    let layout = check_ops_with_options(make_options(true), ops);
    let (_, _, ws) = layout.workspaces().next().unwrap();
    assert!(!ws.has_window(&4));
    assert!(layout.has_window(&4));

    // The window is closed while being moved.
    let mut ops = setup.clone();
    ops.extend(start_moving(4, 1, 640., 360.));
    ops.push(Op::CloseWindow(4));
    assert_snapshot!(off_on(Options::default(), &ops), @"off: 784, on: 268");
}

#[test]
fn interactive_move_back_to_same_workspace() {
    // Drop right back into the same spot, past the last column: x = 1000 + 784 + 8 > 1548.
    let mut ops = four_columns_view_at_end();
    ops.extend(start_moving(4, 1, 1000., 100.));
    ops.push(Op::InteractiveMoveEnd { window: 4 });
    assert_snapshot!(off_on(Options::default(), &ops), @"off: 784, on: 784");

    // The view doesn't move at all.
    let layout = check_ops_with_options(make_options(true), ops);
    let (_, _, ws) = layout.workspaces().next().unwrap();
    assert!(!ws.scrolling().view_offset().is_animation_ongoing());

    // Drop into the column of window 3 (x = 490 + 784 + 8 = 1282), which leaves three columns.
    // The right snap is now at 1032 + 500 + 16 - 1280 = 268.
    let mut ops = four_columns_view_at_end();
    ops.extend(start_moving(4, 1, 490., 100.));
    ops.push(Op::InteractiveMoveEnd { window: 4 });
    assert_snapshot!(off_on(Options::default(), &ops), @"off: 784, on: 268");

    // The view goes straight to the final position.
    let layout = check_ops_with_options(make_options(true), ops);
    let (_, _, ws) = layout.workspaces().next().unwrap();
    let scrolling = ws.scrolling();
    assert_eq!(scrolling.columns().count(), 3);
    assert!(scrolling.view_offset().is_animation_ongoing());
    assert_snapshot!(scrolling.target_view_pos(), @"268");
}

#[test]
fn interactive_move_of_floating_window() {
    // Same setup as in floating_removal_does_not_fill_view().
    let mut ops = vec![
        Op::AddOutput(1),
        Op::AddOutput(2),
        floating_window(5, 300),
        window(1, 500),
        window(2, 500),
        window(3, 500),
        window(4, 500),
        Op::FocusColumnFirst,
        Op::FocusColumnLast,
        Op::SetForcedSize {
            id: 4,
            size: Some(Size::new(200, 200)),
        },
        Op::Communicate(4),
        Op::CompleteAnimations,
    ];
    ops.extend(start_moving(5, 2, 640., 360.));
    ops.push(Op::InteractiveMoveEnd { window: 5 });
    assert_snapshot!(off_on(Options::default(), &ops), @"off: 784, on: 784");

    // A fullscreen window that unfullscreens to floating when the move starts did leave the
    // scrolling layout. Same setup as in unfullscreen_to_floating_fills_view().
    let mut ops = four_columns_view_at_end();
    ops.extend([
        Op::FocusColumnLast,
        floating_window(5, 300),
        Op::FullscreenWindow(5),
        Op::Communicate(5),
        Op::CompleteAnimations,
        Op::FocusColumnLeft,
        Op::CompleteAnimations,
    ]);
    ops.extend(start_moving(5, 1, 640., 360.));
    ops.push(Op::InteractiveMoveEnd { window: 5 });
    assert_snapshot!(off_on(Options::default(), &ops), @"off: 1532, on: 784");
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

#[test]
fn consume_into_column_fills_view() {
    let check = |ops: &[Op]| {
        let mut all = four_columns_view_at_end();
        all.extend_from_slice(ops);
        off_on(Options::default(), &all)
    };

    // Window 3 is active and the last column disappears. The right snap is now at
    // 1032 + 500 + 16 - 1280 = 268.
    assert_snapshot!(check(&[Op::ConsumeWindowIntoColumn]), @"off: 784, on: 268");
    assert_snapshot!(check(&[Op::ConsumeOrExpelWindowLeft { id: Some(4) }]), @"off: 784, on: 268");
    assert_snapshot!(check(&[Op::ConsumeOrExpelWindowRight { id: None }]), @"off: 784, on: 268");
    assert_snapshot!(
        check(&[Op::FocusColumnLast, Op::ConsumeOrExpelWindowLeft { id: None }]),
        @"off: 784, on: 268"
    );

    // Expelling adds a column at 1548, so the view has to show it either way:
    // 1548 + 500 + 16 - 1280 = 784.
    assert_snapshot!(
        check(&[
            Op::ConsumeOrExpelWindowLeft { id: Some(4) },
            Op::ConsumeOrExpelWindowRight { id: None },
        ]),
        @"off: 784, on: 784"
    );

    let mut ops = four_columns_view_at_end();
    ops.push(Op::ConsumeWindowIntoColumn);
    let mut layout = check_ops_with_options(make_options(true), ops);
    Op::CompleteAnimations.apply(&mut layout);
    assert_active_column_visible(&layout);
}

fn set_width(id: usize, width: i32) -> [Op; 2] {
    [
        Op::SetWindowWidth {
            id: Some(id),
            change: SizeChange::SetFixed(width),
        },
        Op::Communicate(id),
    ]
}

#[test]
fn shrink_rightmost_column() {
    let mut ops = four_columns_view_at_end();
    ops.extend([
        Op::FocusColumnLast,
        Op::CompleteAnimations,
        Op::SetWindowWidth {
            id: None,
            change: SizeChange::SetFixed(200),
        },
    ]);

    // The view doesn't move until the window commits the new size.
    assert_snapshot!(resize_off_on(Options::default(), &ops), @"off: 784, on: 784");

    // The right snap is now at 1548 + 200 + 16 - 1280 = 484.
    ops.push(Op::Communicate(4));
    assert_snapshot!(resize_off_on(Options::default(), &ops), @"off: 784, on: 484");

    let mut options = Options::default();
    options.layout.fill_view_after_resize = true;
    let mut layout = check_ops_with_options(options, ops);
    Op::CompleteAnimations.apply(&mut layout);
    assert_active_column_visible(&layout);

    // Same when the focused column is the left neighbor.
    let mut ops = four_columns_view_at_end();
    ops.extend(set_width(4, 200));
    assert_snapshot!(resize_off_on(Options::default(), &ops), @"off: 784, on: 484");
}

#[test]
fn shrink_column_left_of_active() {
    let mut ops = vec![
        Op::AddOutput(1),
        window(1, 500),
        window(2, 500),
        window(3, 500),
        window(4, 500),
        Op::FocusColumnFirst,
        Op::FocusColumnRight,
        Op::CompleteAnimations,
    ];
    ops.extend(set_width(1, 200));

    // The view follows the active column, which exposes empty space on the left.
    assert_snapshot!(resize_off_on(Options::default(), &ops), @"off: -316, on: -16");
}

#[test]
fn shrink_until_columns_fit() {
    // Three 500 px wide columns at x = 0, 516, 1032, the view at the right end.
    let mut ops = vec![
        Op::AddOutput(1),
        window(1, 500),
        window(2, 500),
        window(3, 500),
        Op::FocusColumnFirst,
        Op::FocusColumnLast,
        Op::CompleteAnimations,
    ];
    ops.extend(set_width(3, 200));
    assert_snapshot!(resize_off_on(Options::default(), &ops), @"off: 268, on: -16");
}

#[test]
fn grow_column_does_not_move_view() {
    let mut ops = four_columns_view_at_end();
    ops.extend(set_width(2, 700));
    assert_snapshot!(resize_off_on(Options::default(), &ops), @"off: 984, on: 984");
}

#[test]
fn interactive_resize_fills_view_at_end() {
    let mut ops = four_columns_view_at_end();
    ops.extend([
        Op::FocusColumnLast,
        Op::CompleteAnimations,
        Op::InteractiveResizeBegin {
            window: 4,
            edges: ResizeEdge::RIGHT,
        },
        Op::InteractiveResizeUpdate {
            window: 4,
            dx: -300.,
            dy: 0.,
        },
        Op::Communicate(4),
    ]);

    // The view stays in place during the resize.
    assert_snapshot!(resize_off_on(Options::default(), &ops), @"off: 784, on: 784");

    ops.push(Op::InteractiveResizeEnd { window: 4 });
    assert_snapshot!(resize_off_on(Options::default(), &ops), @"off: 784, on: 484");
}

#[test]
fn resize_with_centering_is_not_affected() {
    let mut ops = four_columns_view_at_end();
    ops.extend(set_width(4, 200));

    let mut options = Options::default();
    options.layout.center_focused_column = CenterFocusedColumn::Always;
    assert_snapshot!(resize_off_on(options, &ops), @"off: 642, on: 642");
}

#[test]
fn unfullscreen_keeps_view() {
    let mut ops = four_columns_view_at_end();
    ops.extend([
        Op::FocusColumnLast,
        Op::CompleteAnimations,
        Op::FullscreenWindow(4),
        Op::Communicate(4),
        Op::CompleteAnimations,
    ]);
    assert_snapshot!(resize_off_on(Options::default(), &ops), @"off: 1548, on: 1548");

    ops.extend([Op::FullscreenWindow(4), Op::Communicate(4)]);
    assert_snapshot!(resize_off_on(Options::default(), &ops), @"off: 784, on: 784");
}

#[test]
fn removal_flag_does_not_fill_view_after_resize() {
    let mut ops = four_columns_view_at_end();
    ops.extend(set_width(4, 200));
    assert_snapshot!(off_on(Options::default(), &ops), @"off: 784, on: 784");
}

/// Screen X and width of each tile on the first workspace, after committing and completing the
/// animations, for resize-neighbor-to-fill off and on.
fn neighbor_off_on(options: Options, ops: &[Op]) -> String {
    let run = |fill| {
        let mut options = options.clone();
        options.layout.resize_neighbor_to_fill = fill;
        let mut layout = check_ops_with_options(options, ops.iter().cloned());
        for id in 1..=5 {
            Op::Communicate(id).apply(&mut layout);
        }
        Op::CompleteAnimations.apply(&mut layout);

        let (_, _, ws) = layout.workspaces().next().unwrap();
        let tiles: Vec<_> = ws
            .tiles_with_render_positions()
            .map(|(tile, pos, _)| format!("{}+{}", pos.x, tile.tile_size().w))
            .collect();
        tiles.join(" ")
    };

    format!("off: {}\non: {}", run(false), run(true))
}

fn proportion(percent: f64) -> Op {
    Op::SetColumnWidth(SizeChange::SetProportion(percent))
}

fn adjust(percent: f64) -> Op {
    Op::SetColumnWidth(SizeChange::AdjustProportion(percent))
}

/// Two half-width columns filling the 1280 px wide view, the right one focused.
fn two_halves() -> Vec<Op> {
    vec![
        Op::AddOutput(1),
        window(1, 500),
        proportion(50.),
        window(2, 500),
        proportion(50.),
        Op::Communicate(1),
        Op::Communicate(2),
        // Move the view to the committed widths.
        Op::FocusColumnFirst,
        Op::FocusColumnLast,
        Op::CompleteAnimations,
    ]
}

#[test]
fn neighbor_two_halves_setup() {
    // 616 px wide columns with 16 px gaps fill 1280 px exactly.
    assert_snapshot!(neighbor_off_on(Options::default(), &two_halves()), @"
    off: 648+616 16+616
    on: 648+616 16+616
    ");
}

#[test]
fn neighbor_grow_right_column() {
    let mut ops = two_halves();
    ops.push(adjust(10.));
    assert_snapshot!(neighbor_off_on(Options::default(), &ops), @"
    off: 522+742 -110+616
    on: 522+742 17+489
    ");
}

#[test]
fn neighbor_shrink_right_column() {
    let mut ops = two_halves();
    ops.push(adjust(-10.));
    assert_snapshot!(neighbor_off_on(Options::default(), &ops), @"
    off: 648+489 16+616
    on: 774+489 16+742
    ");
}

#[test]
fn neighbor_grow_left_column() {
    let mut ops = two_halves();
    ops.extend([Op::FocusColumnLeft, Op::CompleteAnimations, adjust(10.)]);
    assert_snapshot!(neighbor_off_on(Options::default(), &ops), @"
    off: 16+742 774+616
    on: 16+742 774+489
    ");
}

#[test]
fn neighbor_repeat_before_commit() {
    // Pressing the key twice before the windows commit still keeps the view filled.
    let mut ops = two_halves();
    ops.extend([adjust(10.), adjust(10.)]);
    assert_snapshot!(neighbor_off_on(Options::default(), &ops), @"
    off: 396+868 -236+616
    on: 395+868 16+363
    ");
}

#[test]
fn neighbor_switch_preset() {
    let mut options = Options::default();
    options.layout.preset_column_widths = vec![
        PresetSize::Proportion(0.25),
        PresetSize::Proportion(0.5),
        PresetSize::Proportion(0.75),
    ];

    // From one half, the next preset is three quarters, and the neighbor gets one quarter.
    let mut ops = two_halves();
    ops.push(Op::SwitchPresetColumnWidth);
    assert_snapshot!(neighbor_off_on(options.clone(), &ops), @"
    off: 332+932 -300+616
    on: 332+932 16+300
    ");

    // Then one quarter, and the neighbor gets three quarters.
    ops.extend([
        Op::Communicate(1),
        Op::Communicate(2),
        Op::SwitchPresetColumnWidth,
    ]);
    assert_snapshot!(neighbor_off_on(options, &ops), @"
    off: 332+300 -300+616
    on: 964+300 16+932
    ");
}

#[test]
fn neighbor_by_window_id() {
    // Resizing an unfocused column by id works too.
    let mut ops = two_halves();
    ops.push(Op::SetWindowWidth {
        id: Some(1),
        change: SizeChange::AdjustProportion(10.),
    });
    assert_snapshot!(neighbor_off_on(Options::default(), &ops), @"
    off: 648+616 -110+742
    on: 774+489 16+742
    ");
}

#[test]
fn neighbor_three_columns() {
    // Quarter, quarter, half; the middle column grows and the right one shrinks.
    let ops = vec![
        Op::AddOutput(1),
        window(1, 500),
        proportion(25.),
        window(2, 500),
        proportion(25.),
        window(3, 500),
        proportion(50.),
        Op::Communicate(1),
        Op::Communicate(2),
        Op::Communicate(3),
        Op::FocusColumnFirst,
        Op::FocusColumnRight,
        Op::CompleteAnimations,
        adjust(10.),
    ];
    assert_snapshot!(neighbor_off_on(Options::default(), &ops), @"
    off: 332+426 16+300 774+616
    on: 332+426 16+300 774+489
    ");
}

#[test]
fn neighbor_with_columns_off_screen() {
    // Columns off-screen to the left stay where they are.
    let mut ops = vec![
        Op::AddOutput(1),
        window(1, 500),
        window(2, 500),
        proportion(50.),
        window(3, 500),
        proportion(50.),
        Op::Communicate(1),
        Op::Communicate(2),
        Op::Communicate(3),
        Op::FocusColumnFirst,
        Op::FocusColumnLast,
        Op::CompleteAnimations,
    ];
    assert_snapshot!(neighbor_off_on(Options::default(), &ops), @"
    off: 648+616 -500+500 16+616
    on: 648+616 -500+500 16+616
    ");

    ops.push(adjust(10.));
    assert_snapshot!(neighbor_off_on(Options::default(), &ops), @"
    off: 522+742 -626+500 -110+616
    on: 522+742 -499+500 17+489
    ");
}

#[test]
fn neighbor_not_filling_view() {
    // Half and a quarter don't fill the view, so only the column changes.
    let ops = vec![
        Op::AddOutput(1),
        window(1, 500),
        proportion(50.),
        window(2, 500),
        proportion(25.),
        Op::Communicate(1),
        Op::Communicate(2),
        // Move the view to the committed widths.
        Op::FocusColumnFirst,
        Op::FocusColumnLast,
        Op::CompleteAnimations,
        adjust(10.),
    ];
    assert_snapshot!(neighbor_off_on(Options::default(), &ops), @"
    off: 648+426 16+616
    on: 648+426 16+616
    ");
}

#[test]
fn neighbor_too_narrow() {
    // The left window can't be narrower than 600 px, so the right column resizes like usual.
    let mut params = TestWindowParams::new(1);
    params.bbox = Rectangle::from_size(Size::from((500, 200)));
    params.min_max_size = (Size::from((600, 0)), Size::from((0, 0)));
    let mut ops = vec![
        Op::AddOutput(1),
        Op::AddWindow { params },
        proportion(50.),
        window(2, 500),
        proportion(50.),
        Op::Communicate(1),
        Op::Communicate(2),
        // Move the view to the committed widths.
        Op::FocusColumnFirst,
        Op::FocusColumnLast,
        Op::CompleteAnimations,
        adjust(10.),
    ];
    assert_snapshot!(neighbor_off_on(Options::default(), &ops), @"
    off: 522+742 -110+616
    on: 522+742 -110+616
    ");

    // Shrinking still works.
    ops.pop();
    ops.push(adjust(-10.));
    assert_snapshot!(neighbor_off_on(Options::default(), &ops), @"
    off: 648+489 16+616
    on: 774+489 16+742
    ");
}

#[test]
fn neighbor_with_centering_is_not_affected() {
    let mut ops = two_halves();
    ops.push(adjust(10.));

    let mut options = Options::default();
    options.layout.center_focused_column = CenterFocusedColumn::Always;
    assert_snapshot!(neighbor_off_on(options, &ops), @"
    off: 269+742 -363+616
    on: 269+742 -363+616
    ");
}

/// Drags the edges of the window by `dx`, and lets both windows commit.
fn drag(window: usize, edges: ResizeEdge, dx: f64) -> Vec<Op> {
    vec![
        Op::InteractiveResizeBegin { window, edges },
        Op::InteractiveResizeUpdate { window, dx, dy: 0. },
        Op::Refresh { is_active: true },
        Op::Communicate(1),
        Op::Communicate(2),
        Op::Communicate(3),
    ]
}

/// Lets go of the window.
fn release(window: usize) -> [Op; 2] {
    [
        Op::InteractiveResizeEnd { window },
        Op::Refresh { is_active: true },
    ]
}

#[test]
fn neighbor_drag_right_edge() {
    // The right neighbor gets narrower, and the left column stays in place.
    let mut ops = two_halves();
    ops.extend([Op::FocusColumnLeft, Op::CompleteAnimations]);
    ops.extend(drag(1, ResizeEdge::RIGHT, 100.));
    assert_snapshot!(neighbor_off_on(Options::default(), &ops), @"
    off: 16+716 748+616
    on: 16+716 748+516
    ");

    // Same after letting go.
    ops.extend(release(1));
    assert_snapshot!(neighbor_off_on(Options::default(), &ops), @"
    off: 16+716 748+616
    on: 16+716 748+516
    ");
}

#[test]
fn neighbor_drag_left_edge() {
    // The left neighbor gets narrower, and the view stays in place.
    let mut ops = two_halves();
    ops.extend(drag(2, ResizeEdge::LEFT, -100.));
    assert_snapshot!(neighbor_off_on(Options::default(), &ops), @"
    off: 548+716 -84+616
    on: 548+716 16+516
    ");

    // Same after letting go.
    ops.extend(release(2));
    assert_snapshot!(neighbor_off_on(Options::default(), &ops), @"
    off: 548+716 -84+616
    on: 548+716 16+516
    ");
}

#[test]
fn neighbor_drag_left_edge_of_unfocused_column() {
    // The neighbor is the focused column, and the view stays in place.
    let mut ops = two_halves();
    ops.extend([Op::FocusColumnLeft, Op::CompleteAnimations]);
    ops.extend(drag(2, ResizeEdge::LEFT, 100.));
    ops.extend(release(2));
    assert_snapshot!(neighbor_off_on(Options::default(), &ops), @"
    off: 16+616 648+516
    on: 16+716 748+516
    ");
}

#[test]
fn neighbor_drag_in_steps_before_commit() {
    // The drag distance counts from the start of the resize, also before the windows commit.
    let mut ops = two_halves();
    ops.extend([
        Op::InteractiveResizeBegin {
            window: 2,
            edges: ResizeEdge::LEFT,
        },
        Op::InteractiveResizeUpdate {
            window: 2,
            dx: -50.,
            dy: 0.,
        },
        Op::InteractiveResizeUpdate {
            window: 2,
            dx: -100.,
            dy: 0.,
        },
        Op::Refresh { is_active: true },
        Op::Communicate(1),
        Op::Communicate(2),
        Op::InteractiveResizeUpdate {
            window: 2,
            dx: -150.,
            dy: 0.,
        },
    ]);
    assert_snapshot!(neighbor_off_on(Options::default(), &ops), @"
    off: 498+766 -134+616
    on: 498+766 16+466
    ");
}

#[test]
fn neighbor_drag_stops_at_min_width() {
    // The left window can't be narrower than 600 px, so the drag stops there.
    let mut params = TestWindowParams::new(1);
    params.bbox = Rectangle::from_size(Size::from((500, 200)));
    params.min_max_size = (Size::from((600, 0)), Size::from((0, 0)));
    let mut ops = vec![
        Op::AddOutput(1),
        Op::AddWindow { params },
        proportion(50.),
        window(2, 500),
        proportion(50.),
        Op::Communicate(1),
        Op::Communicate(2),
        // Move the view to the committed widths.
        Op::FocusColumnFirst,
        Op::FocusColumnLast,
        Op::CompleteAnimations,
    ];
    ops.extend(drag(2, ResizeEdge::LEFT, -100.));
    assert_snapshot!(neighbor_off_on(Options::default(), &ops), @"
    off: 548+716 -84+616
    on: 632+632 16+600
    ");
}

#[test]
fn neighbor_drag_outer_edge() {
    // There's no visible neighbor past the last column, so it resizes like usual.
    let mut ops = two_halves();
    ops.extend(drag(2, ResizeEdge::RIGHT, -100.));
    assert_snapshot!(neighbor_off_on(Options::default(), &ops), @"
    off: 648+516 16+616
    on: 648+516 16+616
    ");
}

#[test]
fn neighbor_drag_when_not_filling() {
    // The columns don't fill the view, so the drag resizes like usual.
    let ops = vec![
        Op::AddOutput(1),
        window(1, 500),
        proportion(25.),
        window(2, 500),
        proportion(25.),
        Op::Communicate(1),
        Op::Communicate(2),
        Op::FocusColumnFirst,
        Op::CompleteAnimations,
    ];
    let mut ops = ops;
    ops.extend(drag(1, ResizeEdge::RIGHT, 100.));
    assert_snapshot!(neighbor_off_on(Options::default(), &ops), @"
    off: 216+400 632+300
    on: 216+400 632+300
    ");
}

#[test]
fn neighbor_drag_cancelled_by_neighbor_resize() {
    // Resizing the neighbor with an action stops the interactive resize, so the last update does
    // nothing.
    let mut ops = two_halves();
    ops.extend(drag(2, ResizeEdge::LEFT, -100.));
    ops.extend([
        Op::SetWindowWidth {
            id: Some(1),
            change: SizeChange::SetProportion(50.),
        },
        Op::InteractiveResizeUpdate {
            window: 2,
            dx: -200.,
            dy: 0.,
        },
    ]);
    assert_snapshot!(neighbor_off_on(Options::default(), &ops), @"
    off: 448+816 -184+616
    on: 548+716 -84+616
    ");
}
