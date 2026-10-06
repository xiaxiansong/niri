use std::fmt::Write as _;

use insta::assert_snapshot;
use niri_config::animations::{Curve, EasingParams, Kind};

use super::*;

fn format_tiles(layout: &Layout<TestWindow>) -> String {
    let mut buf = String::new();
    let ws = layout.active_workspace().unwrap();
    let mut tiles: Vec<_> = ws.tiles_with_render_positions().collect();

    // We sort by id since that gives us a consistent order (from first opened to last), but we
    // don't print the id since it's nondeterministic (the id is a global counter across all
    // running tests in the same binary).
    tiles.sort_by_key(|(tile, _, _)| tile.window().id());
    for (tile, pos, _visible) in tiles {
        let Size { w, h, .. } = tile.animated_tile_size();
        let Point { x, y, .. } = pos;
        writeln!(&mut buf, "{w:>3.0} × {h:>3.0} at x:{x:>3.0} y:{y:>3.0}").unwrap();
    }
    buf
}

fn make_options() -> Options {
    const LINEAR: Kind = Kind::Easing(EasingParams {
        duration_ms: 1000,
        curve: Curve::Linear,
    });

    let mut options = Options {
        layout: niri_config::Layout {
            gaps: 0.0,
            ..Default::default()
        },
        ..Options::default()
    };
    options.animations.window_resize.anim.kind = LINEAR;
    options.animations.window_movement.0.kind = LINEAR;

    options
}

fn set_up_two_in_column() -> Layout<TestWindow> {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::FocusColumnLeft,
        Op::ConsumeWindowIntoColumn,
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        Op::SetForcedSize {
            id: 2,
            size: Some(Size::new(200, 200)),
        },
        Op::Communicate(1),
        Op::Communicate(2),
        Op::CompleteAnimations,
    ];

    check_ops_with_options(make_options(), ops)
}

#[test]
fn height_resize_animates_next_y() {
    let mut layout = set_up_two_in_column();

    let ops = [
        // Issue a resize.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::AdjustFixed(-50),
        },
        // The top window shrinks in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 50)),
        },
        Op::Communicate(1),
        Op::Communicate(2),
    ];
    check_ops_on_layout(&mut layout, ops);

    // No time had passed yet, so we're at the initial state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y:100
    ");

    // Advance the time halfway.
    Op::AdvanceAnimations { msec_delta: 500 }.apply(&mut layout);

    // Top window is half-resized at 75 px tall, bottom window is at y=75 matching it.
    assert_snapshot!(format_tiles(&layout), @r"
    100 ×  75 at x:  0 y:  0
    200 × 200 at x:  0 y: 75
    ");

    // Advance the time to completion.
    Op::AdvanceAnimations { msec_delta: 500 }.apply(&mut layout);

    // Final state at 50 px.
    assert_snapshot!(format_tiles(&layout), @r"
    100 ×  50 at x:  0 y:  0
    200 × 200 at x:  0 y: 50
    ");
}

#[test]
fn clientside_height_change_doesnt_animate() {
    let mut layout = set_up_two_in_column();

    // The initial state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y:100
    ");

    let ops = [
        // The top window shrinks by itself, without a niri-issued resize.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 50)),
        },
        // This does not start any animations.
        Op::Communicate(1),
        Op::Communicate(2),
    ];
    check_ops_on_layout(&mut layout, ops);

    // No time had passed yet, but we are at the final state right away.
    assert_snapshot!(format_tiles(&layout), @r"
    100 ×  50 at x:  0 y:  0
    200 × 200 at x:  0 y: 50
    ");
}

#[test]
fn height_resize_and_back() {
    let mut layout = set_up_two_in_column();

    // The initial state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y:100
    ");

    let ops = [
        // Issue a resize.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(200),
        },
        // The top window grows in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 200)),
        },
        // This starts the resize animation.
        Op::Communicate(1),
        Op::Communicate(2),
        // Advance the time halfway.
        Op::AdvanceAnimations { msec_delta: 500 },
    ];
    check_ops_on_layout(&mut layout, ops);

    // Top window is half-resized at 150 px tall, bottom window is at y=150 matching it.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 150 at x:  0 y:  0
    200 × 200 at x:  0 y:150
    ");

    let ops = [
        // Issue a resize back.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(100),
        },
        // The top window shrinks in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        // This starts a new resize animation.
        Op::Communicate(1),
        Op::Communicate(2),
    ];
    check_ops_on_layout(&mut layout, ops);

    // No time had passed yet, and we expect no animation jumps, so this state matches the last.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 150 at x:  0 y:  0
    200 × 200 at x:  0 y:150
    ");

    // Advance the time halfway.
    Op::AdvanceAnimations { msec_delta: 500 }.apply(&mut layout);

    // Halfway through at 125px.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 125 at x:  0 y:  0
    200 × 200 at x:  0 y:125
    ");

    // Advance the time to completion.
    Op::AdvanceAnimations { msec_delta: 500 }.apply(&mut layout);

    // Final state back at 100px.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y:100
    ");
}

