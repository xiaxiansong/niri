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

    // 60% of the 1280x720 working area, centered for that size even though the window hasn't
    // resized yet.
    let mut layout = check_ops(setup);
    check_ops_on_layout(&mut layout, [toggle()]);
    assert_snapshot!(floating_geometry(&layout, 4), @"pos: 256 144, size: 768 432");

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
    assert_snapshot!(floating_geometry(&layout, 5), @"pos: 490 180, size: 300 360");

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
fn hidden_tiled_window_resizes_before_shown() {
    // Right after hiding, a tiled window is asked for 60% of the 1280x720 working area.
    let mut layout = check_ops([Op::AddOutput(1), window(1, 500), hide(1)]);
    let size = layout.windows().next().unwrap().1.requested_size().unwrap();
    assert_eq!(size, Size::from((768, 432)));

    // Whether or not the window has resized by the time it's shown, it's shown in the same place,
    // centered for that size.
    let mut resized = check_ops([
        Op::AddOutput(1),
        window(1, 500),
        hide(1),
        Op::Communicate(1),
        toggle(),
    ]);
    check_ops_on_layout(&mut layout, [toggle()]);
    assert_snapshot!(floating_geometry(&layout, 1), @"pos: 256 144, size: 768 432");
    assert_snapshot!(floating_geometry(&resized, 1), @"pos: 256 144, size: 768 432");

    // Showing doesn't ask for anything new.
    check_ops_on_layout(&mut resized, [Op::Communicate(1)]);
    let win = resized.windows().next().unwrap().1;
    assert_eq!(win.requested_size(), Some(Size::from((768, 432))));

    // A window that was floating before keeps its floating size and position, and is asked for
    // that size right away too.
    let layout = check_ops([
        Op::AddOutput(1),
        floating_window(2),
        Op::Communicate(2),
        Op::ToggleWindowFloating { id: Some(2) },
        Op::Communicate(2),
        hide(2),
    ]);
    let size = layout.windows().next().unwrap().1.requested_size().unwrap();
    assert_eq!(size, Size::from((100, 200)));
    let mut layout = layout;
    check_ops_on_layout(&mut layout, [toggle()]);
    assert_snapshot!(floating_geometry(&layout, 2), @"pos: 590 260, size: 100 200");

    // The default-floating-position rule places it by its new size too.
    let mut params = TestWindowParams::new(3);
    params.bbox = Rectangle::from_size(Size::from((500, 200)));
    params.rules = Some(ResolvedWindowRules {
        default_floating_position: Some(niri_config::FloatingPosition {
            x: niri_config::FloatOrInt(0.),
            y: niri_config::FloatOrInt(0.),
            relative_to: niri_config::RelativeTo::Bottom,
        }),
        ..ResolvedWindowRules::default()
    });
    let layout = check_ops([
        Op::AddOutput(1),
        Op::AddWindow { params },
        hide(3),
        toggle(),
    ]);
    assert_snapshot!(floating_geometry(&layout, 3), @"pos: 256 288, size: 768 432");
}

fn toggle_in(id: Option<usize>) -> Op {
    Op::ToggleWindowInScratchpad { id }
}

#[test]
fn toggle_window_in_scratchpad() {
    // A window that's not in the scratchpad is hidden.
    let mut layout = check_ops([
        Op::AddOutput(1),
        window(1, 100),
        window(2, 100),
        toggle_in(None),
    ]);
    assert_snapshot!(summary(&layout), @"hidden: [2], shown: [], focus: Some(1)");

    // A shown window is taken out of the scratchpad, and stays where it is as a floating window.
    check_ops_on_layout(&mut layout, [toggle(), Op::Communicate(2)]);
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [2], focus: Some(2)");
    let before = floating_geometry(&layout, 2);
    check_ops_on_layout(&mut layout, [toggle_in(None)]);
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [], focus: Some(2)");
    assert_eq!(floating_geometry(&layout, 2), before);

    // Then it goes back in.
    check_ops_on_layout(&mut layout, [toggle_in(None)]);
    assert_snapshot!(summary(&layout), @"hidden: [2], shown: [], focus: Some(1)");

    // A hidden window is shown and taken out of the scratchpad.
    check_ops_on_layout(&mut layout, [toggle_in(Some(2))]);
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [], focus: Some(2)");
    let win = layout.windows().find(|(_, win)| win.0.id == 2).unwrap().1;
    assert_eq!(win.scratchpad_state(), ScratchpadState::None);
    floating_geometry(&layout, 2);
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
            .map(|(_, win)| win)
            .filter(|win| win.scratchpad_state() == ScratchpadState::Shown)
            .map(|win| win.0.id.to_string())
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
    // The window was asked to resize to its floating size on hiding, but hasn't committed yet.
    let mut layout = check_ops([Op::AddOutput(1), window(1, 100), hide(1)]);

    // Committing the new size would start a resize animation.
    check_ops_on_layout(&mut layout, [Op::Communicate(1)]);
    let tile = &layout.scratchpad.hidden[0];
    assert_eq!(tile.window().size(), Size::from((768, 432)));
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

    // Shown on another output. Shown windows on inactive workspaces of the same output can't
    // happen since they follow the active workspace.
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
}

fn child(id: usize, parent: usize) -> TestWindowParams {
    let mut params = TestWindowParams::new(id);
    params.parent_id = Some(parent);
    params.is_floating = true;
    params
}

/// Everything that operations on windows could change.
fn full_state(layout: &Layout<TestWindow>) -> String {
    let mut lines = vec![summary(layout)];
    layout.with_windows(|win, output, ws_id, win_layout| {
        // Workspace ids differ between layouts, so use indices.
        let ws_idx = ws_id.map(|id| {
            layout
                .workspaces()
                .position(|(_, _, ws)| ws.id() == id)
                .unwrap()
        });
        lines.push(format!(
            "{} {:?} {ws_idx:?} {win_layout:?} {:?} {:?} {}",
            win.0.id,
            output.map(|output| output.name()),
            win.pending_sizing_mode(),
            win.requested_size(),
            win.is_pending_windowed_fullscreen(),
        ));
    });
    for tile in &layout.scratchpad.hidden {
        lines.push(format!(
            "{:?} {:?}",
            tile.floating_window_size, tile.floating_pos
        ));
    }
    for mon in layout.monitors() {
        lines.push(format!(
            "{} {}/{}",
            mon.output_name(),
            mon.active_workspace_idx,
            mon.workspaces.len()
        ));
    }
    lines.push(format!(
        "{:?} {}",
        layout.active_output().map(|output| output.name()),
        layout.interactive_move.is_some()
    ));
    lines.join("\n")
}

#[test]
fn hidden_windows_are_found() {
    let mut layout = check_ops([Op::AddOutput(1), window(1, 100), window(2, 100), hide(1)]);
    assert!(layout.has_window(&1));

    let windows: Vec<_> = layout
        .windows()
        .map(|(mon, win)| format!("{} {}", win.0.id, mon.is_some()))
        .collect();
    assert_snapshot!(windows.join(", "), @"2 true, 1 false");

    let mut found = Vec::new();
    layout.with_windows(|win, output, ws_id, win_layout| {
        if win.0.id == 1 {
            found.push((output.is_some(), ws_id, win_layout));
        }
    });
    let template = layout.scratchpad.hidden[0].ipc_layout_template();
    assert_eq!(found, [(false, None, template)]);

    let mut found = Vec::new();
    layout.with_windows_mut(|win, output| found.push(format!("{} {}", win.0.id, output.is_some())));
    assert_snapshot!(found.join(", "), @"2 true, 1 false");
}

