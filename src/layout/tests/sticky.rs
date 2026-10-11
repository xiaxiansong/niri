use insta::assert_snapshot;

use super::scratchpad::{
    floating_window, hide, hit, linear_switch_options, state, summary, toggle, window, workspaces,
};
use super::*;

fn sticky(id: Option<usize>) -> Op {
    Op::ToggleWindowSticky { id }
}

/// Sticky windows, and whether each window is floating.
fn sticky_summary(layout: &Layout<TestWindow>) -> String {
    let mut sticky: Vec<_> = layout
        .windows()
        .map(|(_, win)| win)
        .filter(|win| win.is_sticky())
        .map(|win| win.0.id)
        .collect();
    sticky.sort();
    let mut floating: Vec<_> = layout
        .workspaces()
        .flat_map(|(_, _, ws)| ws.floating().tiles().map(|tile| tile.window().0.id))
        .collect();
    floating.sort();
    format!("sticky: {sticky:?}, floating: {floating:?}")
}

#[test]
fn tiled_window_becomes_floating_and_sticky() {
    let mut layout = check_ops([
        Op::AddOutput(1),
        window(1, 100),
        window(2, 100),
        sticky(None),
    ]);
    assert_snapshot!(sticky_summary(&layout), @"sticky: [2], floating: [2]");

    // Toggling again keeps the window floating.
    check_ops_on_layout(&mut layout, [sticky(None)]);
    assert_snapshot!(sticky_summary(&layout), @"sticky: [], floating: [2]");
}

#[test]
fn sticky_window_follows_workspace_switch() {
    let mut layout = check_ops([
        Op::AddOutput(1),
        window(1, 100),
        floating_window(2),
        sticky(Some(2)),
        Op::FocusWorkspaceDown,
    ]);
    assert_snapshot!(workspaces(&layout), @"output1: [1] *[2] []");
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [], focus: Some(2)");

    // Without the focus, it follows too.
    check_ops_on_layout(&mut layout, [window(3, 100), Op::FocusWorkspaceUp]);
    assert_snapshot!(workspaces(&layout), @"output1: *[1, 2] [3] []");
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [], focus: Some(1)");

    // Not sticky anymore, it stays.
    check_ops_on_layout(&mut layout, [sticky(Some(2)), Op::FocusWorkspaceDown]);
    assert_snapshot!(workspaces(&layout), @"output1: [1, 2] *[3] []");
}

#[test]
fn window_made_sticky_elsewhere_is_brought_over() {
    let layout = check_ops([
        Op::AddOutput(1),
        floating_window(1),
        Op::FocusWorkspaceDown,
        window(2, 100),
        sticky(Some(1)),
    ]);
    assert_snapshot!(workspaces(&layout), @"output1: [] *[2, 1] []");
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [], focus: Some(2)");
}

#[test]
fn leaving_floating_or_workspace_unsticks() {
    let setup = [
        Op::AddOutput(1),
        window(1, 100),
        floating_window(2),
        sticky(Some(2)),
    ];

    let mut ops = setup.to_vec();
    ops.push(Op::ToggleWindowFloating { id: Some(2) });
    assert_snapshot!(sticky_summary(&check_ops(ops)), @"sticky: [], floating: []");

    let mut ops = setup.to_vec();
    ops.extend([Op::FocusWindow(2), Op::MoveWindowToWorkspaceDown(false)]);
    let layout = check_ops(ops);
    assert_snapshot!(sticky_summary(&layout), @"sticky: [], floating: [2]");
    assert_snapshot!(workspaces(&layout), @"output1: *[1] [2] []");

    let mut ops = setup.to_vec();
    ops.extend([Op::FocusWindow(2), Op::FullscreenWindow(2)]);
    assert_snapshot!(sticky_summary(&check_ops(ops)), @"sticky: [], floating: []");
}

#[test]
fn scratchpad_and_sticky_exclude_each_other() {
    // Hiding a sticky window in the scratchpad unsticks it.
    let mut layout = check_ops([
        Op::AddOutput(1),
        window(1, 100),
        floating_window(2),
        sticky(Some(2)),
        hide(2),
    ]);
    assert_snapshot!(sticky_summary(&layout), @"sticky: [], floating: []");
    assert_snapshot!(summary(&layout), @"hidden: [2], shown: [], focus: Some(1)");

    // Hidden windows can't be sticky.
    check_ops_on_layout(&mut layout, [sticky(Some(2))]);
    assert_snapshot!(sticky_summary(&layout), @"sticky: [], floating: []");
    assert_snapshot!(summary(&layout), @"hidden: [2], shown: [], focus: Some(1)");

    // A shown scratchpad window becomes a regular sticky window.
    check_ops_on_layout(&mut layout, [toggle(), sticky(Some(2))]);
    assert_snapshot!(sticky_summary(&layout), @"sticky: [2], floating: [2]");
    assert_snapshot!(format!("{:?}", state(&layout, 2)), @"None");
}

#[test]
fn interactive_move_keeps_sticky() {
    let setup = [
        Op::AddOutput(1),
        window(1, 100),
        floating_window(2),
        sticky(Some(2)),
        Op::InteractiveMoveBegin {
            window: 2,
            output_idx: 1,
            px: 0.,
            py: 0.,
        },
        Op::InteractiveMoveUpdate {
            window: 2,
            dx: 100.,
            dy: 100.,
            output_idx: 1,
            px: 640.,
            py: 360.,
        },
    ];

    let mut ops = setup.to_vec();
    ops.push(Op::InteractiveMoveEnd { window: 2 });
    assert_snapshot!(sticky_summary(&check_ops(ops)), @"sticky: [2], floating: [2]");

    // Dropped into the scrolling layout.
    let mut ops = setup.to_vec();
    ops.extend([
        Op::ToggleWindowFloating { id: None },
        Op::InteractiveMoveEnd { window: 2 },
    ]);
    assert_snapshot!(sticky_summary(&check_ops(ops)), @"sticky: [], floating: []");
}

#[test]
fn sticky_window_is_pinned_during_workspace_switch() {
    let mut layout = check_ops_with_options(
        linear_switch_options(),
        [
            Op::AddOutput(1),
            window(1, 100),
            floating_window(2),
            Op::Communicate(2),
            Op::MoveFloatingWindow {
                id: Some(2),
                x: PositionChange::SetFixed(100.),
                y: PositionChange::SetFixed(50.),
                animate: false,
            },
            sticky(Some(2)),
            Op::CompleteAnimations,
        ],
    );
    assert_snapshot!(hit(&layout, 150., 150.), @"2 activate");

    check_ops_on_layout(
        &mut layout,
        [
            Op::FocusWorkspaceDown,
            Op::AdvanceAnimations { msec_delta: 500 },
        ],
    );
    assert_snapshot!(workspaces(&layout), @"output1: [1] *[2] []");
    assert_snapshot!(hit(&layout, 150., 150.), @"2 activate");
}