#[test]
fn height_resize_and_cancel() {
    let mut layout = set_up_two_in_column();

    // The initial state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y:100
    ");

    let ops = [
        // Issue a resize.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(200),
        },
        // The top window grows in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 200)),
        },
        // This starts the resize animation.
        Op::Communicate(1),
        Op::Communicate(2),
        // Advance the time slightly.
        Op::AdvanceAnimations { msec_delta: 50 },
    ];
    check_ops_on_layout(&mut layout, ops);

    // Top window is half-resized at 105 px tall, bottom window is at y=105 matching it.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 105 at x:  0 y:  0
    200 × 200 at x:  0 y:105
    ");

    let ops = [
        // Issue a resize back.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(100),
        },
        // The top window shrinks in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        // This cancels the resize animation since the change of 5 px is less than the resize
        // animation threshold.
        Op::Communicate(1),
        Op::Communicate(2),
    ];
    check_ops_on_layout(&mut layout, ops);

    // Since the resize animation is cancelled, the height goes to the new value immediately. The Y
    // position doesn't jump, instead the animation is offset to preserve the current position.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y:105
    ");

    // Advance to the end of the move animation.
    Op::AdvanceAnimations { msec_delta: 950 }.apply(&mut layout);

    // Final state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y:100
    ");
}

#[test]
fn height_resize_and_back_during_another_y_anim() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::FocusColumnLeft,
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        Op::SetForcedSize {
            id: 2,
            size: Some(Size::new(200, 200)),
        },
        Op::Communicate(1),
        Op::Communicate(2),
        Op::CompleteAnimations,
    ];
    let mut layout = check_ops_with_options(make_options(), ops);

    // The initial state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");

    // Consume second window into column, starting the X/Y move anim down.
    Op::ConsumeWindowIntoColumn.apply(&mut layout);

    // No time had passed, so no change in coordinates yet.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");

    // Advance the time halfway.
    Op::AdvanceAnimations { msec_delta: 500 }.apply(&mut layout);

    // Second window halfway to the bottom.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x: 50 y: 50
    ");

    let ops = [
        // Issue a resize.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(200),
        },
        // The top window grows in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 200)),
        },
        // This starts the resize animation.
        Op::Communicate(1),
        Op::Communicate(2),
    ];
    check_ops_on_layout(&mut layout, ops);

    // No time had passed, so no change in state yet.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x: 50 y: 50
    ");

    // Advance the time a bit.
    Op::AdvanceAnimations { msec_delta: 200 }.apply(&mut layout);

    // X changed by 20, but y changed by 30 since the Y movement from the resize compounds with the
    // Y movement from consume-into-column.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 120 at x:  0 y:  0
    200 × 200 at x: 30 y: 80
    ");

    let ops = [
        // Issue a resize back.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(100),
        },
        // The top window shrinks in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        // This starts the resize animation.
        Op::Communicate(1),
        Op::Communicate(2),
    ];
    check_ops_on_layout(&mut layout, ops);

    // No time had passed, so no change in state yet.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 120 at x:  0 y:  0
    200 × 200 at x: 30 y: 80
    ");

    // Advance the time a bit. Both resize and consume movement are still ongoing.
    Op::AdvanceAnimations { msec_delta: 200 }.apply(&mut layout);

    assert_snapshot!(format_tiles(&layout), @r"
    100 × 116 at x:  0 y:  0
    200 × 200 at x: 10 y: 84
    ");

    // Advance the time to complete the consume movement.
    Op::AdvanceAnimations { msec_delta: 100 }.apply(&mut layout);

    // The Y position is still lower than the height since the window started the resize-induced Y
    // movement high up.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 114 at x:  0 y:  0
    200 × 200 at x:  0 y: 86
    ");

    // Advance the time to complete the resize.
    Op::AdvanceAnimations { msec_delta: 700 }.apply(&mut layout);

    // Final state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y:100
    ");
}

