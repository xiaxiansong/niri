use insta::assert_snapshot;

use super::*;

fn window(id: usize, width: i32) -> Op {
    let mut params = TestWindowParams::new(id);
    params.bbox = Rectangle::from_size(Size::from((width, 200)));
    Op::AddWindow { params }
}

fn floating_window(id: usize) -> Op {
    let mut params = TestWindowParams::new(id);
    params.is_floating = true;
    Op::AddWindow { params }
}

fn hide(id: usize) -> Op {
    Op::MoveWindowToScratchpad { id: Some(id) }
}

fn toggle() -> Op {
    Op::ToggleScratchpad { id: None }
}

/// Hidden queue, shown windows and the focused window.
fn summary(layout: &Layout<TestWindow>) -> String {
    let hidden: Vec<_> = layout
        .scratchpad_hidden_windows()
        .map(|win| win.0.id)
        .collect();
    let mut shown: Vec<_> = layout
        .windows()
        .map(|(_, win)| win)
        .filter(|win| win.scratchpad_state() == ScratchpadState::Shown)
        .map(|win| win.0.id)
        .collect();
    shown.sort();
    let focus = layout.focus().map(|win| win.0.id);
    format!("hidden: {hidden:?}, shown: {shown:?}, focus: {focus:?}")
}

fn state(layout: &Layout<TestWindow>, id: usize) -> ScratchpadState {
    layout
        .windows()
        .map(|(_, win)| win)
        .chain(layout.scratchpad_hidden_windows())
        .find(|win| win.0.id == id)
        .unwrap()
        .scratchpad_state()
}

/// Position and requested size of a floating window on the active workspace.
fn floating_geometry(layout: &Layout<TestWindow>, id: usize) -> String {
    let ws = layout.active_workspace().unwrap();
    let (tile, pos) = ws
        .floating()
        .tiles_with_offsets()
        .find(|(tile, _)| tile.window().0.id == id)
        .unwrap();
    let size = tile.window().requested_size().unwrap();
    format!("pos: {} {}, size: {} {}", pos.x, pos.y, size.w, size.h)
}

/// Runs the ops with fill-view-after-removal off and on, and returns the resulting view positions
/// of the first workspace after completing the animations.
fn off_on(ops: &[Op]) -> String {
    let run = |fill| {
        let mut options = Options::default();
        options.layout.fill_view_after_removal = fill;
        let mut layout = check_ops_with_options(options, ops.iter().cloned());
        Op::CompleteAnimations.apply(&mut layout);
        let (_, _, ws) = layout.workspaces().next().unwrap();
        ws.scrolling().view_pos()
    };

    format!("off: {}, on: {}", run(false), run(true))
}

#[test]
fn floating_window_keeps_geometry() {
    let mut layout = check_ops([
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
        Op::CompleteAnimations,
    ]);
    let before = floating_geometry(&layout, 2);
    assert_snapshot!(before, @"pos: 100 50, size: 100 200");

    check_ops_on_layout(
        &mut layout,
        [
            hide(2),
            toggle(),
            Op::Communicate(2),
            Op::CompleteAnimations,
        ],
    );
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [2], focus: Some(2)");
    assert_eq!(floating_geometry(&layout, 2), before);
}

#[test]
fn tiled_window_first_shown_with_default_size() {
    // Four 500 px wide columns at x = 0, 516, 1032, 1548, with the view at the right end (784) and
    // the last column focused.
    let setup = [
        Op::AddOutput(1),
        window(1, 500),
        window(2, 500),
        window(3, 500),
        window(4, 500),
        Op::FocusColumnFirst,
        Op::FocusColumnLast,
        Op::CompleteAnimations,
        hide(4),
    ];

    // Like closing the column: the right snap is now at 1032 + 500 + 16 - 1280 = 268.
    assert_snapshot!(off_on(&setup), @"off: 784, on: 268");

    // 60% of the 1280x720 working area. The window hasn't resized yet, so its 500x200 tile is
    // centered.
    let mut layout = check_ops(setup);
    check_ops_on_layout(&mut layout, [toggle()]);
    assert_snapshot!(floating_geometry(&layout, 4), @"pos: 390 260, size: 768 432");

    // Window rules take precedence: 300 px wide, half of 720 px tall.
    let mut params = TestWindowParams::new(5);
    params.rules = Some(ResolvedWindowRules {
        default_width: Some(Some(PresetSize::Fixed(300))),
        default_height: Some(Some(PresetSize::Proportion(0.5))),
        ..ResolvedWindowRules::default()
    });
    let mut layout = check_ops([
        Op::AddOutput(1),
        Op::AddWindow { params },
        hide(5),
        toggle(),
    ]);
    assert_snapshot!(floating_geometry(&layout, 5), @"pos: 590 260, size: 300 360");

    // Once the window is floating, its floating size is remembered.
    check_ops_on_layout(
        &mut layout,
        [
            Op::Communicate(5),
            Op::SetWindowWidth {
                id: Some(5),
                change: SizeChange::SetFixed(400),
            },
            Op::Communicate(5),
            hide(5),
            toggle(),
        ],
    );
    let size = layout.windows().next().unwrap().1.requested_size().unwrap();
    assert_eq!(size, Size::from((400, 360)));
}

