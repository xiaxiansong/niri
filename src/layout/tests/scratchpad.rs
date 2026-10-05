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
    assert_eq!(layout.popup_target_rect(&1), Rectangle::default());
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
    // isn't activated there.
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
    assert_snapshot!(workspaces(&layout), @"output1: *[1] []; output2: *[3, 2] [4] []");
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [2], focus: Some(1)");

    // The workspace with the shown window is moved to another output without activating it.
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
    assert_snapshot!(workspaces(&layout), @"output1: *[]; output2: [1] *[2] []");
    assert_snapshot!(summary(&layout), @"hidden: [], shown: [2], focus: Some(2)");
}