#[test]
fn height_resize_and_cancel_during_another_y_anim() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::FocusColumnLeft,
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        Op::SetForcedSize {
            id: 2,
            size: Some(Size::new(200, 200)),
        },
        Op::Communicate(1),
        Op::Communicate(2),
        Op::CompleteAnimations,
    ];
    let mut layout = check_ops_with_options(make_options(), ops);

    // The initial state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");

    // Consume second window into column, starting the X/Y move anim down.
    Op::ConsumeWindowIntoColumn.apply(&mut layout);

    // No time had passed, so no change in coordinates yet.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");

    // Advance the time halfway.
    Op::AdvanceAnimations { msec_delta: 500 }.apply(&mut layout);

    // Second window halfway to the bottom.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x: 50 y: 50
    ");

    let ops = [
        // Issue a resize.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(200),
        },
        // The top window grows in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 200)),
        },
        // This starts the resize animation.
        Op::Communicate(1),
        Op::Communicate(2),
        // Advance the time slightly.
        Op::AdvanceAnimations { msec_delta: 50 },
    ];
    check_ops_on_layout(&mut layout, ops);

    // X changed by 5, but y changed by 8 since the Y movement from the resize compounds with the Y
    // movement from consume-into-column.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 105 at x:  0 y:  0
    200 × 200 at x: 45 y: 58
    ");

    let ops = [
        // Issue a resize back.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(100),
        },
        // The top window shrinks in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        // This cancels the resize animation since the change of 5 px is less than the resize
        // animation threshold.
        Op::Communicate(1),
        Op::Communicate(2),
    ];
    check_ops_on_layout(&mut layout, ops);

    // Since the resize anim was cancelled, second window's Y anim is adjusted to preserve the
    // current position while targeting the new final position.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x: 45 y: 58
    ");

    // Advance the time to complete the consume movement.
    Op::AdvanceAnimations { msec_delta: 450 }.apply(&mut layout);

    // Since we don't cancel the resize-induced part of the anim (in fact the move Y anim isn't
    // split into parts, so there's no way to tell), it keeps going still.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y: 78
    ");

    // Advance the time to complete the resize-induced anim.
    Op::AdvanceAnimations { msec_delta: 550 }.apply(&mut layout);

    // Final state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y:100
    ");
}

#[test]
fn height_resize_before_another_y_anim_then_back() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::FocusColumnLeft,
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        Op::SetForcedSize {
            id: 2,
            size: Some(Size::new(200, 200)),
        },
        Op::Communicate(1),
        Op::Communicate(2),
        Op::CompleteAnimations,
        // Issue a resize.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(200),
        },
        // The top window grows in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 200)),
        },
        // This starts the resize animation.
        Op::Communicate(1),
        Op::Communicate(2),
        // Advance the time a bit.
        Op::AdvanceAnimations { msec_delta: 200 },
    ];
    let mut layout = check_ops_with_options(make_options(), ops);

    // The resize is in progress.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 120 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");

    // Consume second window into column, starting the X/Y move anim down.
    Op::ConsumeWindowIntoColumn.apply(&mut layout);

    // No time had passed, so no change in coordinates yet.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 120 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");

    // Advance the time halfway.
    Op::AdvanceAnimations { msec_delta: 600 }.apply(&mut layout);

    // Second window halfway to the bottom. Since consume happened after the start of the first
    // window's resize, the second window's Y is unaffected by it and is animating towards the
    // final position right away.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 180 at x:  0 y:  0
    200 × 200 at x: 40 y:120
    ");

    let ops = [
        // Issue a resize back.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(100),
        },
        // The top window shrinks in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        // This starts the resize animation.
        Op::Communicate(1),
        Op::Communicate(2),
    ];
    check_ops_on_layout(&mut layout, ops);

    // No time had passed, so no change in state yet.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 180 at x:  0 y:  0
    200 × 200 at x: 40 y:120
    ");

    // Advance the time a bit. Both resize and consume movement are still ongoing.
    Op::AdvanceAnimations { msec_delta: 200 }.apply(&mut layout);

    assert_snapshot!(format_tiles(&layout), @r"
    100 × 164 at x:  0 y:  0
    200 × 200 at x: 20 y:116
    ");

    // Advance the time to complete the consume movement.
    Op::AdvanceAnimations { msec_delta: 200 }.apply(&mut layout);

    assert_snapshot!(format_tiles(&layout), @r"
    100 × 148 at x:  0 y:  0
    200 × 200 at x:  0 y:112
    ");

    // Advance the time to complete the resize.
    Op::AdvanceAnimations { msec_delta: 600 }.apply(&mut layout);

    // Final state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y:100
    ");
}

#[test]
fn height_resize_before_another_y_anim_then_cancel() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::FocusColumnLeft,
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        Op::SetForcedSize {
            id: 2,
            size: Some(Size::new(200, 200)),
        },
        Op::Communicate(1),
        Op::Communicate(2),
        Op::CompleteAnimations,
        // Issue a resize.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(200),
        },
        // The top window grows in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 200)),
        },
        // This starts the resize animation.
        Op::Communicate(1),
        Op::Communicate(2),
        // Advance the time a bit.
        Op::AdvanceAnimations { msec_delta: 20 },
    ];
    let mut layout = check_ops_with_options(make_options(), ops);

    // The resize is in progress.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 102 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");

    // Consume second window into column, starting the X/Y move anim down.
    Op::ConsumeWindowIntoColumn.apply(&mut layout);

    // No time had passed, so no change in coordinates yet.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 102 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");

    // Advance the time a little.
    Op::AdvanceAnimations { msec_delta: 20 }.apply(&mut layout);

    // Second window on its way to the bottom.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 104 at x:  0 y:  0
    200 × 200 at x: 98 y:  4
    ");

    let ops = [
        // Issue a resize back.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(100),
        },
        // The top window shrinks in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        // This cancels the resize animation since the change of 4 px is less than the resize
        // animation threshold.
        Op::Communicate(1),
        Op::Communicate(2),
    ];
    check_ops_on_layout(&mut layout, ops);

    // The second window's trajectory readjusts to the new final position at 100 px, without jumps.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x: 98 y:  4
    ");

    // Advance the time to complete the consume movement.
    Op::AdvanceAnimations { msec_delta: 980 }.apply(&mut layout);

    // Final state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y:100
    ");
}