#[test]
fn generic_toggle_cycles_through_queue() {
    let mut layout = check_ops([
        Op::AddOutput(1),
        window(1, 100),
        window(2, 100),
        window(3, 100),
        hide(1),
        hide(2),
        hide(3),
    ]);

    let mut shown = Vec::new();
    for _ in 0..7 {
        check_ops_on_layout(&mut layout, [toggle()]);
        let ids: Vec<_> = layout
            .windows()
            .map(|(_, win)| win.0.id.to_string())
            .collect();
        shown.push(if ids.is_empty() {
            String::from("-")
        } else {
            ids.join(",")
        });
    }

    assert_snapshot!(shown.join(" "), @"1 - 2 - 3 - 1");
}

#[test]
fn toggle_by_id_keeps_queue_order() {
    let mut layout = check_ops([
        Op::AddOutput(1),
        window(1, 100),
        window(2, 100),
        window(3, 100),
        hide(1),
        hide(2),
        hide(3),
        Op::ToggleScratchpad { id: Some(2) },
    ]);
    assert_snapshot!(summary(&layout), @"hidden: [1, 3], shown: [2], focus: Some(2)");

    check_ops_on_layout(&mut layout, [Op::ToggleScratchpad { id: Some(2) }]);
    assert_snapshot!(summary(&layout), @"hidden: [1, 3, 2], shown: [], focus: None");
}

#[test]
fn generic_toggle_focuses_most_recently_shown() {
    let mut layout = check_ops([
        Op::AddOutput(1),
        window(1, 100),
        window(2, 100),
        window(3, 100),
        window(4, 100),
        hide(1),
        hide(2),
        hide(3),
        Op::ToggleScratchpad { id: Some(1) },
        Op::ToggleScratchpad { id: Some(2) },
        Op::FocusWindow(4),
    ]);
    assert_snapshot!(summary(&layout), @"hidden: [3], shown: [1, 2], focus: Some(4)");

    assert_eq!(
        layout.toggle_scratchpad(|_| true),
        ScratchpadToggle::Focused(2)
    );
    layout.verify_invariants();
    assert_snapshot!(summary(&layout), @"hidden: [3], shown: [1, 2], focus: Some(2)");
}

#[test]
fn leaving_floating_takes_out_of_scratchpad() {
    let setup = [
        Op::AddOutput(1),
        window(1, 100),
        window(2, 100),
        hide(2),
        toggle(),
    ];

    let check = |op: Op| {
        let mut ops = setup.to_vec();
        ops.push(op);
        let layout = check_ops(ops);
        format!("{:?}", state(&layout, 2))
    };

    assert_snapshot!(summary(&check_ops(setup.clone())), @"hidden: [], shown: [2], focus: Some(2)");
    assert_snapshot!(check(Op::ToggleWindowFloating { id: Some(2) }), @"None");
    assert_snapshot!(check(Op::FullscreenWindow(2)), @"None");
    assert_snapshot!(check(Op::MaximizeWindowToEdges { id: Some(2) }), @"None");
    assert_snapshot!(check(Op::MoveWindowToWorkspace {
        window_id: Some(2),
        workspace_idx: 1,
    }), @"None");
}

#[test]
fn interactive_move_of_shown_window() {
    let setup = [
        Op::AddOutput(1),
        Op::AddOutput(2),
        Op::FocusOutput(1),
        window(1, 100),
        window(2, 100),
        hide(2),
        toggle(),
    ];

    let check = |output_idx, ops: &[Op]| {
        let mut all = setup.to_vec();
        all.extend([
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
                output_idx,
                px: 640.,
                py: 360.,
            },
        ]);
        all.extend_from_slice(ops);
        let mut layout = check_ops(all);
        let during = state(&layout, 2);
        check_ops_on_layout(&mut layout, [Op::InteractiveMoveEnd { window: 2 }]);
        let floating = layout
            .workspaces()
            .any(|(_, _, ws)| ws.has_window(&2) && ws.is_floating(&2));
        format!(
            "during: {during:?}, after: {:?}, floating: {floating}",
            state(&layout, 2)
        )
    };

    // Dropped onto the same workspace.
    assert_snapshot!(check(1, &[]), @"during: Shown, after: Shown, floating: true");
    // Dropped onto the active workspace of another output.
    assert_snapshot!(check(2, &[]), @"during: Shown, after: Shown, floating: true");
    // Dropped into the scrolling layout.
    assert_snapshot!(
        check(1, &[Op::ToggleWindowFloating { id: None }]),
        @"during: Shown, after: None, floating: false"
    );
}