#[test]
fn new_dialog_of_hidden_window_shows_it() {
    let setup = [Op::AddOutput(1), window(1, 100), window(2, 100), hide(1)];

    // A floating dialog opens above its parent.
    let mut ops = setup.to_vec();
    ops.push(Op::AddWindowNextTo {
        params: child(3, 1),
        next_to_id: 1,
    });
    let layout = check_ops(ops);
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [1], focus: Some(3)");
    let ws = layout.active_workspace().unwrap();
    let floating: Vec<_> = ws
        .floating()
        .tiles()
        .map(|tile| tile.window().0.id)
        .collect();
    assert_eq!(floating, [3, 1]);

    // A tiled one opens in the scrolling layout.
    let mut params = child(3, 1);
    params.is_floating = false;
    let mut ops = setup.to_vec();
    ops.push(Op::AddWindowNextTo {
        params,
        next_to_id: 1,
    });
    let layout = check_ops(ops);
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [1], focus: Some(3)");
    let ws = layout.active_workspace().unwrap();
    assert!(ws.has_window(&3) && !ws.is_floating(&3));

    // Neither takes focus if the dialog doesn't.
    let mut layout = check_ops(setup);
    layout.add_window(
        TestWindow::new(child(3, 1)),
        AddWindowTarget::NextTo(&1),
        None,
        None,
        false,
        true,
        ActivateWindow::No,
    );
    layout.verify_invariants();
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [1], focus: Some(2)");

    // Without outputs, the parent stays hidden.
    let mut layout = check_ops([
        Op::AddOutput(1),
        window(1, 100),
        hide(1),
        Op::RemoveOutput(1),
        Op::AddWindowNextTo {
            params: child(2, 1),
            next_to_id: 1,
        },
    ]);
    assert_snapshot!(summary(&layout), @"hidden: [1], shown: [], focus: None");
    assert!(layout.workspaces().any(|(_, _, ws)| ws.has_window(&2)));

    check_ops_on_layout(&mut layout, [Op::AddOutput(1)]);
    assert_snapshot!(summary(&layout), @"hidden: [1], shown: [], focus: Some(2)");
}

#[test]
fn hiding_with_visible_child_is_noop() {
    let mut layout = check_ops([
        Op::AddOutput(1),
        window(1, 100),
        Op::AddWindow {
            params: child(2, 1),
        },
    ]);
    assert_eq!(layout.move_to_scratchpad(Some(&1)), None);
    layout.verify_invariants();
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [], focus: Some(2)");

    // Once the child is hidden, the parent can be hidden too.
    check_ops_on_layout(&mut layout, [hide(2), hide(1)]);
    assert_snapshot!(summary(&layout), @"hidden: [2, 1], shown: [], focus: None");

    // Toggling a focused shown window with a visible child doesn't hide it either.
    let mut layout = check_ops([
        Op::AddOutput(1),
        window(1, 100),
        hide(1),
        toggle(),
        Op::AddWindow {
            params: child(2, 1),
        },
        Op::FocusWindow(1),
    ]);
    assert_eq!(
        layout.toggle_scratchpad(|_| true),
        ScratchpadToggle::Nothing
    );
    layout.verify_invariants();
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [1], focus: Some(1)");
}

#[test]
fn operations_on_hidden_window_are_ignored() {
    // The hidden window is also pending windowed fullscreen, which set_fullscreen() handles
    // separately.
    let setup = [
        Op::AddOutput(1),
        Op::AddOutput(2),
        Op::FocusOutput(1),
        window(1, 100),
        window(2, 100),
        Op::ToggleWindowedFullscreen(1),
        hide(1),
        Op::Communicate(1),
        Op::Communicate(2),
        Op::CompleteAnimations,
    ];
    let expected = full_state(&check_ops(setup.clone()));

    let ops = [
        Op::ConsumeOrExpelWindowLeft { id: Some(1) },
        Op::ConsumeOrExpelWindowRight { id: Some(1) },
        Op::CenterWindow { id: Some(1) },
        Op::SwitchPresetWindowWidth { id: Some(1) },
        Op::SwitchPresetWindowWidthBack { id: Some(1) },
        Op::SwitchPresetWindowHeight { id: Some(1) },
        Op::SwitchPresetWindowHeightBack { id: Some(1) },
        Op::SetWindowWidth {
            id: Some(1),
            change: SizeChange::SetFixed(300),
        },
        Op::SetWindowHeight {
            id: Some(1),
            change: SizeChange::SetFixed(300),
        },
        Op::ResetWindowHeight { id: Some(1) },
        Op::ToggleWindowFloating { id: Some(1) },
        Op::SetWindowFloating {
            id: Some(1),
            floating: false,
        },
        Op::SetWindowFloating {
            id: Some(1),
            floating: true,
        },
        Op::MoveFloatingWindow {
            id: Some(1),
            x: PositionChange::SetFixed(10.),
            y: PositionChange::SetFixed(10.),
            animate: false,
        },
        Op::InteractiveMoveBegin {
            window: 1,
            output_idx: 1,
            px: 0.,
            py: 0.,
        },
        Op::InteractiveResizeBegin {
            window: 1,
            edges: ResizeEdge::RIGHT,
        },
        Op::InteractiveResizeUpdate {
            window: 1,
            dx: 100.,
            dy: 100.,
        },
        Op::InteractiveResizeEnd { window: 1 },
        Op::FullscreenWindow(1),
        Op::SetFullscreenWindow {
            window: 1,
            is_fullscreen: true,
        },
        Op::SetFullscreenWindow {
            window: 1,
            is_fullscreen: false,
        },
        Op::ToggleWindowedFullscreen(1),
        Op::MaximizeWindowToEdges { id: Some(1) },
        Op::MoveWindowToWorkspace {
            window_id: Some(1),
            workspace_idx: 1,
        },
        Op::MoveWindowToOutput {
            window_id: Some(1),
            output_id: 2,
            target_ws_idx: None,
        },
    ];
    for op in ops {
        let mut all = setup.to_vec();
        all.push(op.clone());
        let layout = check_ops(all);
        assert_eq!(full_state(&layout), expected, "{op:?}");
    }

    // Methods without a corresponding op.
    let mut layout = check_ops(setup);
    assert_eq!(layout.popup_target_rect(&1, false), Rectangle::default());
    assert_eq!(layout.popup_target_rect(&1, true), Rectangle::default());
    assert_eq!(layout.scroll_amount_to_activate(&1), 0.);
    assert!(!layout.should_trigger_focus_follows_mouse_on(&1));
    let output = layout.outputs().next().unwrap().clone();
    assert!(!layout.interactive_move_begin(1, &output, Point::from((0., 0.))));
    assert!(!layout.interactive_resize_begin(1, ResizeEdge::RIGHT));
    layout.set_maximized(&1, true);
    layout.verify_invariants();
    assert_eq!(full_state(&layout), expected);
}

/// Window ids on each workspace of each output, with `*` marking the active workspace.
fn workspaces(layout: &Layout<TestWindow>) -> String {
    let MonitorSet::Normal { monitors, .. } = &layout.monitor_set else {
        return String::from("no outputs");
    };
    let mons: Vec<_> = monitors
        .iter()
        .map(|mon| {
            let wss: Vec<_> = mon
                .workspaces
                .iter()
                .enumerate()
                .map(|(idx, ws)| {
                    let mark = if idx == mon.active_workspace_idx {
                        "*"
                    } else {
                        ""
                    };
                    let ids: Vec<_> = ws.windows().map(|win| win.0.id).collect();
                    format!("{mark}{ids:?}")
                })
                .collect();
            format!("{}: {}", mon.output_name(), wss.join(" "))
        })
        .collect();
    mons.join("; ")
}

/// Index of the previous workspace on the active output.
fn previous_workspace_idx(layout: &Layout<TestWindow>) -> Option<usize> {
    let mon = layout.active_monitor_ref().unwrap();
    mon.idx_of_ws(mon.previous_workspace_id?)
}

/// Floating window ids on the active workspace, from top to bottom.
fn floating_order(layout: &Layout<TestWindow>) -> String {
    let ws = layout.active_workspace().unwrap();
    let ids: Vec<_> = ws
        .floating()
        .tiles()
        .map(|tile| tile.window().0.id)
        .collect();
    format!("{ids:?}")
}