#[test]
fn clientside_height_change_during_another_y_anim() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::FocusColumnLeft,
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        Op::SetForcedSize {
            id: 2,
            size: Some(Size::new(200, 200)),
        },
        Op::Communicate(1),
        Op::Communicate(2),
        Op::CompleteAnimations,
        Op::ConsumeWindowIntoColumn,
        // Clear the animate next configure flag.
        Op::Communicate(1),
        Op::Communicate(2),
        // Advance the time a bit.
        Op::AdvanceAnimations { msec_delta: 200 },
    ];
    let mut layout = check_ops_with_options(make_options(), ops);

    // Second window on its way to the bottom.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x: 80 y: 20
    ");

    let ops = [
        // The top window suddenly grows.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 200)),
        },
        Op::Communicate(1),
        Op::Communicate(2),
    ];
    check_ops_on_layout(&mut layout, ops);

    // The second window's trajectory readjusts to the new final position at 200 px, without jumps.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 200 at x:  0 y:  0
    200 × 200 at x: 80 y: 20
    ");

    // Advance the time to complete the consume movement.
    Op::AdvanceAnimations { msec_delta: 800 }.apply(&mut layout);

    // Final state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 200 at x:  0 y:  0
    200 × 200 at x:  0 y:200
    ");
}

#[test]
fn height_resize_cancel_with_stationary_second_window() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::FocusColumnLeft,
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        Op::SetForcedSize {
            id: 2,
            size: Some(Size::new(200, 200)),
        },
        Op::Communicate(1),
        Op::Communicate(2),
        Op::CompleteAnimations,
        // Issue a resize.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(200),
        },
        // The top window grows in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 200)),
        },
        // This starts the resize animation.
        Op::Communicate(1),
        Op::Communicate(2),
        // Advance the time a bit.
        Op::AdvanceAnimations { msec_delta: 20 },
    ];
    let mut options = make_options();
    // Window movement will happen instantly.
    options.animations.window_movement.0.off = true;
    let mut layout = check_ops_with_options(options, ops);

    // The resize is in progress.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 102 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");

    // Consume second window into column, starting the X/Y move anim down.
    Op::ConsumeWindowIntoColumn.apply(&mut layout);

    // No time had passed, so no change in coordinates yet.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 102 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");

    // Advance the time a little.
    Op::AdvanceAnimations { msec_delta: 20 }.apply(&mut layout);

    // The window movement anim is off, so the second window is already at the bottom. Since
    // consume started after the resize, the second window is unaffected by the resize-induced Y
    // movement, and sits at the final position at 200 px.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 104 at x:  0 y:  0
    200 × 200 at x:  0 y:200
    ");

    let ops = [
        // Issue a resize back.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(100),
        },
        // The top window shrinks in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        // This cancels the resize animation since the change of 4 px is less than the resize
        // animation threshold.
        Op::Communicate(1),
        Op::Communicate(2),
    ];
    check_ops_on_layout(&mut layout, ops);

    // This causes the second window to jump down, which is correct because it hadn't been in an
    // animation, and as far as it's concerned, this is the same case as a window just deciding to
    // do a clientside resize on its own, which is not animated.
    //
    // Since the resize is also cancelled, this is the final state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y:100
    ");
}

#[test]
fn width_resize_and_cancel() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::FocusColumnLeft,
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        Op::SetForcedSize {
            id: 2,
            size: Some(Size::new(200, 200)),
        },
        Op::Communicate(1),
        Op::Communicate(2),
        Op::CompleteAnimations,
    ];
    let mut layout = check_ops_with_options(make_options(), ops);

    // The initial state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");

    let ops = [
        // Issue a resize.
        Op::SetWindowWidth {
            id: None,
            change: SizeChange::SetFixed(200),
        },
        // The left window grows in response.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(200, 100)),
        },
        // This starts the resize animation.
        Op::Communicate(1),
        Op::Communicate(2),
        // Advance the time slightly.
        Op::AdvanceAnimations { msec_delta: 50 },
    ];
    check_ops_on_layout(&mut layout, ops);

    // Left window is half-resized at 105 px wide, right window is at x=105 matching it.
    assert_snapshot!(format_tiles(&layout), @r"
    105 × 100 at x:  0 y:  0
    200 × 200 at x:105 y:  0
    ");

    let ops = [
        // Issue a resize back.
        Op::SetWindowWidth {
            id: None,
            change: SizeChange::SetFixed(100),
        },
        // The top window shrinks in response.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        // This cancels the resize animation since the change of 5 px is less than the resize
        // animation threshold.
        Op::Communicate(1),
        Op::Communicate(2),
    ];
    check_ops_on_layout(&mut layout, ops);

    // Since the resize animation is cancelled, the width goes to the new value immediately. The X
    // position doesn't jump, instead the animation is restarted to preserve the current position.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:105 y:  0
    ");

    // Advance to the end of the move animation.
    Op::AdvanceAnimations { msec_delta: 1000 }.apply(&mut layout);

    // Final state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");
}