#[test]
fn close_hidden_window() {
    let layout = check_ops([
        Op::AddOutput(1),
        window(1, 100),
        window(2, 100),
        hide(2),
        Op::CloseWindow(2),
    ]);
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [], focus: Some(1)");
}

#[test]
fn hidden_windows_survive_output_removal() {
    let mut layout = check_ops([
        Op::AddOutput(1),
        window(1, 100),
        window(2, 100),
        hide(1),
        hide(2),
        Op::RemoveOutput(1),
    ]);
    assert_snapshot!(summary(&layout), @"hidden: [1, 2], shown: [], focus: None");

    // There's nowhere to show the windows without outputs.
    check_ops_on_layout(
        &mut layout,
        [
            toggle(),
            Op::ToggleScratchpad { id: Some(2) },
            Op::RemoveWindowFromScratchpad { id: Some(1) },
            Op::FocusWindow(1),
        ],
    );
    assert_snapshot!(summary(&layout), @"hidden: [1, 2], shown: [], focus: None");

    check_ops_on_layout(&mut layout, [Op::AddOutput(1)]);
    assert_snapshot!(summary(&layout), @"hidden: [1, 2], shown: [], focus: None");

    check_ops_on_layout(&mut layout, [toggle()]);
    assert_snapshot!(summary(&layout), @"hidden: [2], shown: [1], focus: Some(1)");
}

#[test]
fn hidden_window_commit_does_not_animate() {
    // The window was asked to resize to the column size, but hasn't committed yet.
    let mut layout = check_ops([Op::AddOutput(1), window(1, 100), hide(1)]);

    // Committing the new size would start a resize animation.
    check_ops_on_layout(&mut layout, [Op::Communicate(1)]);
    let tile = &layout.scratchpad.hidden[0];
    assert_eq!(tile.window().size(), Size::from((100, 688)));
    assert!(!tile.are_transitions_ongoing());
}

#[test]
fn activating_hidden_window_shows_it() {
    let layout = check_ops([
        Op::AddOutput(1),
        window(1, 100),
        window(2, 100),
        hide(2),
        Op::FocusWindow(2),
    ]);
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [2], focus: Some(2)");
}

#[test]
fn hiding_twice_is_noop() {
    let mut layout = check_ops([Op::AddOutput(1), window(1, 100), window(2, 100), hide(2)]);
    assert_eq!(layout.move_to_scratchpad(Some(&2)), None);
    layout.verify_invariants();
    assert_snapshot!(summary(&layout), @"hidden: [2], shown: [], focus: Some(1)");
}

#[test]
fn remove_hidden_window_from_scratchpad() {
    let layout = check_ops([
        Op::AddOutput(1),
        window(1, 100),
        window(2, 100),
        hide(2),
        Op::RemoveWindowFromScratchpad { id: Some(2) },
    ]);
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [], focus: Some(2)");
    assert_eq!(state(&layout, 2), ScratchpadState::None);
    assert!(layout.active_workspace().unwrap().is_floating(&2));
}

#[test]
fn shown_window_elsewhere_is_brought_over() {
    // Output name, workspace index and workspace count where the window is.
    let location = |layout: &Layout<TestWindow>, id: usize| {
        let (mon, idx, _) = layout
            .workspaces()
            .find(|(_, _, ws)| ws.has_window(&id))
            .unwrap();
        let mon = mon.unwrap();
        format!("{} ws {idx}/{}", mon.output_name(), mon.workspaces.len())
    };

    // Shown on another output.
    let mut layout = check_ops([
        Op::AddOutput(1),
        Op::AddOutput(2),
        Op::FocusOutput(1),
        window(1, 100),
        window(2, 100),
        hide(2),
        toggle(),
        Op::FocusOutput(2),
    ]);
    assert_eq!(
        layout.toggle_scratchpad(|_| true),
        ScratchpadToggle::Shown(2)
    );
    layout.verify_invariants();
    assert_snapshot!(location(&layout, 2), @"output2 ws 0/2");
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [2], focus: Some(2)");

    // Shown on an inactive workspace of the same output.
    let layout = check_ops([
        Op::AddOutput(1),
        window(1, 100),
        window(2, 100),
        hide(2),
        toggle(),
        Op::FocusWorkspaceDown,
        Op::CompleteAnimations,
        toggle(),
    ]);
    assert_snapshot!(location(&layout, 2), @"output1 ws 1/3");
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [2], focus: Some(2)");

    // The source workspace becomes empty and gets cleaned up.
    let layout = check_ops([
        Op::AddOutput(1),
        window(1, 100),
        hide(1),
        toggle(),
        Op::FocusWorkspaceDown,
        Op::CompleteAnimations,
        toggle(),
    ]);
    assert_snapshot!(location(&layout, 1), @"output1 ws 0/2");
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [1], focus: Some(1)");
}