#[test]
fn shown_window_follows_workspace_switch() {
    let setup = [
        Op::AddOutput(1),
        window(1, 100),
        floating_window(2),
        hide(2),
        toggle(),
    ];

    // Keyboard. This takes effect right away, without a refresh.
    let mut ops = setup.to_vec();
    ops.push(Op::FocusWorkspaceDown);
    let mut layout = check_ops(ops);
    assert_snapshot!(workspaces(&layout), @"output1: [1] *[2] []");
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [2], focus: Some(2)");

    // Following back empties the workspace, which gets cleaned up after the switch animation.
    check_ops_on_layout(&mut layout, [Op::FocusWorkspaceUp]);
    assert_snapshot!(workspaces(&layout), @"output1: *[1, 2] [] []");
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
    assert_snapshot!(workspaces(&layout), @"output1: *[1, 2] []");

    // Gesture. The window stays on the old workspace until the gesture ends.
    let mut ops = setup.to_vec();
    ops.extend([
        Op::WorkspaceSwitchGestureBegin {
            output_idx: 1,
            is_touchpad: true,
        },
        Op::WorkspaceSwitchGestureUpdate {
            delta: 300.,
            timestamp: Duration::ZERO,
            is_touchpad: true,
        },
    ]);
    let mut layout = check_ops(ops);
    assert_snapshot!(workspaces(&layout), @"output1: *[1, 2] []");
    check_ops_on_layout(
        &mut layout,
        [Op::WorkspaceSwitchGestureEnd {
            is_touchpad: Some(true),
        }],
    );
    assert_snapshot!(workspaces(&layout), @"output1: [1] *[2] []");
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [2], focus: Some(2)");

    // Activating a workspace by adding a window to it.
    let mut ops = setup.to_vec();
    ops.extend([window(3, 100), Op::MoveWindowToWorkspaceDown(true)]);
    let layout = check_ops(ops);
    assert_snapshot!(workspaces(&layout), @"output1: [1] *[3, 2] []");
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [2], focus: Some(3)");
}

#[test]
fn follow_adds_empty_workspaces() {
    // Following to the last empty workspace adds a new one.
    let mut layout = check_ops([
        Op::AddOutput(1),
        window(1, 100),
        floating_window(2),
        hide(2),
        toggle(),
        Op::FocusWorkspaceDown,
    ]);
    assert_snapshot!(workspaces(&layout), @"output1: [1] *[2] []");
    assert_eq!(previous_workspace_idx(&layout), Some(0));
    check_ops_on_layout(&mut layout, [Op::FocusWorkspacePrevious]);
    assert_snapshot!(workspaces(&layout), @"output1: *[1, 2] [] []");

    // Following to the first empty workspace adds a new one above it.
    let mut options = Options::default();
    options.layout.empty_workspace_above_first = true;
    let layout = check_ops_with_options(
        options,
        [
            Op::AddOutput(1),
            window(1, 100),
            floating_window(2),
            hide(2),
            toggle(),
            Op::FocusWorkspaceUp,
        ],
    );
    assert_snapshot!(workspaces(&layout), @"output1: [] *[2] [1] []");
    assert_eq!(previous_workspace_idx(&layout), Some(2));
}

#[test]
fn unfocused_shown_window_follows_without_focus() {
    // Tiled window focused on the target workspace.
    let layout = check_ops([
        Op::AddOutput(1),
        window(1, 100),
        Op::FocusWorkspaceDown,
        window(3, 100),
        Op::FocusWorkspaceUp,
        floating_window(2),
        hide(2),
        toggle(),
        Op::FocusTiling,
        Op::FocusWorkspaceDown,
    ]);
    assert_snapshot!(workspaces(&layout), @"output1: [1] *[3, 2] []");
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [2], focus: Some(3)");

    // Floating window focused on the target workspace.
    let layout = check_ops([
        Op::AddOutput(1),
        window(1, 100),
        Op::FocusWorkspaceDown,
        floating_window(3),
        Op::FocusWorkspaceUp,
        floating_window(2),
        hide(2),
        toggle(),
        Op::FocusTiling,
        Op::FocusWorkspaceDown,
    ]);
    assert_snapshot!(workspaces(&layout), @"output1: [1] *[2, 3] []");
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [2], focus: Some(3)");
}

#[test]
fn follow_keeps_stacking_order() {
    let mut layout = check_ops([
        Op::AddOutput(1),
        window(1, 100),
        floating_window(2),
        floating_window(3),
        floating_window(4),
        hide(2),
        hide(3),
        hide(4),
        Op::ToggleScratchpad { id: Some(2) },
        Op::ToggleScratchpad { id: Some(3) },
        Op::ToggleScratchpad { id: Some(4) },
        Op::FocusWindow(2),
    ]);
    assert_snapshot!(floating_order(&layout), @"[2, 4, 3]");

    check_ops_on_layout(&mut layout, [Op::FocusWorkspaceDown]);
    assert_snapshot!(workspaces(&layout), @"output1: [1] *[2, 4, 3] []");
    assert_snapshot!(floating_order(&layout), @"[2, 4, 3]");
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [2, 3, 4], focus: Some(2)");
}

#[test]
fn shown_window_follows_workspace_to_other_output() {
    // The output with the shown window is removed.
    let layout = check_ops([
        Op::AddOutput(1),
        Op::AddOutput(2),
        Op::FocusOutput(2),
        window(3, 100),
        Op::FocusOutput(1),
        window(1, 100),
        floating_window(2),
        hide(2),
        toggle(),
        Op::RemoveOutput(1),
    ]);
    assert_snapshot!(workspaces(&layout), @"output2: *[3, 2] [1] []");
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [2], focus: Some(2)");

    // The emptied workspace is cleaned up right away since there's no workspace switch.
    let layout = check_ops([
        Op::AddOutput(1),
        Op::AddOutput(2),
        Op::FocusOutput(2),
        window(3, 100),
        Op::FocusOutput(1),
        floating_window(2),
        hide(2),
        toggle(),
        Op::RemoveOutput(1),
    ]);
    assert_snapshot!(workspaces(&layout), @"output2: *[3, 2] []");
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [2], focus: Some(2)");

    // The workspace with the shown window goes back to its original output when it's added, but
    // the shown window stays on its output.
    let mut layout = check_ops([
        Op::AddOutput(1),
        window(1, 100),
        Op::AddOutput(2),
        Op::FocusOutput(2),
        window(3, 100),
        Op::FocusWorkspaceDown,
        window(4, 100),
        Op::FocusWorkspaceUp,
        floating_window(2),
        hide(2),
        Op::RemoveOutput(2),
        Op::FocusWorkspace(2),
        toggle(),
    ]);
    assert_snapshot!(workspaces(&layout), @"output1: [1] [3] *[4, 2] []");
    check_ops_on_layout(&mut layout, [Op::AddOutput(2)]);
    assert_snapshot!(workspaces(&layout), @"output1: *[1, 2] []; output2: *[3] [4] []");
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [2], focus: Some(2)");

    // The workspace with the shown window is moved to another output, but the shown window stays
    // on its output.
    let mut layout = check_ops([
        Op::AddOutput(1),
        Op::AddOutput(2),
        window(1, 100),
        floating_window(2),
        hide(2),
        toggle(),
        Op::FocusOutput(2),
    ]);
    let output1 = layout.outputs().find(|o| o.name() == "output1").cloned();
    let output2 = layout.outputs().find(|o| o.name() == "output2").cloned();
    assert!(!layout.move_workspace_to_output_by_id(0, output1, &output2.unwrap()));
    layout.verify_invariants();
    assert_snapshot!(workspaces(&layout), @"output1: *[2] []; output2: [1] *[]");
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [2], focus: None");
}

#[test]
fn activated_window_keeps_focus_when_shown_window_follows() {
    let setup = [
        Op::AddOutput(1),
        window(1, 100),
        Op::FocusWorkspaceDown,
        window(3, 100),
        Op::FocusWorkspaceUp,
        floating_window(2),
        hide(2),
        toggle(),
    ];

    // A plain workspace switch keeps the focus on the shown window.
    let mut ops = setup.to_vec();
    ops.push(Op::FocusWorkspaceDown);
    let layout = check_ops(ops);
    assert_snapshot!(workspaces(&layout), @"output1: [1] *[3, 2] []");
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [2], focus: Some(2)");

    // Activating a window on another workspace.
    let mut ops = setup.to_vec();
    ops.push(Op::FocusWindow(3));
    let layout = check_ops(ops);
    assert_snapshot!(workspaces(&layout), @"output1: [1] *[3, 2] []");
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [2], focus: Some(3)");

    let mut layout = check_ops(setup.clone());
    layout.activate_window_without_raising(&3);
    layout.verify_invariants();
    assert_snapshot!(workspaces(&layout), @"output1: [1] *[3, 2] []");
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [2], focus: Some(3)");

    // Moving the last tiled window or column to another workspace, which leaves the shown window
    // focused on the source workspace before it follows.
    let setup = [
        Op::AddOutput(1),
        window(1, 100),
        floating_window(2),
        hide(2),
        toggle(),
        Op::FocusTiling,
    ];
    for op in [
        Op::MoveWindowToWorkspaceDown(true),
        Op::MoveColumnToWorkspaceDown(true),
    ] {
        let mut ops = setup.to_vec();
        ops.push(op.clone());
        let layout = check_ops(ops);
        assert_eq!(workspaces(&layout), "output1: [] *[1, 2] []", "{op:?}");
        assert_eq!(
            summary(&layout),
            "hidden: [], shown: [2], focus: Some(1)",
            "{op:?}"
        );
    }
}