#[test]
fn width_resize_and_cancel_of_column_to_the_left() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        Op::SetForcedSize {
            id: 2,
            size: Some(Size::new(200, 200)),
        },
        Op::Communicate(1),
        Op::Communicate(2),
        Op::CompleteAnimations,
    ];
    let mut layout = check_ops_with_options(make_options(), ops);

    // The initial state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");

    let ops = [
        // Issue a resize.
        Op::SetWindowWidth {
            id: Some(1),
            change: SizeChange::SetFixed(200),
        },
        // The left window grows in response.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(200, 100)),
        },
        // This starts the resize animation.
        Op::Communicate(1),
        Op::Communicate(2),
        // Advance the time slightly.
        Op::AdvanceAnimations { msec_delta: 50 },
    ];
    check_ops_on_layout(&mut layout, ops);

    // Left window is half-resized at 105 px wide, it's at x=-5 matching the right edge position.
    assert_snapshot!(format_tiles(&layout), @r"
    105 × 100 at x: -5 y:  0
    200 × 200 at x:100 y:  0
    ");

    let ops = [
        // Issue a resize back.
        Op::SetWindowWidth {
            id: Some(1),
            change: SizeChange::SetFixed(100),
        },
        // The top window shrinks in response.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        // This cancels the resize animation since the change of 5 px is less than the resize
        // animation threshold.
        Op::Communicate(1),
        Op::Communicate(2),
    ];
    check_ops_on_layout(&mut layout, ops);

    // Since the resize animation is cancelled, the width goes to the new value immediately. The X
    // position doesn't jump, instead the animation is restarted to preserve the current position.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x: -5 y:  0
    200 × 200 at x:100 y:  0
    ");

    // Advance to the end of the move animation.
    Op::AdvanceAnimations { msec_delta: 1000 }.apply(&mut layout);

    // Final state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");
}

fn set_up_workspaces(count: usize) -> Layout<TestWindow> {
    let mut options = make_options();
    options.animations.workspace_switch.0.kind = Kind::Easing(EasingParams {
        duration_ms: 1000,
        curve: Curve::Linear,
    });

    let mut layout = Layout::with_options(Clock::with_time(Duration::ZERO), options);
    check_ops_on_layout(&mut layout, [Op::AddOutput(1)]);

    // One window per workspace, plus the empty workspace at the end.
    for id in 1..count {
        check_ops_on_layout(
            &mut layout,
            [
                Op::AddWindow {
                    params: TestWindowParams::new(id),
                },
                Op::FocusWorkspaceDown,
            ],
        );
    }
    check_ops_on_layout(&mut layout, [Op::FocusWorkspace(0), Op::CompleteAnimations]);
    layout
}

fn workspace_geos(layout: &Layout<TestWindow>) -> Vec<Rectangle<f64, Logical>> {
    let mon = layout.monitors().next().unwrap();
    mon.workspaces_render_geo().collect()
}

fn is_visible(layout: &Layout<TestWindow>, idx: usize) -> bool {
    let mon = layout.monitors().next().unwrap();
    let output = Rectangle::from_size(
        mon.output()
            .current_mode()
            .unwrap()
            .size
            .to_logical(1)
            .to_f64(),
    );
    workspace_geos(layout)[idx].intersection(output).is_some()
}

#[test]
fn workspace_switch_skips_workspaces_in_between() {
    let mut layout = set_up_workspaces(5);
    let at_rest = workspace_geos(&layout);
    let step = at_rest[1].loc.y - at_rest[0].loc.y;

    check_ops_on_layout(
        &mut layout,
        [
            Op::FocusWorkspace(4),
            Op::AdvanceAnimations { msec_delta: 500 },
        ],
    );

    let geos = workspace_geos(&layout);
    assert!(is_visible(&layout, 0));
    assert!(!is_visible(&layout, 1));
    assert!(!is_visible(&layout, 2));
    assert!(!is_visible(&layout, 3));
    assert!(is_visible(&layout, 4));

    // Halfway through, the two workspaces are next to each other, each half moved.
    assert_eq!(geos[4].loc.y - geos[0].loc.y, step);
    assert_eq!(geos[0].loc.y, at_rest[0].loc.y - step / 2.);

    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
    let geos = workspace_geos(&layout);
    assert_eq!(geos[4].loc.y, at_rest[0].loc.y);
}