fn linear_switch_options() -> Options {
    use niri_config::animations::{Curve, EasingParams, Kind};

    const LINEAR: Kind = Kind::Easing(EasingParams {
        duration_ms: 1000,
        curve: Curve::Linear,
    });

    let mut options = Options::default();
    options.animations.workspace_switch.0.kind = LINEAR;
    options.animations.overview_open_close.0.kind = LINEAR;
    options
}

/// A tiled window 1 and a shown scratchpad window 2 at 100, 50 with size 100 × 200.
fn shown_window_at_100_50() -> Vec<Op> {
    vec![
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
        hide(2),
        toggle(),
        Op::Communicate(2),
        Op::CompleteAnimations,
    ]
}

fn output1(layout: &Layout<TestWindow>) -> Output {
    layout
        .outputs()
        .find(|o| o.name() == "output1")
        .unwrap()
        .clone()
}

fn hit(layout: &Layout<TestWindow>, x: f64, y: f64) -> String {
    let output = output1(layout);
    match layout.window_under(&output, Point::from((x, y))) {
        None => String::from("none"),
        Some((win, HitType::Input { win_pos })) => {
            format!("{} input at {} {}", win.0.id, win_pos.x, win_pos.y)
        }
        Some((win, HitType::Activate { .. })) => format!("{} activate", win.0.id),
    }
}

/// Position of the window on the output, as it is rendered.
fn visual_pos(layout: &Layout<TestWindow>, id: usize) -> Point<f64, Logical> {
    let mon = layout.monitors().next().unwrap();

    if let Some(pin_geo) = mon.scratchpad_pin_geo() {
        let ws = mon.active_workspace_ref();
        if let Some((_, pos)) = ws
            .scratchpad_tiles_with_render_positions()
            .find(|(tile, _)| tile.window().0.id == id)
        {
            return pin_geo.loc + pos;
        }
    }

    let zoom = mon.overview_zoom();
    mon.workspaces_with_render_geo_cull(false)
        .find_map(|(ws, geo)| {
            ws.tiles_with_render_positions()
                .find(|(tile, _, _)| tile.window().0.id == id)
                .map(|(_, pos, _)| geo.loc + pos.upscale(zoom))
        })
        .unwrap()
}

#[track_caller]
fn assert_close(a: Point<f64, Logical>, b: Point<f64, Logical>, tolerance: f64) {
    let diff = a - b;
    assert!(
        diff.x.abs() <= tolerance && diff.y.abs() <= tolerance,
        "{a:?} is not within {tolerance} of {b:?}"
    );
}

#[test]
fn shown_window_is_pinned_during_workspace_switch() {
    let mut layout = check_ops_with_options(linear_switch_options(), shown_window_at_100_50());
    assert_snapshot!(hit(&layout, 150., 150.), @"2 activate");

    // Halfway through the switch, the new workspace is at y = 396, but the window stays in place.
    check_ops_on_layout(
        &mut layout,
        [
            Op::FocusWorkspaceDown,
            Op::AdvanceAnimations { msec_delta: 500 },
        ],
    );
    assert_snapshot!(workspaces(&layout), @"output1: [1] *[2] []");
    assert_snapshot!(hit(&layout, 150., 150.), @"2 activate");
    // Where the window would be if it moved together with its workspace.
    assert_snapshot!(hit(&layout, 150., 546.), @"none");

    let output = output1(&layout);
    assert_eq!(
        layout.resize_edges_under(&output, Point::from((105., 55.))),
        Some(ResizeEdge::TOP | ResizeEdge::LEFT)
    );
    assert_eq!(
        layout.resize_edges_under(&output, Point::from((105., 451.))),
        None
    );

    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
    assert_snapshot!(hit(&layout, 150., 150.), @"2 activate");
}

#[test]
fn shown_window_is_pinned_during_workspace_switch_gesture() {
    let mut ops = shown_window_at_100_50();
    ops.extend([
        Op::WorkspaceSwitchGestureBegin {
            output_idx: 1,
            is_touchpad: false,
        },
        Op::WorkspaceSwitchGestureUpdate {
            delta: 396.,
            timestamp: Duration::ZERO,
            is_touchpad: false,
        },
    ]);
    let mut layout = check_ops_with_options(linear_switch_options(), ops);
    assert_snapshot!(workspaces(&layout), @"output1: *[1, 2] []");
    assert_snapshot!(hit(&layout, 150., 150.), @"2 activate");

    // The old workspace with the window goes entirely off-screen.
    check_ops_on_layout(
        &mut layout,
        [Op::WorkspaceSwitchGestureUpdate {
            delta: 396.,
            timestamp: Duration::ZERO,
            is_touchpad: false,
        }],
    );
    assert_snapshot!(hit(&layout, 150., 150.), @"2 activate");

    check_ops_on_layout(
        &mut layout,
        [
            Op::WorkspaceSwitchGestureEnd {
                is_touchpad: Some(false),
            },
            Op::CompleteAnimations,
        ],
    );
    assert_snapshot!(workspaces(&layout), @"output1: [1] *[2] []");
    assert_snapshot!(hit(&layout, 150., 150.), @"2 activate");
}

#[test]
fn pin_geometry_matches_workspace_at_rest() {
    let mut ops = shown_window_at_100_50();
    ops[0] = Op::AddScaledOutput {
        id: 1,
        scale: 1.5,
        layout_config: None,
    };
    let mut layout = check_ops_with_options(linear_switch_options(), ops);

    let mon = layout.monitors().next().unwrap();
    assert_eq!(mon.scratchpad_pin_geo(), None);
    let rest = mon.workspaces_render_geo().next().unwrap();

    check_ops_on_layout(
        &mut layout,
        [
            Op::FocusWorkspaceDown,
            Op::AdvanceAnimations { msec_delta: 300 },
        ],
    );
    let mon = layout.monitors().next().unwrap();
    assert_eq!(mon.scratchpad_pin_geo(), Some(rest));

    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
    let mon = layout.monitors().next().unwrap();
    assert_eq!(mon.scratchpad_pin_geo(), None);
    assert_eq!(mon.workspaces_render_geo().nth(1).unwrap(), rest);
}

#[test]
fn pinned_window_hidden_behind_fullscreen_is_not_hit() {
    let mut layout = check_ops_with_options(
        linear_switch_options(),
        [
            Op::AddOutput(1),
            window(1, 100),
            Op::FocusWorkspaceDown,
            window(3, 100),
            Op::FullscreenWindow(3),
            Op::Communicate(3),
            Op::FocusWorkspaceUp,
            floating_window(2),
            Op::Communicate(2),
            Op::MoveFloatingWindow {
                id: Some(2),
                x: PositionChange::SetFixed(100.),
                y: PositionChange::SetFixed(50.),
                animate: false,
            },
            hide(2),
            toggle(),
            Op::Communicate(2),
            Op::FocusTiling,
            Op::CompleteAnimations,
        ],
    );
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [2], focus: Some(1)");

    // The unfocused window follows to a workspace with a focused fullscreen window, where floating
    // windows aren't visible. At 90% of the switch, that workspace is at y = 79.
    check_ops_on_layout(
        &mut layout,
        [
            Op::FocusWorkspaceDown,
            Op::AdvanceAnimations { msec_delta: 900 },
        ],
    );
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [2], focus: Some(3)");
    assert_snapshot!(hit(&layout, 150., 150.), @"3 activate");
}