#[test]
fn workspace_switch_skips_upwards() {
    let mut layout = set_up_workspaces(5);
    check_ops_on_layout(&mut layout, [Op::FocusWorkspace(4), Op::CompleteAnimations]);
    let at_rest = workspace_geos(&layout);
    let step = at_rest[1].loc.y - at_rest[0].loc.y;

    check_ops_on_layout(
        &mut layout,
        [
            Op::FocusWorkspace(1),
            Op::AdvanceAnimations { msec_delta: 250 },
        ],
    );

    let geos = workspace_geos(&layout);
    assert!(is_visible(&layout, 4));
    assert!(is_visible(&layout, 1));
    assert!(!is_visible(&layout, 2));
    assert!(!is_visible(&layout, 3));
    // Workspace 1 is above workspace 4.
    assert_eq!(geos[4].loc.y - geos[1].loc.y, step);
}

#[test]
fn workspace_switch_to_neighbor_is_unchanged() {
    let mut layout = set_up_workspaces(5);
    let at_rest = workspace_geos(&layout);
    let step = at_rest[1].loc.y - at_rest[0].loc.y;

    check_ops_on_layout(
        &mut layout,
        [
            Op::FocusWorkspace(1),
            Op::AdvanceAnimations { msec_delta: 500 },
        ],
    );

    let geos = workspace_geos(&layout);
    assert_eq!(geos[0].loc.y, at_rest[0].loc.y - step / 2.);
    assert_eq!(geos[1].loc.y, at_rest[1].loc.y - step / 2.);
    assert_eq!(geos[2].loc.y, at_rest[2].loc.y - step / 2.);
}

/// Indices and Y positions of the workspaces on screen, in a stable order.
fn visible_workspaces(layout: &Layout<TestWindow>) -> Vec<(usize, f64)> {
    let geos = workspace_geos(layout);
    (0..geos.len() - 1)
        .filter(|&idx| is_visible(layout, idx))
        .map(|idx| (idx, geos[idx].loc.y))
        .collect()
}

/// Checks that the workspaces that were on screen didn't move because of `ops`.
#[track_caller]
fn assert_view_continuous(layout: &mut Layout<TestWindow>, ops: impl IntoIterator<Item = Op>) {
    let before = visible_workspaces(layout);
    check_ops_on_layout(layout, ops);
    let after = visible_workspaces(layout);
    for (idx, y) in before {
        let moved = after.iter().find(|(i, _)| *i == idx);
        assert_eq!(moved.map(|(_, y)| *y), Some(y), "workspace {idx} jumped");
    }
}

#[test]
fn skipping_switch_reversed_midway_is_continuous() {
    let mut layout = set_up_workspaces(5);
    check_ops_on_layout(
        &mut layout,
        [
            Op::FocusWorkspace(4),
            Op::AdvanceAnimations { msec_delta: 300 },
        ],
    );
    assert_eq!(visible_workspaces(&layout).len(), 2);

    assert_view_continuous(&mut layout, [Op::FocusWorkspace(0)]);

    // It goes back to where it started, without going through the workspaces in between.
    check_ops_on_layout(&mut layout, [Op::AdvanceAnimations { msec_delta: 400 }]);
    assert!(!is_visible(&layout, 2));
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
    assert_eq!(workspace_geos(&layout)[0].loc.y, 0.);
    assert_eq!(visible_workspaces(&layout), vec![(0, 0.)]);
}

#[test]
fn skipping_switch_to_third_workspace_midway_is_continuous() {
    let mut layout = set_up_workspaces(5);
    check_ops_on_layout(
        &mut layout,
        [
            Op::FocusWorkspace(4),
            Op::AdvanceAnimations { msec_delta: 300 },
        ],
    );

    assert_view_continuous(&mut layout, [Op::FocusWorkspace(2)]);
    check_ops_on_layout(&mut layout, [Op::AdvanceAnimations { msec_delta: 500 }]);
    assert!(!is_visible(&layout, 3));
    assert!(!is_visible(&layout, 1));

    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
    assert_eq!(workspace_geos(&layout)[2].loc.y, 0.);
}

#[test]
fn skipping_switch_to_the_same_target_midway_is_noop() {
    let mut layout = set_up_workspaces(5);
    check_ops_on_layout(
        &mut layout,
        [
            Op::FocusWorkspace(4),
            Op::AdvanceAnimations { msec_delta: 300 },
        ],
    );
    let before = workspace_geos(&layout);
    check_ops_on_layout(&mut layout, [Op::FocusWorkspace(4)]);
    assert_eq!(workspace_geos(&layout), before);
}