#[test]
fn interactive_move_of_pinned_window_starts_at_pinned_position() {
    let mut ops = shown_window_at_100_50();
    ops.extend([
        Op::FocusWorkspaceDown,
        Op::AdvanceAnimations { msec_delta: 500 },
        Op::InteractiveMoveBegin {
            window: 2,
            output_idx: 1,
            px: 150.,
            py: 150.,
        },
    ]);
    let layout = check_ops_with_options(linear_switch_options(), ops);

    let Some(InteractiveMoveState::Starting {
        pointer_ratio_within_window,
        ..
    }) = &layout.interactive_move
    else {
        panic!("interactive move must have started");
    };
    assert_eq!(*pointer_ratio_within_window, (0.5, 0.5));
}

#[test]
fn dragged_pinned_window_starts_from_pinned_position() {
    let mut ops = shown_window_at_100_50();
    ops.extend([
        Op::FocusWorkspaceDown,
        Op::AdvanceAnimations { msec_delta: 500 },
        Op::InteractiveMoveBegin {
            window: 2,
            output_idx: 1,
            px: 150.,
            py: 150.,
        },
        Op::InteractiveMoveUpdate {
            window: 2,
            dx: 100.,
            dy: 100.,
            output_idx: 1,
            px: 250.,
            py: 250.,
        },
    ]);
    let layout = check_ops_with_options(linear_switch_options(), ops);

    // The window then animates from where it was seen towards the pointer, rather than from where
    // its workspace is. Before the move started, it was pulled 19 px towards the pointer from its
    // pinned position at 100, 50.
    let Some(InteractiveMoveState::Moving(move_)) = &layout.interactive_move else {
        panic!("interactive move must be moving");
    };
    assert_close(move_.tile_render_location(1.), Point::from((119., 69.)), 1.);
}

#[test]
fn following_window_keeps_screen_position() {
    let mut ops = shown_window_at_100_50();
    ops.extend([Op::FocusWorkspaceDown, Op::CompleteAnimations]);
    let layout = check_ops(ops);
    assert_snapshot!(workspaces(&layout), @"output1: [1] *[2] []");
    assert_snapshot!(floating_geometry(&layout, 2), @"pos: 100 50, size: 100 200");
}

#[test]
fn following_window_slides_in_overview() {
    let mut ops = shown_window_at_100_50();
    ops.extend([Op::ToggleOverview, Op::CompleteAnimations]);
    let mut layout = check_ops_with_options(linear_switch_options(), ops);
    let before = visual_pos(&layout, 2);

    check_ops_on_layout(&mut layout, [Op::FocusWorkspaceDown]);
    assert_snapshot!(workspaces(&layout), @"output1: [1] *[2] []");
    assert_close(visual_pos(&layout, 2), before, 1.);

    // The window moves together with the workspace switch, so it stays in place.
    check_ops_on_layout(&mut layout, [Op::AdvanceAnimations { msec_delta: 500 }]);
    let mon = layout.monitors().next().unwrap();
    let tile = mon
        .active_workspace_ref()
        .floating()
        .tiles()
        .next()
        .unwrap();
    assert!(tile.is_moving_between_workspaces());
    assert_close(visual_pos(&layout, 2), before, 1.5);

    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
    assert_close(visual_pos(&layout, 2), before, 1.);

    // Not pinned in the overview, and only activation hits.
    let mon = layout.monitors().next().unwrap();
    assert_eq!(mon.scratchpad_pin_geo(), None);
    let zoom = mon.overview_zoom();
    let center = before + Point::from((50., 100.)).upscale(zoom);
    assert_snapshot!(hit(&layout, center.x, center.y), @"2 activate");
}

#[test]
fn pinned_window_is_handed_over_to_overview() {
    // The overview opens during a workspace switch.
    let mut ops = shown_window_at_100_50();
    ops.extend([
        Op::FocusWorkspaceDown,
        Op::AdvanceAnimations { msec_delta: 300 },
    ]);
    let mut layout = check_ops_with_options(linear_switch_options(), ops);
    let before = visual_pos(&layout, 2);
    assert_close(before, Point::from((100., 50.)), 0.);

    check_ops_on_layout(&mut layout, [Op::ToggleOverview]);
    let mon = layout.monitors().next().unwrap();
    assert_eq!(mon.scratchpad_pin_geo(), None);
    assert_close(visual_pos(&layout, 2), before, 1.);

    // The overview finishes closing during a workspace switch.
    let mut ops = shown_window_at_100_50();
    ops.extend([
        Op::ToggleOverview,
        Op::CompleteAnimations,
        Op::ToggleOverview,
        Op::AdvanceAnimations { msec_delta: 500 },
        Op::FocusWorkspaceDown,
        Op::AdvanceAnimations { msec_delta: 499 },
    ]);
    let mut layout = check_ops_with_options(linear_switch_options(), ops);
    let mon = layout.monitors().next().unwrap();
    assert_eq!(mon.scratchpad_pin_geo(), None);
    let before = visual_pos(&layout, 2);

    check_ops_on_layout(&mut layout, [Op::AdvanceAnimations { msec_delta: 1 }]);
    let mon = layout.monitors().next().unwrap();
    assert!(mon.scratchpad_pin_geo().is_some());
    assert_close(visual_pos(&layout, 2), before, 3.);
}

#[test]
fn shown_window_follows_after_moving_workspace_to_first() {
    let mut options = Options::default();
    options.layout.empty_workspace_above_first = true;
    let layout = check_ops_with_options(
        options,
        [
            window(5, 100),
            Op::AddOutput(1),
            hide(5),
            toggle(),
            Op::MoveWorkspaceToIndex {
                ws_name: None,
                target_idx: 0,
            },
        ],
    );
    assert_snapshot!(workspaces(&layout), @"output1: [] *[5] []");
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [5], focus: Some(5)");
}

/// Options with a linear window opening animation of 100 ms, so that its progress after `t` ms is
/// `t / 100` when going from 0 to 1.
fn linear_open_options() -> Options {
    use niri_config::animations::{Curve, EasingParams, Kind};

    let mut options = Options::default();
    options.animations.window_open.anim.kind = Kind::Easing(EasingParams {
        duration_ms: 100,
        curve: Curve::Linear,
    });
    options
}

/// Windows animating out on all workspaces, with their generation and progress.
fn hiding(layout: &Layout<TestWindow>) -> String {
    layout
        .workspaces()
        .flat_map(|(_, _, ws)| ws.scratchpad_hiding())
        .map(|hiding| {
            format!(
                "{} gen {} at {:.2}",
                hiding.id,
                hiding.gen,
                hiding.progress()
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Progress of the open animation of a window on a workspace.
fn open_progress(layout: &Layout<TestWindow>, id: usize) -> String {
    let tile = layout
        .workspaces()
        .flat_map(|(_, _, ws)| ws.tiles())
        .find(|tile| tile.window().0.id == id)
        .unwrap();
    match tile.open_animation_progress() {
        Some(progress) => format!("{progress:.2}"),
        None => String::from("none"),
    }
}

fn advance(msec_delta: i32) -> Op {
    Op::AdvanceAnimations { msec_delta }
}

#[test]
fn hiding_on_active_workspace_animates() {
    let mut layout = check_ops_with_options(
        linear_open_options(),
        [Op::AddOutput(1), floating_window(1), hide(1)],
    );
    assert_snapshot!(hiding(&layout), @"1 gen 0 at 1.00");
    let output = layout.hiding_window_output(&1).map(|output| output.name());
    assert_snapshot!(format!("{output:?}"), @r#"Some("output1")"#);
    assert!(layout.are_animations_ongoing(None));

    // The open animation backwards: 1 - 40 / 100.
    check_ops_on_layout(&mut layout, [advance(40)]);
    assert_snapshot!(hiding(&layout), @"1 gen 0 at 0.60");

    check_ops_on_layout(&mut layout, [advance(60)]);
    assert_snapshot!(hiding(&layout), @"");
    assert_eq!(layout.hiding_window_output(&1), None);
}

#[test]
fn hiding_pinned_window_stays_pinned() {
    let pinned = |layout: &Layout<TestWindow>| {
        let hiding: Vec<_> = layout
            .workspaces()
            .flat_map(|(_, _, ws)| ws.scratchpad_hiding())
            .map(|hiding| hiding.pinned)
            .collect();
        format!("{hiding:?}")
    };

    // During a workspace switch, it animates out where it was pinned.
    let mut options = linear_switch_options();
    options.animations.window_open = linear_open_options().animations.window_open;
    let mut ops = shown_window_at_100_50();
    ops.extend([
        Op::FocusWorkspaceDown,
        Op::AdvanceAnimations { msec_delta: 500 },
        hide(2),
    ]);
    let mut layout = check_ops_with_options(options, ops);
    assert_snapshot!(pinned(&layout), @"[true]");

    // At rest too, so that it stays on the screen if a workspace switch starts.
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations, toggle(), hide(2)]);
    assert_snapshot!(pinned(&layout), @"[true]");

    // A window that wasn't in the scratchpad isn't pinned.
    check_ops_on_layout(
        &mut layout,
        [Op::CompleteAnimations, window(3, 100), hide(3)],
    );
    assert_snapshot!(pinned(&layout), @"[false]");
}

/// Workspaces with windows animating out to the scratchpad, by workspace index.
fn hiding_workspaces(layout: &Layout<TestWindow>) -> String {
    let mon = layout.monitors().next().unwrap();
    let hiding: Vec<_> = mon
        .workspaces
        .iter()
        .enumerate()
        .flat_map(|(idx, ws)| ws.scratchpad_hiding().iter().map(move |h| (h.id, idx)))
        .collect();
    format!("{hiding:?} active {}", mon.active_workspace_idx())
}

#[test]
fn hiding_pinned_window_follows_active_workspace() {
    let mut options = linear_switch_options();
    options.animations.window_open = linear_open_options().animations.window_open;

    // Switching again while it animates out.
    let mut ops = shown_window_at_100_50();
    ops.extend([
        Op::FocusWorkspaceDown,
        Op::AdvanceAnimations { msec_delta: 500 },
        hide(2),
    ]);
    let mut layout = check_ops_with_options(options.clone(), ops);
    assert_snapshot!(hiding_workspaces(&layout), @"[(2, 1)] active 1");
    check_ops_on_layout(&mut layout, [Op::FocusWorkspaceUp]);
    assert_snapshot!(hiding_workspaces(&layout), @"[(2, 0)] active 0");

    // Hiding at rest, then switching.
    let mut ops = shown_window_at_100_50();
    ops.extend([hide(2), Op::FocusWorkspaceDown]);
    let layout = check_ops_with_options(options.clone(), ops);
    assert_snapshot!(hiding_workspaces(&layout), @"[(2, 1)] active 1");

    // A gesture keeps the window where it was until it ends.
    let mut ops = shown_window_at_100_50();
    ops.extend([
        Op::WorkspaceSwitchGestureBegin {
            output_idx: 1,
            is_touchpad: true,
        },
        hide(2),
        Op::WorkspaceSwitchGestureUpdate {
            delta: 1000.,
            timestamp: Duration::from_millis(10),
            is_touchpad: true,
        },
    ]);
    let mut layout = check_ops_with_options(options.clone(), ops);
    assert_snapshot!(hiding_workspaces(&layout), @"[(2, 0)] active 0");
    check_ops_on_layout(
        &mut layout,
        [Op::WorkspaceSwitchGestureEnd {
            is_touchpad: Some(true),
        }],
    );
    assert_snapshot!(hiding_workspaces(&layout), @"[(2, 1)] active 1");

    // A window that wasn't in the scratchpad stays on its workspace.
    let layout = check_ops_with_options(
        options.clone(),
        [
            Op::AddOutput(1),
            window(1, 100),
            hide(1),
            Op::FocusWorkspaceDown,
        ],
    );
    assert_snapshot!(hiding_workspaces(&layout), @"[(1, 0)] active 1");

    // In the overview, it stays on its workspace rather than jumping to another one.
    let mut ops = shown_window_at_100_50();
    ops.extend([
        Op::ToggleOverview,
        Op::CompleteAnimations,
        hide(2),
        Op::FocusWorkspaceDown,
    ]);
    let layout = check_ops_with_options(options, ops);
    assert_snapshot!(hiding_workspaces(&layout), @"[(2, 0)] active 1");
}

#[test]
fn hidden_window_gets_new_config() {
    let mut layout = check_ops([Op::AddOutput(1), floating_window(1), hide(1)]);

    let mut options = Options::default();
    options.animations.window_open.anim.off = true;
    options.layout.border.off = false;
    options.layout.border.width = 7.;
    layout.update_options(options);

    let tile = layout
        .scratchpad
        .hidden
        .iter()
        .find(|tile| tile.window().0.id == 1)
        .unwrap();
    assert!(tile.options().animations.window_open.anim.off);
    assert_eq!(tile.options().layout.border.width, 7.);
    layout.verify_invariants();
}

#[test]
fn hiding_without_animation() {
    // On an inactive workspace with --id.
    let layout = check_ops_with_options(
        linear_open_options(),
        [
            Op::AddOutput(1),
            floating_window(1),
            Op::FocusWorkspaceDown,
            window(2, 100),
            hide(1),
        ],
    );
    assert_snapshot!(summary(&layout), @"hidden: [1], shown: [], focus: Some(2)");
    assert_snapshot!(hiding(&layout), @"");

    // With the window opening animation off.
    let mut options = linear_open_options();
    options.animations.window_open.anim.off = true;
    let layout = check_ops_with_options(options, [Op::AddOutput(1), floating_window(1), hide(1)]);
    assert_snapshot!(summary(&layout), @"hidden: [1], shown: [], focus: None");
    assert_snapshot!(hiding(&layout), @"");

    // With no outputs.
    let layout = check_ops_with_options(
        linear_open_options(),
        [
            Op::AddOutput(1),
            floating_window(1),
            Op::RemoveOutput(1),
            hide(1),
        ],
    );
    assert_snapshot!(summary(&layout), @"hidden: [1], shown: [], focus: None");
    assert_snapshot!(hiding(&layout), @"");

    // An inactive tab.
    let mut layout = check_ops_with_options(
        linear_open_options(),
        [
            Op::AddOutput(1),
            window(1, 100),
            window(2, 100),
            Op::ConsumeOrExpelWindowLeft { id: None },
            Op::ToggleColumnTabbedDisplay,
            hide(1),
        ],
    );
    assert_snapshot!(summary(&layout), @"hidden: [1], shown: [], focus: Some(2)");
    assert_snapshot!(hiding(&layout), @"");

    // The active tab animates. The generation of the first hiding was used up.
    check_ops_on_layout(&mut layout, [hide(2)]);
    assert_snapshot!(hiding(&layout), @"2 gen 1 at 1.00");

    // A floating window behind a focused fullscreen window.
    let layout = check_ops_with_options(
        linear_open_options(),
        [
            Op::AddOutput(1),
            floating_window(1),
            window(2, 100),
            Op::FullscreenWindow(2),
            Op::Communicate(2),
            // The floating window shows while the view moves.
            Op::CompleteAnimations,
            hide(1),
        ],
    );
    assert_snapshot!(summary(&layout), @"hidden: [1], shown: [], focus: Some(2)");
    assert_snapshot!(hiding(&layout), @"");
}

#[test]
fn showing_while_hiding_continues_from_progress() {
    let setup = [Op::AddOutput(1), floating_window(1), hide(1), advance(40)];

    let mut layout = check_ops_with_options(linear_open_options(), setup.clone());
    assert_snapshot!(hiding(&layout), @"1 gen 0 at 0.60");
    check_ops_on_layout(&mut layout, [toggle()]);
    assert_snapshot!(hiding(&layout), @"");
    assert_snapshot!(open_progress(&layout, 1), @"0.60");

    // From 0.6 to 1 in 100 ms: 0.6 + 0.4 * 20 / 100.
    check_ops_on_layout(&mut layout, [advance(20)]);
    assert_snapshot!(open_progress(&layout, 1), @"0.68");

    // Activating goes through the same path.
    let mut layout = check_ops_with_options(linear_open_options(), setup);
    check_ops_on_layout(&mut layout, [Op::FocusWindow(1)]);
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [1], focus: Some(1)");
    assert_snapshot!(hiding(&layout), @"");
    assert_snapshot!(open_progress(&layout, 1), @"0.60");
}

#[test]
fn hiding_while_opening_starts_from_progress() {
    let layout = check_ops_with_options(
        linear_open_options(),
        [
            Op::AddOutput(1),
            floating_window(1),
            hide(1),
            Op::CompleteAnimations,
            toggle(),
            advance(30),
            hide(1),
        ],
    );
    assert_snapshot!(hiding(&layout), @"1 gen 1 at 0.30");
}

#[test]
fn hiding_windows_are_released() {
    // The window is closed.
    let layout = check_ops_with_options(
        linear_open_options(),
        [
            Op::AddOutput(1),
            floating_window(1),
            hide(1),
            Op::CloseWindow(1),
        ],
    );
    assert_snapshot!(hiding(&layout), @"");

    // The output is removed and the workspace moves to another output.
    let layout = check_ops_with_options(
        linear_open_options(),
        [
            Op::AddOutput(1),
            Op::AddOutput(2),
            Op::FocusOutput(1),
            window(2, 100),
            floating_window(1),
            hide(1),
            Op::RemoveOutput(1),
        ],
    );
    assert_snapshot!(workspaces(&layout), @"output2: [2] *[]");
    assert_snapshot!(hiding(&layout), @"");

    // The workspace is cleaned up after switching away from it. The window opening animation is
    // slower than the workspace switch here.
    let mut options = linear_switch_options();
    options.animations.window_open.anim = options.animations.workspace_switch.0;
    options.animations.workspace_switch.0.kind =
        niri_config::animations::Kind::Easing(niri_config::animations::EasingParams {
            duration_ms: 100,
            curve: niri_config::animations::Curve::Linear,
        });
    let mut layout = check_ops_with_options(
        options,
        [
            Op::AddOutput(1),
            floating_window(1),
            hide(1),
            Op::FocusWorkspaceDown,
            advance(50),
        ],
    );
    assert_snapshot!(hiding(&layout), @"1 gen 0 at 0.95");
    check_ops_on_layout(&mut layout, [advance(50)]);
    assert_snapshot!(workspaces(&layout), @"output1: *[]");
    assert_snapshot!(hiding(&layout), @"");

    // The workspace is moved to another output.
    let layout = check_ops_with_options(
        linear_open_options(),
        [
            Op::AddOutput(1),
            Op::AddOutput(2),
            Op::FocusOutput(1),
            window(2, 100),
            floating_window(1),
            hide(1),
            Op::MoveWorkspaceToOutput(2),
        ],
    );
    assert_snapshot!(workspaces(&layout), @"output1: *[]; output2: *[2] []");
    assert_snapshot!(hiding(&layout), @"");

    // The workspace goes back to its original output when it's added.
    let layout = check_ops_with_options(
        linear_open_options(),
        [
            Op::AddOutput(1),
            window(1, 100),
            Op::AddOutput(2),
            Op::FocusOutput(2),
            window(3, 100),
            floating_window(2),
            Op::RemoveOutput(2),
            Op::FocusWorkspace(1),
            hide(2),
            Op::AddOutput(2),
        ],
    );
    assert_snapshot!(workspaces(&layout), @"output1: *[1] []; output2: *[3] []");
    assert_snapshot!(hiding(&layout), @"");
}

#[test]
fn hide_show_hide_has_one_hiding_window() {
    let layout = check_ops_with_options(
        linear_open_options(),
        [
            Op::AddOutput(1),
            floating_window(1),
            hide(1),
            advance(40),
            toggle(),
            // From 0.6 to 1: 0.6 + 0.4 * 50 / 100.
            advance(50),
            hide(1),
        ],
    );
    assert_snapshot!(hiding(&layout), @"1 gen 1 at 0.80");
}

#[test]
fn shown_window_brought_over_animates() {
    let mut layout = check_ops_with_options(
        linear_open_options(),
        [
            Op::AddOutput(1),
            Op::AddOutput(2),
            Op::FocusOutput(1),
            window(1, 100),
            floating_window(2),
            hide(2),
            toggle(),
            Op::CompleteAnimations,
            Op::FocusOutput(2),
        ],
    );
    assert_snapshot!(open_progress(&layout, 2), @"none");

    assert_eq!(
        layout.toggle_scratchpad(|_| true),
        ScratchpadToggle::Shown(2)
    );
    layout.verify_invariants();
    assert_snapshot!(open_progress(&layout, 2), @"0.00");
    assert_snapshot!(hiding(&layout), @"");

    check_ops_on_layout(&mut layout, [advance(25)]);
    assert_snapshot!(open_progress(&layout, 2), @"0.25");
}

fn add_hidden(id: usize, show: Option<bool>) -> Op {
    Op::AddWindowToScratchpad {
        params: TestWindowParams::new(id),
        show,
    }
}

/// Render positions of the tiles on the active workspace.
fn positions(layout: &Layout<TestWindow>) -> String {
    let ws = layout.active_workspace().unwrap();
    let mut tiles: Vec<_> = ws
        .tiles_with_render_positions()
        .map(|(tile, pos, _)| format!("{}: {},{}", tile.window().0.id, pos.x, pos.y))
        .collect();
    tiles.sort();
    tiles.join("; ")
}

#[test]
fn new_window_hides_without_trace() {
    let mut layout = check_ops([
        Op::AddOutput(1),
        window(1, 100),
        window(2, 100),
        Op::CompleteAnimations,
    ]);
    let before = positions(&layout);
    // Columns at x = 16 and 16 + 100 + 16, below the 16 px gap.
    assert_snapshot!(before, @"1: 16,16; 2: 132,16");
    assert!(!layout.are_animations_ongoing(None));

    check_ops_on_layout(&mut layout, [add_hidden(3, None)]);
    assert_snapshot!(summary(&layout), @"hidden: [3], shown: [], focus: Some(2)");
    assert_eq!(state(&layout, 3), ScratchpadState::Hidden);

    // The window was never visible, so there is nothing to animate out, and the others stay put.
    assert_eq!(hiding(&layout), "");
    assert!(layout.hiding_window_output(&3).is_none());
    assert_eq!(positions(&layout), before);
    assert!(!layout.are_animations_ongoing(None));

    // Contrast: hiding a window that is just as new through the regular path animates it out.
    check_ops_on_layout(&mut layout, [window(4, 100), hide(4)]);
    assert_snapshot!(hiding(&layout), @"4 gen 0 at 1.00");
}

#[test]
fn new_window_shown_in_scratchpad() {
    let layout = check_ops([Op::AddOutput(1), window(1, 100), add_hidden(2, Some(true))]);
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [2], focus: Some(2)");
    assert!(layout.active_workspace().unwrap().is_floating(&2));
    assert_eq!(hiding(&layout), "");

    let layout = check_ops([Op::AddOutput(1), window(1, 100), add_hidden(2, Some(false))]);
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [2], focus: Some(1)");
    assert!(layout.active_workspace().unwrap().is_floating(&2));

    // Without outputs, the window stays hidden until there is somewhere to show it.
    let mut layout = check_ops([add_hidden(1, Some(true))]);
    assert_snapshot!(summary(&layout), @"hidden: [1], shown: [], focus: None");
    check_ops_on_layout(&mut layout, [Op::AddOutput(1), toggle()]);
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [1], focus: Some(1)");
}

#[test]
fn new_floating_window_keeps_geometry_in_scratchpad() {
    let mut params = TestWindowParams::new(2);
    params.is_floating = true;

    let reference = check_ops([
        Op::AddOutput(1),
        window(1, 100),
        Op::AddWindow {
            params: params.clone(),
        },
        Op::Communicate(2),
        Op::CompleteAnimations,
    ]);
    let before = floating_geometry(&reference, 2);
    assert_snapshot!(before, @"pos: 590 260, size: 100 200");

    let layout = check_ops([
        Op::AddOutput(1),
        window(1, 100),
        Op::AddWindowToScratchpad { params, show: None },
        toggle(),
        Op::Communicate(2),
        Op::CompleteAnimations,
    ]);
    assert_eq!(floating_geometry(&layout, 2), before);
}

#[test]
fn new_window_hidden_on_empty_workspace() {
    // Like opening and closing a window on the last workspace: the new empty workspace stays
    // until the active one is left.
    let mut layout = check_ops([Op::AddOutput(1), add_hidden(1, None)]);
    assert_snapshot!(workspaces(&layout), @"output1: *[] []");

    // The empty workspace is cleaned up once the switch animation is over.
    check_ops_on_layout(&mut layout, [Op::FocusWorkspaceDown]);
    assert_snapshot!(workspaces(&layout), @"output1: [] *[]");
    check_ops_on_layout(&mut layout, [Op::CompleteAnimations]);
    assert_snapshot!(workspaces(&layout), @"output1: *[]");
}

#[test]
fn new_window_hides_without_moving_view() {
    // Four columns with the view at the right end, as in the first-show test.
    let setup = [
        Op::AddOutput(1),
        window(1, 500),
        window(2, 500),
        window(3, 500),
        window(4, 500),
        Op::FocusColumnFirst,
        Op::FocusColumnLast,
        Op::CompleteAnimations,
    ];
    let mut with_hidden = setup.to_vec();
    with_hidden.push(add_hidden(5, None));

    assert_snapshot!(off_on(&setup), @"off: 784, on: 784");
    assert_eq!(off_on(&with_hidden), off_on(&setup));
}

#[test]
fn new_window_in_scratchpad_does_not_activate_output() {
    let setup = || {
        check_ops([
            Op::AddOutput(1),
            Op::AddOutput(2),
            Op::FocusOutput(1),
            window(1, 100),
        ])
    };
    let add = |layout: &mut Layout<TestWindow>, activate| {
        let output = layout.monitors().nth(1).unwrap().output().clone();
        layout.add_window(
            TestWindow::new(TestWindowParams::new(2)),
            AddWindowTarget::Output(&output),
            None,
            None,
            false,
            false,
            activate,
        );
        layout.move_new_window_to_scratchpad(&2, None);
        layout.verify_invariants();
    };
    let active_output = |layout: &Layout<TestWindow>| layout.active_output().unwrap().name();

    let mut layout = setup();
    add(&mut layout, ActivateWindow::No);
    assert_eq!(active_output(&layout), "output1");
    assert_snapshot!(summary(&layout), @"hidden: [2], shown: [], focus: Some(1)");

    // Contrast: the map handler passes No, because activating moves the active output to where the
    // window was added, and hiding the window doesn't bring it back.
    let mut layout = setup();
    add(&mut layout, ActivateWindow::Yes);
    assert_eq!(active_output(&layout), "output2");
}

fn cycle_options() -> Options {
    let mut options = Options::default();
    options.scratchpad.cycle_windows = true;
    options
}

/// Three hidden windows, queued in order 1, 2, 3, and a tiled window 4.
fn three_hidden() -> Vec<Op> {
    vec![
        Op::AddOutput(1),
        window(1, 100),
        window(2, 100),
        window(3, 100),
        window(4, 100),
        hide(1),
        hide(2),
        hide(3),
    ]
}

#[test]
fn cycle_through_windows() {
    let mut layout = check_ops_with_options(cycle_options(), three_hidden());

    let mut steps = Vec::new();
    for _ in 0..5 {
        let res = layout.toggle_scratchpad(|_| true);
        layout.verify_invariants();
        steps.push(format!("{res:?} -> {}", summary(&layout)));
    }
    assert_snapshot!(steps.join("\n"), @"
    Shown(1) -> hidden: [2, 3], shown: [1], focus: Some(1)
    Cycled { hidden: 1, shown: 2 } -> hidden: [3, 1], shown: [2], focus: Some(2)
    Cycled { hidden: 2, shown: 3 } -> hidden: [1, 2], shown: [3], focus: Some(3)
    Hidden(3) -> hidden: [1, 2, 3], shown: [], focus: Some(4)
    Shown(1) -> hidden: [2, 3], shown: [1], focus: Some(1)
    ");
}

#[test]
fn cycle_without_option_toggles() {
    let mut layout = check_ops(three_hidden());

    let mut steps = Vec::new();
    for _ in 0..3 {
        let res = layout.toggle_scratchpad(|_| true);
        layout.verify_invariants();
        steps.push(format!("{res:?} -> {}", summary(&layout)));
    }
    assert_snapshot!(steps.join("\n"), @"
    Shown(1) -> hidden: [2, 3], shown: [1], focus: Some(1)
    Hidden(1) -> hidden: [2, 3, 1], shown: [], focus: Some(4)
    Shown(2) -> hidden: [3, 1], shown: [2], focus: Some(2)
    ");
}

#[test]
fn cycle_single_window_toggles() {
    let ops = [Op::AddOutput(1), window(1, 100), hide(1), toggle()];
    let mut layout = check_ops_with_options(cycle_options(), ops);
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [1], focus: Some(1)");

    assert_eq!(
        layout.toggle_scratchpad(|_| true),
        ScratchpadToggle::Hidden(1)
    );
    layout.verify_invariants();
    assert_snapshot!(summary(&layout), @"hidden: [1], shown: [], focus: None");
}

#[test]
fn cycle_with_filter() {
    // Only the matching window takes part, so this toggles it.
    let mut ops = three_hidden();
    ops.push(Op::ToggleScratchpad { id: Some(2) });
    let mut layout = check_ops_with_options(cycle_options(), ops);
    assert_snapshot!(summary(&layout), @"hidden: [1, 3], shown: [2], focus: Some(2)");

    check_ops_on_layout(&mut layout, [Op::ToggleScratchpad { id: Some(2) }]);
    assert_snapshot!(summary(&layout), @"hidden: [1, 3, 2], shown: [], focus: Some(4)");
}

#[test]
fn cycle_round_restarts_after_focusing_shown_window() {
    let mut ops = three_hidden();
    ops.extend([toggle(), toggle(), Op::FocusWindow(4)]);
    let mut layout = check_ops_with_options(cycle_options(), ops);
    assert_snapshot!(summary(&layout), @"hidden: [3, 1], shown: [2], focus: Some(4)");

    // Focusing the shown window starts a new round from it.
    let mut steps = Vec::new();
    for _ in 0..4 {
        let res = layout.toggle_scratchpad(|_| true);
        layout.verify_invariants();
        steps.push(format!("{res:?} -> {}", summary(&layout)));
    }
    assert_snapshot!(steps.join("\n"), @"
    Focused(2) -> hidden: [3, 1], shown: [2], focus: Some(2)
    Cycled { hidden: 2, shown: 3 } -> hidden: [1, 2], shown: [3], focus: Some(3)
    Cycled { hidden: 3, shown: 1 } -> hidden: [2, 3], shown: [1], focus: Some(1)
    Hidden(1) -> hidden: [2, 3, 1], shown: [], focus: Some(4)
    ");
}

#[test]
fn cycle_forgets_closed_windows() {
    let mut ops = three_hidden();
    // Show 1, then 2, then close 1, which is hidden.
    ops.extend([toggle(), toggle(), Op::CloseWindow(1)]);
    let mut layout = check_ops_with_options(cycle_options(), ops);
    assert_snapshot!(summary(&layout), @"hidden: [3], shown: [2], focus: Some(2)");

    let mut steps = Vec::new();
    for _ in 0..2 {
        let res = layout.toggle_scratchpad(|_| true);
        layout.verify_invariants();
        steps.push(format!("{res:?} -> {}", summary(&layout)));
    }
    assert_snapshot!(steps.join("\n"), @"
    Cycled { hidden: 2, shown: 3 } -> hidden: [2], shown: [3], focus: Some(3)
    Hidden(3) -> hidden: [2, 3], shown: [], focus: Some(4)
    ");
}

#[test]
fn cycle_does_nothing_when_hiding_is_refused() {
    let mut ops = three_hidden();
    ops.extend([
        toggle(),
        // A dialog of the shown window keeps it from hiding.
        window(5, 100),
        Op::SetParent {
            id: 5,
            new_parent_id: Some(1),
        },
        Op::FocusWindow(1),
    ]);
    let mut layout = check_ops_with_options(cycle_options(), ops);
    assert_snapshot!(summary(&layout), @"hidden: [2, 3], shown: [1], focus: Some(1)");

    assert_eq!(
        layout.toggle_scratchpad(|_| true),
        ScratchpadToggle::Nothing
    );
    layout.verify_invariants();
    assert_snapshot!(summary(&layout), @"hidden: [2, 3], shown: [1], focus: Some(1)");
}