#[test]
fn skipping_switch_interrupts_neighbor_switch_midway() {
    let mut layout = set_up_workspaces(5);
    // Halfway between 0 and 1, then to somewhere far away.
    check_ops_on_layout(
        &mut layout,
        [
            Op::FocusWorkspace(1),
            Op::AdvanceAnimations { msec_delta: 500 },
        ],
    );
    assert_eq!(visible_workspaces(&layout).len(), 2);

    assert_view_continuous(&mut layout, [Op::FocusWorkspace(4)]);
    check_ops_on_layout(&mut layout, [Op::AdvanceAnimations { msec_delta: 500 }]);
    assert!(!is_visible(&layout, 2));
    assert!(!is_visible(&layout, 3) || is_visible(&layout, 4));

    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
    assert_eq!(workspace_geos(&layout)[4].loc.y, 0.);
}

#[test]
fn skipping_switch_keeps_going_up_and_down() {
    // Back and forth between several targets at arbitrary times, nothing may jump.
    let mut layout = set_up_workspaces(6);
    for (target, msec_delta) in [(5, 100), (0, 250), (3, 50), (1, 400), (5, 10), (2, 700)] {
        check_ops_on_layout(
            &mut layout,
            [
                Op::FocusWorkspace(target),
                Op::AdvanceAnimations { msec_delta },
            ],
        );
        let index = visible_workspaces(&layout);
        assert!((1..=3).contains(&index.len()), "{index:?}");
    }
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
    assert_eq!(workspace_geos(&layout)[2].loc.y, 0.);
}

#[test]
fn skipping_switch_is_dimmed_by_distance() {
    let mut layout = set_up_workspaces(5);
    let dimming =
        |layout: &Layout<TestWindow>| layout.monitors().next().unwrap().skip_dimming().to_vec();
    assert!(dimming(&layout).is_empty());

    check_ops_on_layout(&mut layout, [Op::FocusWorkspace(4)]);
    // At the very start the active workspace is in place, and the other one is out of view.
    let start = dimming(&layout);
    assert_eq!(start.len(), 1);
    assert_eq!(start[0].0, 4);

    check_ops_on_layout(&mut layout, [Op::AdvanceAnimations { msec_delta: 500 }]);
    let middle = dimming(&layout);
    assert_eq!(middle.len(), 2);
    assert_eq!(middle[0].1, middle[1].1);
    assert!(middle[0].1 > 0.);

    check_ops_on_layout(&mut layout, [Op::AdvanceAnimations { msec_delta: 400 }]);
    let late = dimming(&layout);
    let (leaving, entering) = (
        late.iter().find(|(idx, _)| *idx == 0).unwrap().1,
        late.iter().find(|(idx, _)| *idx == 4).unwrap().1,
    );
    assert!(leaving > middle[0].1);
    assert!(entering < middle[0].1);

    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
    assert!(dimming(&layout).is_empty());
}

#[test]
fn neighbor_switch_is_not_dimmed() {
    let mut layout = set_up_workspaces(5);
    check_ops_on_layout(
        &mut layout,
        [
            Op::FocusWorkspace(1),
            Op::AdvanceAnimations { msec_delta: 500 },
        ],
    );
    assert!(layout.monitors().next().unwrap().skip_dimming().is_empty());
}

#[test]
fn opening_overview_during_skipping_switch_is_continuous() {
    let mut layout = set_up_workspaces(5);
    check_ops_on_layout(
        &mut layout,
        [
            Op::FocusWorkspace(4),
            Op::AdvanceAnimations { msec_delta: 350 },
        ],
    );

    // The most visible workspace stays where it is.
    let (idx, y) = visible_workspaces(&layout)
        .into_iter()
        .min_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .unwrap();
    check_ops_on_layout(&mut layout, [Op::ToggleOverview]);
    assert_eq!(workspace_geos(&layout)[idx].loc.y, y);
    assert!(layout.monitors().next().unwrap().skip_dimming().is_empty());

    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
}

#[test]
fn switching_in_overview_does_not_skip() {
    let mut layout = set_up_workspaces(5);
    check_ops_on_layout(&mut layout, [Op::ToggleOverview, Op::CompleteAnimations]);
    check_ops_on_layout(
        &mut layout,
        [
            Op::FocusWorkspace(4),
            Op::AdvanceAnimations { msec_delta: 500 },
        ],
    );
    assert!(layout.monitors().next().unwrap().skip_dimming().is_empty());
    assert!(is_visible(&layout, 2));
}

#[test]
fn gesture_during_skipping_switch_is_continuous() {
    let mut layout = set_up_workspaces(5);
    check_ops_on_layout(
        &mut layout,
        [
            Op::FocusWorkspace(4),
            Op::AdvanceAnimations { msec_delta: 350 },
        ],
    );

    let (idx, y) = visible_workspaces(&layout)
        .into_iter()
        .min_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .unwrap();
    check_ops_on_layout(
        &mut layout,
        [Op::WorkspaceSwitchGestureBegin {
            output_idx: 1,
            is_touchpad: true,
        }],
    );
    assert_eq!(workspace_geos(&layout)[idx].loc.y, y);
    assert!(layout.monitors().next().unwrap().skip_dimming().is_empty());

    // Ending the gesture never skips, it's a regular animation.
    check_ops_on_layout(
        &mut layout,
        [
            Op::WorkspaceSwitchGestureEnd {
                is_touchpad: Some(true),
            },
            Op::AdvanceAnimations { msec_delta: 10 },
        ],
    );
    assert!(layout.monitors().next().unwrap().skip_dimming().is_empty());
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
}

#[test]
fn skipping_switch_survives_workspace_insertions() {
    let mut options = make_options();
    options.layout.empty_workspace_above_first = true;
    let mut layout = Layout::with_options(Clock::with_time(Duration::ZERO), options);

    check_ops_on_layout(&mut layout, [Op::AddOutput(1)]);
    for id in 1..=3 {
        check_ops_on_layout(
            &mut layout,
            [
                Op::AddWindow {
                    params: TestWindowParams::new(id),
                },
                Op::FocusWorkspaceDown,
                Op::CompleteAnimations,
            ],
        );
    }

    // Going to the empty workspace at the top, then adding a window to it which inserts another
    // empty workspace above, in the middle of the switch.
    check_ops_on_layout(
        &mut layout,
        [
            Op::FocusWorkspace(0),
            Op::AdvanceAnimations { msec_delta: 100 },
        ],
    );
    assert_eq!(visible_workspaces(&layout).len(), 2);
    let before = visible_workspaces(&layout);
    check_ops_on_layout(
        &mut layout,
        [Op::AddWindow {
            params: TestWindowParams::new(100),
        }],
    );

    // Everything on screen shifted down by the new workspace, but didn't move.
    let after = visible_workspaces(&layout);
    assert_eq!(before.len(), after.len());
    for ((before_idx, before_y), (after_idx, after_y)) in before.into_iter().zip(after) {
        assert_eq!(before_idx + 1, after_idx);
        assert_eq!(before_y, after_y);
    }

    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
    let active = layout.monitors().next().unwrap().active_workspace_idx();
    assert_eq!(workspace_geos(&layout)[active].loc.y, 0.);
}

/// Y position on screen of the window, including its move animation.
fn window_screen_y(layout: &mut Layout<TestWindow>, id: usize) -> f64 {
    let mon = layout.monitors_mut().next().unwrap();
    mon.workspaces_with_render_geo_mut(false)
        .find_map(|(ws, geo)| {
            ws.tiles_with_render_positions_mut(false)
                .find(|(tile, _)| tile.window().id() == &id)
                .map(|(_, pos)| geo.loc.y + pos.y)
        })
        .unwrap()
}

#[test]
fn moving_window_across_skipped_workspaces_starts_from_where_it_was() {
    let mut layout = set_up_workspaces(5);
    let before = window_screen_y(&mut layout, 1);

    check_ops_on_layout(
        &mut layout,
        [Op::MoveWindowToWorkspace {
            window_id: Some(1),
            workspace_idx: 4,
        }],
    );
    assert_eq!(window_screen_y(&mut layout, 1), before);

    // And then arrives at the new workspace together with the view.
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
    let at_rest = window_screen_y(&mut layout, 1);
    assert_eq!(at_rest, before);
}

#[test]
fn moving_column_across_skipped_workspaces_starts_from_where_it_was() {
    let mut layout = set_up_workspaces(5);
    let before = window_screen_y(&mut layout, 1);

    check_ops_on_layout(&mut layout, [Op::MoveColumnToWorkspace(4, true)]);
    assert_eq!(window_screen_y(&mut layout, 1), before);

    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
    assert_eq!(window_screen_y(&mut layout, 1), before);
}

#[test]
fn moving_window_during_skipping_switch_does_not_jump() {
    let mut layout = set_up_workspaces(5);
    check_ops_on_layout(
        &mut layout,
        [
            Op::FocusWorkspace(4),
            Op::AdvanceAnimations { msec_delta: 400 },
        ],
    );
    let before = window_screen_y(&mut layout, 1);

    // Active workspace is 4, and 0 is still on screen.
    check_ops_on_layout(
        &mut layout,
        [Op::MoveWindowToWorkspace {
            window_id: Some(1),
            workspace_idx: 4,
        }],
    );
    assert_eq!(window_screen_y(&mut layout, 1), before);
}

#[test]
fn repeated_neighbor_switches_are_not_skipping() {
    // Holding the key to go through workspaces one by one is a regular scroll.
    let mut layout = set_up_workspaces(6);
    for target in 1..5 {
        check_ops_on_layout(
            &mut layout,
            [
                Op::FocusWorkspace(target),
                Op::AdvanceAnimations { msec_delta: 50 },
            ],
        );
        assert!(
            layout.monitors().next().unwrap().skip_dimming().is_empty(),
            "switch to {target} is skipping"
        );
    }

    // All of the workspaces in between were seen on the way.
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
}
